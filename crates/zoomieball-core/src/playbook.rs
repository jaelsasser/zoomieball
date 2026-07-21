//! Fixed-schema RON playbook compilation and oracle intent resolution.

use std::collections::BTreeSet;
use std::fmt;

use crate::fixed::{Fx, Vec3Fx};
use crate::world::{BodyId, Role, Team, World};

/// Version of the accepted playbook schema.
pub const PLAYBOOK_ABI_VERSION: u32 = 1;

/// Desired position and spin emitted by the naive play solver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OracleIntent {
    /// Desired world-space position.
    pub position: Vec3Fx,
    /// Desired world-space angular velocity.
    pub spin: Vec3Fx,
}

/// Reusable oracle-intent buffer in canonical body order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OracleIntentBatch {
    /// One intent per physical sphere.
    pub intents: Vec<OracleIntent>,
}

impl OracleIntentBatch {
    /// Allocate one initialized intent per body.
    #[must_use]
    pub fn with_len(body_count: usize) -> Self {
        Self {
            intents: vec![OracleIntent::default(); body_count],
        }
    }
}

/// Role-wide formation template in one play node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleIntent {
    /// Center position before deterministic per-local-ID spreading.
    pub position: Vec3Fx,
    /// Desired angular velocity.
    pub spin: Vec3Fx,
}

/// Validated play graph node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayNode {
    name: String,
    edges: Vec<usize>,
    squad_cycle: Vec<u8>,
    goalie: RoleIntent,
    fielder: RoleIntent,
}

impl PlayNode {
    /// Stable node name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ordered outgoing node indices.
    #[must_use]
    pub fn edges(&self) -> &[usize] {
        &self.edges
    }

    /// Mailbox assigned to one local ID.
    #[must_use]
    pub fn squad_for(&self, local: u8) -> u8 {
        self.squad_cycle[usize::from(local) % self.squad_cycle.len()]
    }
}

/// Validated cyclic play graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playbook {
    nodes: Vec<PlayNode>,
}

impl Playbook {
    /// Compile the fixed-schema RON subset without floating-point conversion.
    pub fn compile_ron(source: &str) -> Result<Self, PlaybookError> {
        let tokens = lex(source)?;
        Parser::new(&tokens).parse_playbook()
    }

    /// Validated graph nodes.
    #[must_use]
    pub fn nodes(&self) -> &[PlayNode] {
        &self.nodes
    }

    /// Follow one ordered outgoing port from a node.
    #[must_use]
    pub fn traverse(&self, node: usize, port: usize) -> Option<usize> {
        self.nodes.get(node)?.edges.get(port).copied()
    }

    /// Resolve one node into caller-owned intent and squad buffers.
    pub fn resolve(&self, node: usize, world: &mut World, output: &mut OracleIntentBatch) {
        let node = &self.nodes[node];
        output
            .intents
            .resize(world.view().len(), OracleIntent::default());
        for index in 0..world.ids.len() {
            let BodyId::Player { team, local } = world.ids[index] else {
                output.intents[index] = OracleIntent {
                    position: world.positions[index],
                    spin: Vec3Fx::ZERO,
                };
                continue;
            };
            let template = match world.roles[index] {
                Role::Goalie => node.goalie,
                Role::Fielder => node.fielder,
                Role::Objective => unreachable!("a player identity cannot have objective role"),
            };
            let direction = match team {
                Team::Zero => Fx::ONE,
                Team::One => -Fx::ONE,
            };
            let ordinal = i32::from(local.get());
            let column = ordinal % 11 - 5;
            let row = ordinal / 11;
            output.intents[index] = OracleIntent {
                position: Vec3Fx::new(
                    template.position.x * direction
                        + Fx::from_raw(row * Fx::ONE_RAW / 4) * direction,
                    template.position.y + Fx::from_raw(column * Fx::ONE_RAW / 2),
                    template.position.z,
                ),
                spin: Vec3Fx::new(
                    template.spin.x * direction,
                    template.spin.y,
                    template.spin.z,
                ),
            };
            world.squads[index] = node.squad_for(local.get());
        }
    }
}

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
    /// Edge points outside the node table.
    DanglingEdge {
        /// Source node name.
        node: String,
        /// Missing target index.
        target: usize,
    },
    /// Squad cycle is empty or contains a value above seven.
    InvalidSquad(String),
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
            Self::DanglingEdge { node, target } => {
                write!(
                    formatter,
                    "play node {node} points to missing node {target}"
                )
            }
            Self::InvalidSquad(node) => write!(formatter, "play node {node} has invalid squads"),
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

struct Parser<'a> {
    tokens: &'a [Token],
    cursor: usize,
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

    fn node(&mut self) -> Result<PlayNode, PlaybookError> {
        self.punctuation(&Token::LeftParen, "`(`")?;
        self.ident("name")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let name = self.string()?;
        self.comma()?;
        self.ident("edges")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let edges = self
            .unsigned_array()?
            .into_iter()
            .map(|edge| usize::try_from(edge).expect("u32 fits usize on supported targets"))
            .collect();
        self.comma()?;
        self.ident("squad_cycle")?;
        self.punctuation(&Token::Colon, "`:`")?;
        let squad_cycle = self
            .unsigned_array()?
            .into_iter()
            .map(|squad| u8::try_from(squad).unwrap_or(u8::MAX))
            .collect();
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
        Ok(PlayNode {
            name,
            edges,
            squad_cycle,
            goalie,
            fielder,
        })
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

    fn string(&mut self) -> Result<String, PlaybookError> {
        let Some(Token::String(value)) = self.tokens.get(self.cursor) else {
            return self.syntax("string");
        };
        self.cursor += 1;
        Ok(value.clone())
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

fn validate_nodes(nodes: Vec<PlayNode>) -> Result<Playbook, PlaybookError> {
    if nodes.is_empty() {
        return Err(PlaybookError::Empty);
    }
    let mut names = BTreeSet::new();
    for node in &nodes {
        if !names.insert(node.name.clone()) {
            return Err(PlaybookError::DuplicateName(node.name.clone()));
        }
        if !(1..=8).contains(&node.edges.len()) {
            return Err(PlaybookError::EdgeCount {
                node: node.name.clone(),
                count: node.edges.len(),
            });
        }
        if node.squad_cycle.is_empty() || node.squad_cycle.iter().any(|&squad| squad > 7) {
            return Err(PlaybookError::InvalidSquad(node.name.clone()));
        }
        if let Some(&target) = node.edges.iter().find(|&&target| target >= nodes.len()) {
            return Err(PlaybookError::DanglingEdge {
                node: node.name.clone(),
                target,
            });
        }
    }
    Ok(Playbook { nodes })
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
    use super::*;

    const CYCLIC: &str = r#"(
        version: 1,
        nodes: [
          (name: "a", edges: [1], squad_cycle: [0, 1],
           goalie: (position: [-14.0, 0, 1], spin: [0, 0, 0]),
           fielder: (position: [1.25, 0, 1], spin: [0, 0, 0])),
          (name: "b", edges: [0], squad_cycle: [7],
           goalie: (position: [-14, 0, 1], spin: [0, 0, 0]),
           fielder: (position: [-1.25, 0, 1], spin: [0, 0, 0])),
        ],
    )"#;

    #[test]
    fn cyclic_ron_compiles_and_traverses_without_float_parsing() {
        let playbook = Playbook::compile_ron(CYCLIC).unwrap();
        assert_eq!(playbook.traverse(0, 0), Some(1));
        assert_eq!(playbook.traverse(1, 0), Some(0));
        assert_eq!(playbook.nodes()[0].fielder.position.x.raw(), 81_920);
    }

    #[test]
    fn dangling_edges_fail_at_the_compile_boundary() {
        let source = CYCLIC.replacen("edges: [1]", "edges: [9]", 1);
        assert!(matches!(
            Playbook::compile_ron(&source),
            Err(PlaybookError::DanglingEdge { .. })
        ));
    }

    #[test]
    fn team_one_intents_mirror_team_zero_and_node_changes_reassign_squads() {
        let playbook = Playbook::compile_ron(CYCLIC).unwrap();
        let mut world = World::new(10);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve(0, &mut world, &mut intents);
        let zero = world
            .player_index(Team::Zero, crate::world::LocalId::new(1).unwrap())
            .unwrap();
        let one = world
            .player_index(Team::One, crate::world::LocalId::new(1).unwrap())
            .unwrap();
        assert_eq!(
            intents.intents[zero].position.x,
            -intents.intents[one].position.x
        );
        assert_eq!(world.view().squads[zero], 1);
        playbook.resolve(1, &mut world, &mut intents);
        assert_eq!(world.view().squads[zero], 7);
    }
}
