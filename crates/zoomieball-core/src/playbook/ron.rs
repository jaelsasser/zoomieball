//! The at-rest form: lexer, fixed-schema parser, and validation for graph-v0 RON sheets.
//!
//! Decimal literals parse straight to Q16.16 — no `f32`/`f64` ever exists here, per
//! determinism rule 1 — and every shape outside the closed vocabulary fails with its own
//! typed [`PlaybookError`] before a [`PlayNode`] is built.

use std::collections::BTreeSet;
use std::fmt;

use crate::fixed::{Fx, Vec3Fx};
use crate::perception::Relation;

use super::{
    Form, PLAYBOOK_ABI_VERSION, PORT_COUNT, PlayEdge, PlayNode, Playbook, RoleIntent, SQUAD_COUNT,
    Target, Trigger, Verb, VerbEntry,
};

/// Widest and deepest a pod may be authored. A pod larger than the physical roster places
/// nobody, and the bound is what keeps every `Fx::from_i32` in the layout arithmetic in domain.
const POD_EXTENT_MAX: u32 = 100;

/// Largest form gap magnitude a sheet may author.
///
/// The layout arithmetic multiplies a gap by ordinal-derived integers; with locals capped at 99
/// and pod extents at [`POD_EXTENT_MAX`], the worst multiplier is under 150, so gaps to 100 keep
/// every `qmul` inside the `i32` domain the defined-domain helpers demand. An unbounded gap would
/// panic the CPU tier and produce garbage on the GPU tier from authored data alone.
const FORM_GAP_MAX: Fx = Fx::from_i32(100);

/// Playbook syntax or validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybookError {
    /// Lexical failure at a source byte.
    Lex {
        /// Source byte offset.
        offset: usize,
        /// Stable failure description.
        message: &'static str,
    },
    /// Schema token mismatch.
    Syntax {
        /// Token ordinal.
        token: usize,
        /// Expected schema element.
        expected: &'static str,
    },
    /// Unsupported ABI version.
    Version(u32),
    /// Graph has no nodes.
    Empty,
    /// Node name is duplicated.
    DuplicateName(String),
    /// Edge count is outside `1..=8`.
    EdgeCount {
        /// Node name.
        node: String,
        /// Actual edge count.
        count: usize,
    },
    /// Last port carries a trigger other than `Always`, so the node cannot deliberately leave.
    MissingAlwaysPort(String),
    /// Verb table is not exactly eight entries.
    VerbCount {
        /// Node name.
        node: String,
        /// Actual entry count.
        count: usize,
    },
    /// Edge points outside the node table.
    DanglingEdge {
        /// Source node name.
        node: String,
        /// Missing target index.
        target: usize,
    },
    /// Squad cycle is empty, or a squad number exceeds seven.
    InvalidSquad(String),
    /// Pod rank or file is outside `1..=100`.
    PodExtent(String),
    /// Form gap magnitude exceeds 100, which could drive the layout arithmetic out of `i32`.
    FormGap(String),
    /// Trigger identifier is outside the closed vocabulary.
    UnknownTrigger(String),
    /// `Possession` operand is outside `Teammate`, `Opponent`, `Neutral`.
    UnknownRelation(String),
    /// Target identifier is outside the closed vocabulary.
    UnknownTarget(String),
    /// Verb identifier is outside the closed vocabulary.
    UnknownVerb(String),
    /// Form identifier is outside the closed vocabulary.
    UnknownForm(String),
    /// Decimal does not fit Q16.16.
    InvalidFixed(String),
}

impl fmt::Display for PlaybookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex { offset, message } => write!(formatter, "RON byte {offset}: {message}"),
            Self::Syntax { token, expected } => {
                write!(formatter, "RON token {token}: expected {expected}")
            }
            Self::Version(version) => write!(formatter, "unsupported playbook ABI {version}"),
            Self::Empty => write!(formatter, "playbook requires at least one node"),
            Self::DuplicateName(name) => write!(formatter, "duplicate play node {name}"),
            Self::EdgeCount { node, count } => {
                write!(
                    formatter,
                    "play node {node} has {count} edges; expected 1..=8"
                )
            }
            Self::MissingAlwaysPort(node) => {
                write!(formatter, "play node {node} last port is not Always")
            }
            Self::VerbCount { node, count } => {
                write!(
                    formatter,
                    "play node {node} has {count} verb entries; expected 8"
                )
            }
            Self::DanglingEdge { node, target } => {
                write!(
                    formatter,
                    "play node {node} points to missing node {target}"
                )
            }
            Self::InvalidSquad(node) => write!(formatter, "play node {node} has invalid squads"),
            Self::PodExtent(node) => {
                write!(formatter, "play node {node} has a pod outside 1..=100")
            }
            Self::FormGap(node) => {
                write!(
                    formatter,
                    "play node {node} has a form gap outside |gap| <= 100"
                )
            }
            Self::UnknownTrigger(name) => write!(formatter, "unknown trigger {name}"),
            Self::UnknownRelation(name) => write!(formatter, "unknown possession relation {name}"),
            Self::UnknownTarget(name) => write!(formatter, "unknown target {name}"),
            Self::UnknownVerb(name) => write!(formatter, "unknown verb {name}"),
            Self::UnknownForm(name) => write!(formatter, "unknown form {name}"),
            Self::InvalidFixed(number) => write!(formatter, "{number} does not fit Q16.16"),
        }
    }
}

impl std::error::Error for PlaybookError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Colon,
    Comma,
    Ident(String),
    String(String),
    Number(String),
}

fn lex(source: &str) -> Result<Vec<Token>, PlaybookError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b' ' | b'\t' | b'\r' | b'\n' => cursor += 1,
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                cursor += 2;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'(' => push_advance(&mut tokens, Token::LeftParen, &mut cursor),
            b')' => push_advance(&mut tokens, Token::RightParen, &mut cursor),
            b'[' => push_advance(&mut tokens, Token::LeftBracket, &mut cursor),
            b']' => push_advance(&mut tokens, Token::RightBracket, &mut cursor),
            b':' => push_advance(&mut tokens, Token::Colon, &mut cursor),
            b',' => push_advance(&mut tokens, Token::Comma, &mut cursor),
            b'"' => {
                let start = cursor + 1;
                cursor = start;
                while cursor < bytes.len() && bytes[cursor] != b'"' {
                    if bytes[cursor] == b'\\' {
                        return Err(PlaybookError::Lex {
                            offset: cursor,
                            message: "escaped strings are not part of the playbook subset",
                        });
                    }
                    cursor += 1;
                }
                if cursor == bytes.len() {
                    return Err(PlaybookError::Lex {
                        offset: start - 1,
                        message: "unterminated string",
                    });
                }
                let value = source
                    .get(start..cursor)
                    .ok_or(PlaybookError::Lex {
                        offset: start,
                        message: "string is not UTF-8 aligned",
                    })?
                    .to_owned();
                tokens.push(Token::String(value));
                cursor += 1;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = cursor;
                cursor += 1;
                while cursor < bytes.len()
                    && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
                {
                    cursor += 1;
                }
                tokens.push(Token::Ident(source[start..cursor].to_owned()));
            }
            byte if byte.is_ascii_digit() || byte == b'-' || byte == b'+' => {
                let start = cursor;
                cursor += 1;
                while cursor < bytes.len()
                    && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'.')
                {
                    cursor += 1;
                }
                tokens.push(Token::Number(source[start..cursor].to_owned()));
            }
            _ => {
                return Err(PlaybookError::Lex {
                    offset: cursor,
                    message: "unsupported character",
                });
            }
        }
    }
    Ok(tokens)
}

fn push_advance(tokens: &mut Vec<Token>, token: Token, cursor: &mut usize) {
    tokens.push(token);
    *cursor += 1;
}

/// One node as written, before validation compiles its ordinal tables.
struct ParsedNode {
    name: String,
    edges: Vec<PlayEdge>,
    squad_cycle: Vec<u8>,
    coach_gate: Fx,
    goalie_verb: VerbEntry,
    verbs: Vec<VerbEntry>,
    goalie: RoleIntent,
    fielder: RoleIntent,
}

struct Parser<'a> {
    tokens: &'a [Token],
    cursor: usize,
}

/// Compile one RON source to a validated graph: [`Playbook::compile_ron`]'s implementation.
pub(super) fn compile(source: &str) -> Result<Playbook, PlaybookError> {
    let tokens = lex(source)?;
    Parser::new(&tokens).parse_playbook()
}

impl<'a> Parser<'a> {
    const fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn parse_playbook(mut self) -> Result<Playbook, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("version")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let version = self.unsigned()?;
        if version != PLAYBOOK_ABI_VERSION {
            return Err(PlaybookError::Version(version));
        }
        self.comma()?;
        self.ident("nodes")?;
        self.punctuation(&Token::Colon, "`:`")?;
        self.punctuation(&Token::LeftBracket, "`[`")?;
        let mut nodes = Vec::new();
        while self.peek() != Some(&Token::RightBracket) {
            nodes.push(self.node()?);
            self.optional_comma();
        }
        self.punctuation(&Token::RightBracket, "`]`")?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        if self.cursor != self.tokens.len() {
            return self.syntax("end of input");
        }
        validate_nodes(nodes)
    }

    fn node(&mut self) -> Result<ParsedNode, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("name")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let name = self.string()?;
        self.comma()?;
        self.ident("edges")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let edges = self.edges()?;
        self.comma()?;
        self.ident("squad_cycle")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let squad_cycle = self
            .unsigned_array()?
            .into_iter()
            .map(|squad| u8::try_from(squad).unwrap_or(u8::MAX))
            .collect();
        self.comma()?;
        self.ident("coach_gate")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let coach_gate = self.fixed()?;
        self.comma()?;
        self.ident("goalie_verb")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let goalie_verb = self.verb_entry()?;
        self.comma()?;
        self.ident("verbs")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let verbs = self.verb_entries()?;
        self.comma()?;
        self.ident("goalie")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let goalie = self.role_intent()?;
        self.comma()?;
        self.ident("fielder")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let fielder = self.role_intent()?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(ParsedNode {
            name,
            edges,
            squad_cycle,
            coach_gate,
            goalie_verb,
            verbs,
            goalie,
            fielder,
        })
    }

    fn edges(&mut self) -> Result<Vec<PlayEdge>, PlaybookError> {
        self.punctuation(&Token::LeftBracket, "`[`")?;
        let mut edges = Vec::new();
        while self.peek() != Some(&Token::RightBracket) {
            edges.push(self.edge()?);
            self.optional_comma();
        }
        self.punctuation(&Token::RightBracket, "`]`")?;
        Ok(edges)
    }

    fn edge(&mut self) -> Result<PlayEdge, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("to")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let to = usize::try_from(self.unsigned()?).expect("u32 fits usize on supported targets");
        self.comma()?;
        self.ident("trigger")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let trigger = self.trigger()?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(PlayEdge { to, trigger })
    }

    fn verb_entries(&mut self) -> Result<Vec<VerbEntry>, PlaybookError> {
        self.punctuation(&Token::LeftBracket, "`[`")?;
        let mut entries = Vec::new();
        while self.peek() != Some(&Token::RightBracket) {
            entries.push(self.verb_entry()?);
            self.optional_comma();
        }
        self.punctuation(&Token::RightBracket, "`]`")?;
        Ok(entries)
    }

    fn verb_entry(&mut self) -> Result<VerbEntry, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("verb")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let verb = self.verb()?;
        self.comma()?;
        self.ident("target")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let target = self.target()?;
        self.comma()?;
        self.ident("form")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let form = self.form()?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(VerbEntry { verb, target, form })
    }

    fn trigger(&mut self) -> Result<Trigger, PlaybookError> {
        let name = self.variant()?;
        match name {
            "Always" => Ok(Trigger::Always),
            "Elapsed" => self.payload(Self::unsigned).map(Trigger::Elapsed),
            "BallPast" => self.payload(Self::fixed).map(Trigger::BallPast),
            "BallBehind" => self.payload(Self::fixed).map(Trigger::BallBehind),
            "BallAloft" => self.payload(Self::fixed).map(Trigger::BallAloft),
            "Possession" => self.payload(Self::relation).map(Trigger::Possession),
            "Lead" => self.payload(Self::signed).map(Trigger::Lead),
            "CoachEdge" => Ok(Trigger::CoachEdge),
            _ => Err(PlaybookError::UnknownTrigger(name.to_owned())),
        }
    }

    fn relation(&mut self) -> Result<Relation, PlaybookError> {
        let name = self.variant()?;
        match name {
            "Teammate" => Ok(Relation::Teammate),
            "Opponent" => Ok(Relation::Opponent),
            "Neutral" => Ok(Relation::Neutral),
            _ => Err(PlaybookError::UnknownRelation(name.to_owned())),
        }
    }

    fn target(&mut self) -> Result<Target, PlaybookError> {
        let name = self.variant()?;
        match name {
            "GameBall" => Ok(Target::GameBall),
            "OwnGoal" => Ok(Target::OwnGoal),
            "OpponentGoal" => Ok(Target::OpponentGoal),
            "Squad" => self.payload(Self::squad).map(Target::Squad),
            "NearestOpponent" => Ok(Target::NearestOpponent),
            "NearestToMe" => Ok(Target::NearestToMe),
            "Slot" => Ok(Target::Slot),
            _ => Err(PlaybookError::UnknownTarget(name.to_owned())),
        }
    }

    fn verb(&mut self) -> Result<Verb, PlaybookError> {
        let name = self.variant()?;
        match name {
            "Align" => Ok(Verb::Align),
            "Pursue" => Ok(Verb::Pursue),
            "Drive" => Ok(Verb::Drive),
            "Clear" => Ok(Verb::Clear),
            "Cover" => Ok(Verb::Cover),
            "Zone" => Ok(Verb::Zone),
            "Sweep" => Ok(Verb::Sweep),
            "Block" => Ok(Verb::Block),
            "Lead" => Ok(Verb::Lead),
            "Jam" => Ok(Verb::Jam),
            "Guard" => Ok(Verb::Guard),
            _ => Err(PlaybookError::UnknownVerb(name.to_owned())),
        }
    }

    fn form(&mut self) -> Result<Form, PlaybookError> {
        let name = self.variant()?;
        match name {
            "Point" => Ok(Form::Point),
            "Pod" => self.pod(),
            "Wedge" => self.payload(Self::fixed).map(Form::Wedge),
            "Arc" => self.payload(Self::fixed).map(Form::Arc),
            _ => Err(PlaybookError::UnknownForm(name.to_owned())),
        }
    }

    fn pod(&mut self) -> Result<Form, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        let rank = self.unsigned()?;
        self.comma()?;
        let file = self.unsigned()?;
        self.comma()?;
        let gap = self.fixed()?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(Form::Pod { rank, file, gap })
    }

    /// Consume one parenthesized enum payload.
    fn payload<T>(
        &mut self,
        value: impl FnOnce(&mut Self) -> Result<T, PlaybookError>,
    ) -> Result<T, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        let parsed = value(self)?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(parsed)
    }

    fn role_intent(&mut self) -> Result<RoleIntent, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("position")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let position = self.vector()?;
        self.comma()?;
        self.ident("spin")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let spin = self.vector()?;
        self.optional_comma();
        self.punctuation(&Token::RightParen, "`)`")?;
        Ok(RoleIntent { position, spin })
    }

    fn vector(&mut self) -> Result<Vec3Fx, PlaybookError> {
        self.punctuation(&Token::LeftBracket, "`[`")?;
        let x = self.fixed()?;
        self.comma()?;
        let y = self.fixed()?;
        self.comma()?;
        let z = self.fixed()?;
        self.optional_comma();
        self.punctuation(&Token::RightBracket, "`]`")?;
        Ok(Vec3Fx::new(x, y, z))
    }

    fn unsigned_array(&mut self) -> Result<Vec<u32>, PlaybookError> {
        self.punctuation(&Token::LeftBracket, "`[`")?;
        let mut values = Vec::new();
        while self.peek() != Some(&Token::RightBracket) {
            values.push(self.unsigned()?);
            if self.peek() != Some(&Token::RightBracket) {
                self.comma()?;
            }
        }
        self.punctuation(&Token::RightBracket, "`]`")?;
        Ok(values)
    }

    fn fixed(&mut self) -> Result<Fx, PlaybookError> {
        let Some(Token::Number(number)) = self.tokens.get(self.cursor) else {
            return self.syntax("fixed decimal");
        };
        self.cursor += 1;
        parse_fixed(number)
    }

    fn unsigned(&mut self) -> Result<u32, PlaybookError> {
        let Some(Token::Number(number)) = self.tokens.get(self.cursor) else {
            return self.syntax("unsigned integer");
        };
        let value = number
            .parse()
            .map_err(|_| PlaybookError::InvalidFixed(number.clone()))?;
        self.cursor += 1;
        Ok(value)
    }

    fn signed(&mut self) -> Result<i32, PlaybookError> {
        let Some(Token::Number(number)) = self.tokens.get(self.cursor) else {
            return self.syntax("signed integer");
        };
        let value = number
            .parse()
            .map_err(|_| PlaybookError::InvalidFixed(number.clone()))?;
        self.cursor += 1;
        Ok(value)
    }

    /// Out-of-range squads survive parsing as `u8::MAX` and fail validation with their node name.
    fn squad(&mut self) -> Result<u8, PlaybookError> {
        Ok(u8::try_from(self.unsigned()?).unwrap_or(u8::MAX))
    }

    fn string(&mut self) -> Result<String, PlaybookError> {
        let Some(Token::String(value)) = self.tokens.get(self.cursor) else {
            return self.syntax("string");
        };
        self.cursor += 1;
        Ok(value.clone())
    }

    fn variant(&mut self) -> Result<&'a str, PlaybookError> {
        let tokens = self.tokens;
        let Some(Token::Ident(name)) = tokens.get(self.cursor) else {
            return self.syntax("enum variant");
        };
        self.cursor += 1;
        Ok(name.as_str())
    }

    fn ident(&mut self, expected: &'static str) -> Result<(), PlaybookError> {
        if self.tokens.get(self.cursor) == Some(&Token::Ident(expected.to_owned())) {
            self.cursor += 1;
            Ok(())
        } else {
            self.syntax(expected)
        }
    }

    fn comma(&mut self) -> Result<(), PlaybookError> {
        self.punctuation(&Token::Comma, "`,`")
    }

    fn optional_comma(&mut self) {
        if self.peek() == Some(&Token::Comma) {
            self.cursor += 1;
        }
    }

    fn punctuation(&mut self, expected: &Token, label: &'static str) -> Result<(), PlaybookError> {
        if self.tokens.get(self.cursor) == Some(expected) {
            self.cursor += 1;
            Ok(())
        } else {
            self.syntax(label)
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn syntax<T>(&self, expected: &'static str) -> Result<T, PlaybookError> {
        Err(PlaybookError::Syntax {
            token: self.cursor,
            expected,
        })
    }
}

/// Every verb entry a node carries, goalie first.
fn entries(node: &ParsedNode) -> impl Iterator<Item = VerbEntry> + '_ {
    std::iter::once(node.goalie_verb).chain(node.verbs.iter().copied())
}

fn validate_nodes(nodes: Vec<ParsedNode>) -> Result<Playbook, PlaybookError> {
    if nodes.is_empty() {
        return Err(PlaybookError::Empty);
    }
    let mut names = BTreeSet::new();
    for node in &nodes {
        if !names.insert(node.name.clone()) {
            return Err(PlaybookError::DuplicateName(node.name.clone()));
        }
        if !(1..=PORT_COUNT).contains(&node.edges.len()) {
            return Err(PlaybookError::EdgeCount {
                node: node.name.clone(),
                count: node.edges.len(),
            });
        }
        if node.edges.last().map(|edge| edge.trigger) != Some(Trigger::Always) {
            return Err(PlaybookError::MissingAlwaysPort(node.name.clone()));
        }
        if node.verbs.len() != SQUAD_COUNT {
            return Err(PlaybookError::VerbCount {
                node: node.name.clone(),
                count: node.verbs.len(),
            });
        }
        if node.squad_cycle.is_empty()
            || node.squad_cycle.iter().any(|&squad| squad > 7)
            || entries(node).any(|entry| matches!(entry.target, Target::Squad(squad) if squad > 7))
        {
            return Err(PlaybookError::InvalidSquad(node.name.clone()));
        }
        if entries(node).any(|entry| {
            matches!(entry.form, Form::Pod { rank, file, .. }
                if !(1..=POD_EXTENT_MAX).contains(&rank) || !(1..=POD_EXTENT_MAX).contains(&file))
        }) {
            return Err(PlaybookError::PodExtent(node.name.clone()));
        }
        // `unsigned_abs` rather than `Fx::abs`: the wrapping absolute value of the negative
        // endpoint is itself, which would slip the widest representable gap through the bound.
        if entries(node).any(|entry| {
            let gap = match entry.form {
                Form::Point => Fx::ZERO,
                Form::Pod { gap, .. } | Form::Wedge(gap) | Form::Arc(gap) => gap,
            };
            gap.raw().unsigned_abs() > FORM_GAP_MAX.raw().unsigned_abs()
        }) {
            return Err(PlaybookError::FormGap(node.name.clone()));
        }
        if let Some(edge) = node.edges.iter().find(|edge| edge.to >= nodes.len()) {
            return Err(PlaybookError::DanglingEdge {
                node: node.name.clone(),
                target: edge.to,
            });
        }
    }
    Ok(Playbook {
        nodes: nodes.into_iter().map(compile_node).collect(),
    })
}

/// Flatten one validated node, compiling the three tables the closed-form squad ordinal reads.
fn compile_node(node: ParsedNode) -> PlayNode {
    let squad_count = node
        .squad_cycle
        .iter()
        .fold([0u32; SQUAD_COUNT], |mut counts, &squad| {
            counts[usize::from(squad)] += 1;
            counts
        });
    let cycle_prefix = node
        .squad_cycle
        .iter()
        .scan([0u32; SQUAD_COUNT], |seen, &squad| {
            let before = seen[usize::from(squad)];
            seen[usize::from(squad)] += 1;
            Some(before)
        })
        .collect();
    let goalie_correction =
        std::array::from_fn(|squad| u32::from(usize::from(node.squad_cycle[0]) == squad));
    PlayNode {
        name: node.name,
        edges: node.edges,
        squad_cycle: node.squad_cycle,
        coach_gate: node.coach_gate,
        goalie_verb: node.goalie_verb,
        verbs: node
            .verbs
            .try_into()
            .expect("validation accepted exactly eight verb entries"),
        goalie: node.goalie,
        fielder: node.fielder,
        squad_count,
        cycle_prefix,
        goalie_correction,
    }
}

fn parse_fixed(source: &str) -> Result<Fx, PlaybookError> {
    let (negative, unsigned) = match source.as_bytes().first() {
        Some(b'-') => (true, &source[1..]),
        Some(b'+') => (false, &source[1..]),
        _ => (false, source),
    };
    let mut parts = unsigned.split('.');
    let integral = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if integral.is_empty() || parts.next().is_some() {
        return Err(PlaybookError::InvalidFixed(source.to_owned()));
    }
    let integral: i128 = integral
        .parse()
        .map_err(|_| PlaybookError::InvalidFixed(source.to_owned()))?;
    let fractional = if let Some(fraction) = fraction {
        if fraction.is_empty() || fraction.len() > 9 {
            return Err(PlaybookError::InvalidFixed(source.to_owned()));
        }
        let numerator: i128 = fraction
            .parse()
            .map_err(|_| PlaybookError::InvalidFixed(source.to_owned()))?;
        let denominator = 10i128
            .pow(u32::try_from(fraction.len()).expect("fraction length is limited below u32::MAX"));
        (numerator * i128::from(Fx::ONE_RAW) + denominator / 2) / denominator
    } else {
        0
    };
    let magnitude = integral
        .checked_mul(i128::from(Fx::ONE_RAW))
        .and_then(|whole| whole.checked_add(fractional))
        .ok_or_else(|| PlaybookError::InvalidFixed(source.to_owned()))?;
    let signed = if negative { -magnitude } else { magnitude };
    i32::try_from(signed)
        .map(Fx::from_raw)
        .map_err(|_| PlaybookError::InvalidFixed(source.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::cyclic;
    use super::*;

    #[test]
    fn cyclic_ron_compiles_and_traverses_without_float_parsing() {
        let playbook = Playbook::compile_ron(&cyclic()).unwrap();
        assert_eq!(playbook.traverse(0, 0), Some(1));
        assert_eq!(playbook.traverse(1, 0), Some(0));
        assert_eq!(playbook.nodes()[0].fielder.position.x.raw(), 81_920);
    }

    #[test]
    fn dangling_edges_fail_at_the_compile_boundary() {
        let source = cyclic().replacen("(to: 1,", "(to: 9,", 1);
        assert!(matches!(
            Playbook::compile_ron(&source),
            Err(PlaybookError::DanglingEdge { .. })
        ));
    }

    #[test]
    fn a_node_must_leave_deliberately_and_assign_every_squad() {
        let stranded =
            cyclic().replacen("(to: 1, trigger: Always)", "(to: 1, trigger: Lead(1))", 1);
        assert!(matches!(
            Playbook::compile_ron(&stranded),
            Err(PlaybookError::MissingAlwaysPort(_))
        ));
        let short = cyclic().replacen("(verb: Align, target: Slot, form: Point),\n", "", 1);
        assert!(matches!(
            Playbook::compile_ron(&short),
            Err(PlaybookError::VerbCount { count: 7, .. })
        ));
    }
}
