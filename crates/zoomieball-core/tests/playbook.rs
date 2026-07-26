//! Contract fixtures for the acknowledged graph-v0 vocabulary.
//!
//! The unit tests inside `src/playbook/` cover the lexer, the parser, and the layout internals.
//! This suite covers the closed vocabulary itself as a contract: every trigger, verb, target, and form
//! the acknowledged proposal names round-trips through the RON fixture, every shape outside it is
//! rejected with its own typed error, each verb builds the aim point the proposal tabulates, and
//! the two teams' cursors are independent.
//!
//! Every expectation is an exact Q16.16 word. A tolerance here would be an arithmetic mistake
//! rather than a rounding allowance, so none appears.

/// The whole suite is one module so that `cargo test -p zoomieball-core playbook` — this
/// bite's completion command — selects it alongside the unit tests inside `playbook.rs`.
mod playbook {
    use zoomieball_core::controller::{
        ActRequest, CheckpointError, ControllerBackend, MotorCommandBatch, RewardBatch,
    };
    use zoomieball_core::fixed::{Fx, Vec3Fx};
    use zoomieball_core::perception::Relation;
    use zoomieball_core::physics::Arena;
    use zoomieball_core::pipeline::{Match, MatchConfig};
    use zoomieball_core::playbook::{
        COVER_GAP, Form, GraphState, OracleIntent, OracleIntentBatch, PLAYBOOK_ABI_VERSION,
        PORT_COUNT, POSSESSION_TICKS, PlayEdge, PlayNode, Playbook, PlaybookError, SQUAD_COUNT,
        Target, Trigger, Verb, VerbEntry, next_cursor,
    };
    use zoomieball_core::world::{LocalId, Team, World};

    const SHIPPED: &str = include_str!("../../../assets/default-playbook.ron");

    /// A verb entry that holds its template slot, which is what every squad a fixture does not name
    /// takes.
    const HOLD: &str = "(verb: Align, target: Slot, form: Point)";

    /// The fielder template every resolve fixture reads back. Its `x` and `y` are both what the
    /// frame convention turns, its `y` sits past the goal mouth's half-width so `Guard` has
    /// something to clamp, and its spin is the one `Align` alone emits.
    const FIELDER: &str = "(position: [2.0, 4.0, 1.0], spin: [1.0, 2.0, 3.0])";

    /// The goalie template, distinct from the fielder's in every component so a fixture cannot read
    /// one for the other.
    const GOALIE: &str = "(position: [-14.0, 3.0, 1.0], spin: [-1.0, 0.5, 0.25])";

    /// One play node as RON text: a fixture names the fields it varies and inherits inert defaults
    /// for the rest — one self-looping `Always` port, one squad holding the whole roster, and eight
    /// fielder entries on their template slot.
    #[derive(Debug, Clone, Copy)]
    struct Node<'a> {
        name: &'a str,
        edges: &'a str,
        squad_cycle: &'a str,
        coach_gate: &'a str,
        goalie_verb: &'a str,
        /// Leading verb-table entries; the table is padded to eight with `HOLD`, and a longer slice
        /// stays long so the entry-count fixture can overshoot.
        verbs: &'a [&'a str],
        fielder: &'a str,
    }

    impl Default for Node<'_> {
        fn default() -> Self {
            Self {
                name: "a",
                edges: "[(to: 0, trigger: Always)]",
                squad_cycle: "[0]",
                coach_gate: "0.0",
                goalie_verb: "(verb: Guard, target: GameBall, form: Point)",
                verbs: &[],
                fielder: FIELDER,
            }
        }
    }

    impl Node<'_> {
        fn ron(&self) -> String {
            let Self {
                name,
                edges,
                squad_cycle,
                coach_gate,
                goalie_verb,
                verbs,
                fielder,
            } = *self;
            let verbs = (0..verbs.len().max(SQUAD_COUNT))
                .map(|slot| verbs.get(slot).copied().unwrap_or(HOLD))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "(name: \"{name}\", edges: {edges}, squad_cycle: {squad_cycle}, \
                 coach_gate: {coach_gate}, goalie_verb: {goalie_verb}, verbs: [{verbs}], \
                 goalie: {GOALIE}, fielder: {fielder})"
            )
        }
    }

    /// One `(verb, target, form)` entry as RON text.
    fn entry(verb: &str, target: &str, form: &str) -> String {
        format!("(verb: {verb}, target: {target}, form: {form})")
    }

    /// Borrow a built table of entries as the slice `Node` takes.
    fn borrowed(entries: &[String]) -> Vec<&str> {
        entries.iter().map(String::as_str).collect()
    }

    /// A whole playbook at an arbitrary declared version, which only the version fixture needs.
    fn at_version(version: u32, nodes: &[Node<'_>]) -> String {
        let nodes = nodes.iter().map(Node::ron).collect::<Vec<_>>().join(",");
        format!("(version: {version}, nodes: [{nodes}])")
    }

    fn book(nodes: &[Node<'_>]) -> String {
        at_version(PLAYBOOK_ABI_VERSION, nodes)
    }

    fn compiled(nodes: &[Node<'_>]) -> Playbook {
        Playbook::compile_ron(&book(nodes)).expect("the fixture sheet is inside the vocabulary")
    }

    /// Compile one fixture book and hand back the failure its shape must produce.
    fn rejection(nodes: &[Node<'_>]) -> PlaybookError {
        Playbook::compile_ron(&book(nodes))
            .expect_err("the fixture sheet is outside the vocabulary")
    }

    /// A whole-unit world position, which is what most of these expectations are.
    fn at(x: i32, y: i32, z: i32) -> Vec3Fx {
        Vec3Fx::new(Fx::from_i32(x), Fx::from_i32(y), Fx::from_i32(z))
    }

    /// The exact Q16.16 word for `units / divisor`. Every decimal these fixtures write is dyadic,
    /// so the truncating divide is exact and no expectation carries a tolerance.
    fn ratio(units: i32, divisor: i32) -> Fx {
        Fx::from_i32(units) / Fx::from_i32(divisor)
    }

    /// The frame convention: the resolving team's half turn about `+z`, which negates `x` and `y`
    /// together and leaves `z` alone. Team zero's turn is the identity, so this is team one's.
    fn half_turned(vector: Vec3Fx) -> Vec3Fx {
        Vec3Fx::new(-vector.x, -vector.y, vector.z)
    }

    fn body(world: &World, team: Team, local: u8) -> usize {
        world
            .player_index(
                team,
                LocalId::new(local).expect("a fixture local id is in range"),
            )
            .expect("the fixture roster holds that local id")
    }

    /// A ten-a-side world with the game ball placed and the fielder every verb fixture reads parked
    /// on the origin plane. `Jam` is the only verb that reads the resolving body at all, and it is
    /// the only reason that body is moved.
    fn ten_a_side(ball: Vec3Fx) -> World {
        let mut world = World::new(10);
        world.set_position(world.objective_index(), ball);
        let resolving = body(&world, Team::Zero, 1);
        world.set_position(resolving, at(0, 0, 1));
        world
    }

    /// Resolve `node` over `world` and hand back team zero's fielder intents in local-ID order.
    ///
    /// The default squad cycle `[0]` puts the whole roster in one squad, which makes that order the
    /// squad-ordinal order the forms lay out along: index `k` is ordinal `k`.
    fn fielder_intents(node: Node<'_>, world: &mut World) -> Vec<OracleIntent> {
        let playbook = compiled(&[node]);
        let fielders: Vec<usize> = (1..world.active_per_team())
            .map(|local| {
                body(
                    world,
                    Team::Zero,
                    u8::try_from(local).expect("a roster index fits u8"),
                )
            })
            .collect();
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 0], &Arena::default(), world, &mut intents);
        fielders
            .into_iter()
            .map(|index| intents.intents[index])
            .collect()
    }

    #[test]
    fn the_acknowledged_constants_are_the_ones_game_tick_pins() {
        assert_eq!(PLAYBOOK_ABI_VERSION, 2);
        assert_eq!(POSSESSION_TICKS, 30);
        assert_eq!(COVER_GAP, Fx::from_i32(3));
        assert_eq!(PORT_COUNT, 8);
        assert_eq!(SQUAD_COUNT, 8);
    }

    #[test]
    fn the_shipped_call_sheet_round_trips_its_graph() {
        let playbook = Playbook::compile_ron(SHIPPED).expect("the shipped call sheet compiles");
        assert_eq!(playbook.nodes().len(), 2);
        let press = &playbook.nodes()[0];
        let recover = &playbook.nodes()[1];

        assert_eq!(press.name(), "press");
        assert_eq!(recover.name(), "recover");
        assert_eq!(
            press.edges(),
            [
                PlayEdge {
                    to: 1,
                    trigger: Trigger::BallBehind(Fx::from_i32(-8)),
                },
                PlayEdge {
                    to: 0,
                    trigger: Trigger::Always,
                },
            ]
            .as_slice()
        );
        assert_eq!(
            recover.edges(),
            [
                PlayEdge {
                    to: 0,
                    trigger: Trigger::BallPast(Fx::ZERO),
                },
                PlayEdge {
                    to: 0,
                    trigger: Trigger::Elapsed(180),
                },
                PlayEdge {
                    to: 1,
                    trigger: Trigger::Always,
                },
            ]
            .as_slice()
        );
        assert_eq!(press.coach_gate(), Fx::ZERO);
        assert_eq!(recover.coach_gate(), Fx::ZERO);

        // Squad assignment is the authored cycle indexed by local ID, and it wraps past the cycle.
        let assigned = |node: &PlayNode| {
            (0u8..10)
                .map(|local| node.squad_for(local))
                .collect::<Vec<_>>()
        };
        assert_eq!(assigned(press), [0, 1, 2, 3, 4, 5, 6, 7, 0, 1]);
        assert_eq!(assigned(recover), [7, 6, 5, 4, 3, 2, 1, 0, 7, 6]);
    }

    #[test]
    fn the_shipped_call_sheet_round_trips_its_squad_indexed_verb_tables() {
        let playbook = Playbook::compile_ron(SHIPPED).expect("the shipped call sheet compiles");
        let guard_the_ball = VerbEntry {
            verb: Verb::Guard,
            target: Target::GameBall,
            form: Form::Point,
        };
        let hold = VerbEntry {
            verb: Verb::Align,
            target: Target::Slot,
            form: Form::Pod {
                rank: 1,
                file: 11,
                gap: Fx::HALF,
            },
        };
        let point = |verb, target| VerbEntry {
            verb,
            target,
            form: Form::Point,
        };
        let shaped = |verb, target, form| VerbEntry { verb, target, form };

        assert_eq!(playbook.nodes()[0].goalie_verb(), guard_the_ball);
        assert_eq!(
            playbook.nodes()[0].verbs(),
            &[
                point(Verb::Drive, Target::GameBall),
                shaped(Verb::Pursue, Target::GameBall, Form::Wedge(ratio(3, 2)),),
                shaped(
                    Verb::Block,
                    Target::NearestOpponent,
                    Form::Pod {
                        rank: 2,
                        file: 3,
                        gap: ratio(3, 2),
                    },
                ),
                shaped(Verb::Lead, Target::GameBall, Form::Arc(Fx::from_i32(2)),),
                point(Verb::Cover, Target::NearestOpponent),
                shaped(
                    Verb::Zone,
                    Target::GameBall,
                    Form::Pod {
                        rank: 1,
                        file: 4,
                        gap: ratio(5, 2),
                    },
                ),
                point(Verb::Sweep, Target::GameBall),
                hold,
            ]
        );

        assert_eq!(playbook.nodes()[1].goalie_verb(), guard_the_ball);
        assert_eq!(
            playbook.nodes()[1].verbs(),
            &[
                point(Verb::Clear, Target::GameBall),
                point(Verb::Cover, Target::NearestOpponent),
                point(Verb::Cover, Target::NearestOpponent),
                shaped(
                    Verb::Zone,
                    Target::GameBall,
                    Form::Pod {
                        rank: 1,
                        file: 3,
                        gap: ratio(5, 2),
                    },
                ),
                shaped(
                    Verb::Sweep,
                    Target::GameBall,
                    Form::Pod {
                        rank: 1,
                        file: 3,
                        gap: Fx::from_i32(3),
                    },
                ),
                point(Verb::Jam, Target::NearestOpponent),
                hold,
                hold,
            ]
        );
    }

    #[test]
    fn every_trigger_in_the_closed_vocabulary_round_trips() {
        // Eight triggers, eight ports, `Always` last: the whole vocabulary fits one node exactly.
        let node = Node {
            edges: "[(to: 0, trigger: Elapsed(180)),
                     (to: 0, trigger: BallPast(2.5)),
                     (to: 0, trigger: BallBehind(-1.5)),
                     (to: 0, trigger: BallAloft(0.5)),
                     (to: 0, trigger: Possession(Teammate)),
                     (to: 0, trigger: Lead(-2)),
                     (to: 0, trigger: CoachEdge),
                     (to: 0, trigger: Always)]",
            ..Node::default()
        };
        let port = |trigger| PlayEdge { to: 0, trigger };
        assert_eq!(
            compiled(&[node]).nodes()[0].edges(),
            [
                port(Trigger::Elapsed(180)),
                port(Trigger::BallPast(ratio(5, 2))),
                port(Trigger::BallBehind(-ratio(3, 2))),
                port(Trigger::BallAloft(Fx::HALF)),
                port(Trigger::Possession(Relation::Teammate)),
                port(Trigger::Lead(-2)),
                port(Trigger::CoachEdge),
                port(Trigger::Always),
            ]
            .as_slice()
        );

        // `Possession` closes over three of the five perception relations, and all three are
        // written.
        for (written, expected) in [
            ("Teammate", Relation::Teammate),
            ("Opponent", Relation::Opponent),
            ("Neutral", Relation::Neutral),
        ] {
            let edges =
                format!("[(to: 0, trigger: Possession({written})), (to: 0, trigger: Always)]");
            let playbook = compiled(&[Node {
                edges: &edges,
                ..Node::default()
            }]);
            assert_eq!(
                playbook.nodes()[0].edges()[0].trigger,
                Trigger::Possession(expected),
                "{written}"
            );
        }
    }

    #[test]
    fn every_verb_in_the_closed_vocabulary_round_trips() {
        const NAMES: [&str; 11] = [
            "Align", "Pursue", "Drive", "Clear", "Cover", "Zone", "Sweep", "Block", "Lead", "Jam",
            "Guard",
        ];
        const VERBS: [Verb; 11] = [
            Verb::Align,
            Verb::Pursue,
            Verb::Drive,
            Verb::Clear,
            Verb::Cover,
            Verb::Zone,
            Verb::Sweep,
            Verb::Block,
            Verb::Lead,
            Verb::Jam,
            Verb::Guard,
        ];

        // The table is exactly eight squads wide, so eleven verbs need a second node to land in.
        let written = NAMES.map(|verb| entry(verb, "GameBall", "Point"));
        let (front, back) = written.split_at(SQUAD_COUNT);
        let playbook = compiled(&[
            Node {
                verbs: &borrowed(front),
                ..Node::default()
            },
            Node {
                name: "b",
                verbs: &borrowed(back),
                ..Node::default()
            },
        ]);

        let read: Vec<VerbEntry> = playbook.nodes()[0]
            .verbs()
            .iter()
            .chain(playbook.nodes()[1].verbs()[..back.len()].iter())
            .copied()
            .collect();
        assert_eq!(
            read,
            VERBS
                .map(|verb| VerbEntry {
                    verb,
                    target: Target::GameBall,
                    form: Form::Point,
                })
                .to_vec()
        );

        // The goalie slot takes the same grammar as a squad slot.
        assert_eq!(
            playbook.nodes()[0].goalie_verb(),
            VerbEntry {
                verb: Verb::Guard,
                target: Target::GameBall,
                form: Form::Point,
            }
        );
    }

    #[test]
    fn every_target_in_the_closed_vocabulary_round_trips() {
        const NAMES: [&str; 7] = [
            "GameBall",
            "OwnGoal",
            "OpponentGoal",
            "Squad(7)",
            "NearestOpponent",
            "NearestToMe",
            "Slot",
        ];
        const TARGETS: [Target; 7] = [
            Target::GameBall,
            Target::OwnGoal,
            Target::OpponentGoal,
            Target::Squad(7),
            Target::NearestOpponent,
            Target::NearestToMe,
            Target::Slot,
        ];

        let written = NAMES.map(|target| entry("Pursue", target, "Point"));
        let playbook = compiled(&[Node {
            verbs: &borrowed(&written),
            ..Node::default()
        }]);
        let read: Vec<Target> = playbook.nodes()[0].verbs()[..NAMES.len()]
            .iter()
            .map(|entry| entry.target)
            .collect();
        assert_eq!(read, TARGETS.to_vec());
    }

    #[test]
    fn every_form_in_the_closed_vocabulary_round_trips() {
        const NAMES: [&str; 4] = ["Point", "Pod(2, 3, 1.5)", "Wedge(2.0)", "Arc(0.5)"];

        let written = NAMES.map(|form| entry("Align", "Slot", form));
        let playbook = compiled(&[Node {
            verbs: &borrowed(&written),
            ..Node::default()
        }]);
        let read: Vec<Form> = playbook.nodes()[0].verbs()[..NAMES.len()]
            .iter()
            .map(|entry| entry.form)
            .collect();
        assert_eq!(
            read,
            vec![
                Form::Point,
                Form::Pod {
                    rank: 2,
                    file: 3,
                    gap: ratio(3, 2),
                },
                Form::Wedge(Fx::from_i32(2)),
                Form::Arc(Fx::HALF),
            ]
        );
    }

    #[test]
    fn a_version_one_file_is_rejected_rather_than_migrated() {
        let source = at_version(1, &[Node::default()]);
        assert_eq!(
            Playbook::compile_ron(&source),
            Err(PlaybookError::Version(1))
        );
    }

    #[test]
    fn every_retired_verb_name_is_rejected() {
        for retired in ["Strike", "Mark", "Screen", "Hold", "Chase", "Post", "Punt"] {
            let written = entry(retired, "GameBall", "Point");
            assert_eq!(
                rejection(&[Node {
                    verbs: &[&written],
                    ..Node::default()
                }]),
                PlaybookError::UnknownVerb(retired.to_owned())
            );
        }
    }

    #[test]
    fn every_retired_form_name_is_rejected() {
        for retired in ["Line", "Column", "Ranks", "Grid"] {
            let written = entry("Align", "Slot", retired);
            assert_eq!(
                rejection(&[Node {
                    verbs: &[&written],
                    ..Node::default()
                }]),
                PlaybookError::UnknownForm(retired.to_owned())
            );
        }
    }

    #[test]
    fn a_name_outside_the_trigger_target_or_relation_vocabulary_is_rejected() {
        assert_eq!(
            rejection(&[Node {
                edges: "[(to: 0, trigger: BallLoose(1.0)), (to: 0, trigger: Always)]",
                ..Node::default()
            }]),
            PlaybookError::UnknownTrigger("BallLoose".to_owned())
        );

        let written = entry("Pursue", "Ball", "Point");
        assert_eq!(
            rejection(&[Node {
                verbs: &[&written],
                ..Node::default()
            }]),
            PlaybookError::UnknownTarget("Ball".to_owned())
        );

        // `Arena` and `Goal` are perception relations the acknowledged `Possession` operand
        // excludes.
        for outside in ["Arena", "Goal"] {
            let edges =
                format!("[(to: 0, trigger: Possession({outside})), (to: 0, trigger: Always)]");
            assert_eq!(
                rejection(&[Node {
                    edges: &edges,
                    ..Node::default()
                }]),
                PlaybookError::UnknownRelation(outside.to_owned())
            );
        }
    }

    #[test]
    fn a_last_port_that_is_not_always_cannot_deliberately_leave() {
        assert_eq!(
            rejection(&[Node {
                edges: "[(to: 0, trigger: Always), (to: 0, trigger: Elapsed(1))]",
                ..Node::default()
            }]),
            PlaybookError::MissingAlwaysPort("a".to_owned())
        );
    }

    #[test]
    fn a_verb_table_that_is_not_eight_entries_is_rejected() {
        let short = book(&[Node::default()]).replacen(&format!("{HOLD},"), "", 1);
        assert_eq!(
            Playbook::compile_ron(&short),
            Err(PlaybookError::VerbCount {
                node: "a".to_owned(),
                count: 7,
            })
        );

        assert_eq!(
            rejection(&[Node {
                verbs: &[HOLD; SQUAD_COUNT + 1],
                ..Node::default()
            }]),
            PlaybookError::VerbCount {
                node: "a".to_owned(),
                count: 9,
            }
        );
    }

    #[test]
    fn a_squad_above_seven_is_rejected_in_the_cycle_and_in_a_target() {
        assert_eq!(
            rejection(&[Node {
                squad_cycle: "[0, 8]",
                ..Node::default()
            }]),
            PlaybookError::InvalidSquad("a".to_owned())
        );

        let written = entry("Pursue", "Squad(8)", "Point");
        assert_eq!(
            rejection(&[Node {
                verbs: &[&written],
                ..Node::default()
            }]),
            PlaybookError::InvalidSquad("a".to_owned())
        );

        // A cycle that assigns nobody cannot name a squad either.
        assert_eq!(
            rejection(&[Node {
                squad_cycle: "[]",
                ..Node::default()
            }]),
            PlaybookError::InvalidSquad("a".to_owned())
        );
    }

    #[test]
    fn a_dangling_edge_is_rejected() {
        assert_eq!(
            rejection(&[Node {
                edges: "[(to: 9, trigger: Always)]",
                ..Node::default()
            }]),
            PlaybookError::DanglingEdge {
                node: "a".to_owned(),
                target: 9,
            }
        );
    }

    #[test]
    fn a_duplicate_node_name_is_rejected() {
        assert_eq!(
            rejection(&[Node::default(), Node::default()]),
            PlaybookError::DuplicateName("a".to_owned())
        );
    }

    #[test]
    fn an_edge_count_outside_one_through_eight_is_rejected() {
        assert_eq!(
            rejection(&[Node {
                edges: "[]",
                ..Node::default()
            }]),
            PlaybookError::EdgeCount {
                node: "a".to_owned(),
                count: 0,
            }
        );

        let overfull = format!("[{}]", "(to: 0, trigger: Always),".repeat(PORT_COUNT + 1));
        assert_eq!(
            rejection(&[Node {
                edges: &overfull,
                ..Node::default()
            }]),
            PlaybookError::EdgeCount {
                node: "a".to_owned(),
                count: 9,
            }
        );
    }

    #[test]
    fn a_pod_with_a_zero_rank_or_file_is_rejected() {
        for degenerate in ["Pod(0, 3, 1.5)", "Pod(2, 0, 1.5)"] {
            let written = entry("Align", "Slot", degenerate);
            assert_eq!(
                rejection(&[Node {
                    verbs: &[&written],
                    ..Node::default()
                }]),
                PlaybookError::PodExtent("a".to_owned()),
                "{degenerate}"
            );
        }
    }

    /// Determinism rule 1's obligation on authored data: a gap large enough to drive the layout
    /// arithmetic out of the `i32` domain would panic the CPU tier and silently produce garbage on
    /// the GPU tier, so the compile boundary rejects it the way it rejects an oversized pod.
    #[test]
    fn a_form_gap_that_could_overflow_the_layout_arithmetic_is_rejected() {
        for oversized in ["Pod(1, 1, 101.0)", "Wedge(-101.0)", "Arc(101.0)"] {
            let written = entry("Align", "Slot", oversized);
            assert_eq!(
                rejection(&[Node {
                    verbs: &[&written],
                    ..Node::default()
                }]),
                PlaybookError::FormGap("a".to_owned()),
                "{oversized}"
            );
        }

        // The bound is inclusive, and a bounded sheet resolves for the full roster without
        // leaving the arithmetic domain — which is the property the bound exists to buy.
        let widest = entry("Align", "Slot", "Pod(1, 1, 100.0)");
        let playbook = compiled(&[Node {
            verbs: &[&widest],
            ..Node::default()
        }]);
        let mut world = World::new(100);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 0], &Arena::default(), &mut world, &mut intents);
    }

    /// The proposal names a widened component sum for `Squad(n)`: three same-squad bodies parked
    /// near the positive coordinate boundary must average exactly, not wrap through `i32`.
    #[test]
    fn a_squad_centroid_is_a_widened_sum_rather_than_a_wrapping_one() {
        // Locals congruent to two mod three take squad seven: locals 2, 5, and 8.
        let far = at(30_000, 0, 1);
        let mut world = World::new(10);
        for local in [2u8, 5, 8] {
            let index = body(&world, Team::Zero, local);
            world.set_position(index, far);
        }

        let written = entry("Pursue", "Squad(7)", "Point");
        let intents = fielder_intents(
            Node {
                squad_cycle: "[0, 0, 7]",
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        assert_eq!(intents[0].position, far);
    }

    /// Each row is the proposal's verb table read straight across: the aim point is written as the
    /// construction the table names, not as whatever the solver happens to emit.
    ///
    /// The world is one small fixture — the game ball at `x = 8` on the goal-to-goal line, the own
    /// goal at `x = -16` and the opposing goal at `x = 16`, and the resolving fielder on the origin
    /// plane. Every construction axis is therefore exactly `±x` and every normalization is exact.
    ///
    /// "One radius" and `COVER_GAP` are absolute lengths — `GAME_TICK.md` fixes the length unit at
    /// one ball radius, so the offsets are `1` and `3`, never rescaled by a live sphere's width.
    #[test]
    fn every_verb_builds_the_aim_point_the_proposal_tables() {
        let radius = Fx::ONE;
        let gap = COVER_GAP;
        let ball = at(8, 0, 1);
        let slot = at(2, 4, 1);
        let own_goal = at(-16, 0, 1);
        let opponent_goal = at(16, 0, 1);

        for (verb, target, expected) in [
            // the role's node template position, whatever the entry's target says
            ("Align", "Slot", slot),
            ("Align", "GameBall", slot),
            // the target's position
            ("Pursue", "GameBall", ball),
            // one radius behind the target along target → opposing goal
            ("Drive", "GameBall", ball - Vec3Fx::X * radius),
            // one radius behind the target along own goal → target
            ("Clear", "OpponentGoal", opponent_goal - Vec3Fx::X * radius),
            // Each of the pair aimed at its own reference normalizes a zero-length direction, and
            // the spec's degenerate case falls the verb back to the target with no epsilon in the
            // way. These two rows are also what separates the pair: on the goal-to-goal line every
            // other ball position leaves both constructions pointing the same way.
            ("Drive", "OpponentGoal", opponent_goal),
            ("Clear", "OwnGoal", own_goal),
            // COVER_GAP from the target toward the own goal
            ("Cover", "GameBall", ball - Vec3Fx::X * gap),
            // the midpoint between the target and the own goal, every component halved
            ("Zone", "Slot", at(-7, 2, 1)),
            // the `Zone` depth in x, the target's own y and z
            ("Sweep", "Slot", at(-7, 4, 1)),
            // COVER_GAP from the target toward the game ball
            ("Block", "OwnGoal", own_goal + Vec3Fx::X * gap),
            // COVER_GAP from the target toward the opposing goal
            ("Lead", "GameBall", ball + Vec3Fx::X * gap),
            // one radius past the target along resolving body → target
            ("Jam", "GameBall", ball + Vec3Fx::X * radius),
            // the own goal-mouth plane, y clamped to the mouth half-width of two
            ("Guard", "Slot", at(-16, 2, 1)),
        ] {
            let written = entry(verb, target, "Point");
            let mut world = ten_a_side(ball);
            let intents = fielder_intents(
                Node {
                    verbs: &[&written],
                    ..Node::default()
                },
                &mut world,
            );
            assert_eq!(intents[0].position, expected, "{written}");
        }
    }

    /// The role intent templates round-trip through `Align`, which is the one verb that reads them
    /// and the one verb that emits a spin at all.
    #[test]
    fn only_align_emits_the_node_templates_position_and_spin() {
        let hold = entry("Align", "Slot", "Point");
        let playbook = compiled(&[Node {
            goalie_verb: &hold,
            ..Node::default()
        }]);
        let mut world = World::new(10);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 0], &Arena::default(), &mut world, &mut intents);

        let read = |team, local| intents.intents[body(&world, team, local)];
        let goalie_spin = Vec3Fx::new(-Fx::ONE, Fx::HALF, ratio(1, 4));
        assert_eq!(
            read(Team::Zero, 0),
            OracleIntent {
                position: at(-14, 3, 1),
                spin: goalie_spin,
            }
        );
        assert_eq!(
            read(Team::One, 0),
            OracleIntent {
                position: half_turned(at(-14, 3, 1)),
                spin: half_turned(goalie_spin),
            }
        );
        assert_eq!(
            read(Team::Zero, 1),
            OracleIntent {
                position: at(2, 4, 1),
                spin: at(1, 2, 3),
            }
        );
        assert_eq!(
            read(Team::One, 1),
            OracleIntent {
                position: half_turned(at(2, 4, 1)),
                spin: half_turned(at(1, 2, 3)),
            }
        );

        // Every other verb emits a zero spin target, whatever the template says.
        let pursue = entry("Pursue", "GameBall", "Point");
        let mut world = ten_a_side(at(8, 0, 1));
        let intents = fielder_intents(
            Node {
                verbs: &[&pursue],
                ..Node::default()
            },
            &mut world,
        );
        assert_eq!(intents[0].spin, Vec3Fx::ZERO);
    }

    /// `Pursue` aims at the target and nothing else, so each row reads the target resolution alone.
    #[test]
    fn every_target_resolves_to_the_reference_the_proposal_names() {
        // The cycle sends local IDs congruent to seven to squad seven, which at a ten-a-side roster
        // is local seven alone, so that squad's centroid is one body a fixture places.
        let cycle = "[0, 0, 0, 0, 0, 0, 0, 7]";
        let ball = at(8, 0, 1);
        let mut world = World::new(10);
        world.set_position(world.objective_index(), ball);
        let lone_member = body(&world, Team::Zero, 7);
        world.set_position(lone_member, at(3, -5, 2));
        let closest_to_the_ball = body(&world, Team::One, 3);
        world.set_position(closest_to_the_ball, at(7, 1, 1));
        // The resolving body is team zero's local one, parked at its spawn: this opponent stands
        // beside it and nowhere near the ball, so the two nearest-opponent rows cannot share an
        // answer.
        let closest_to_the_body = body(&world, Team::One, 5);
        world.set_position(closest_to_the_body, at(-4, -4, 1));

        for (target, expected) in [
            ("GameBall", ball),
            ("OwnGoal", at(-16, 0, 1)),
            ("OpponentGoal", at(16, 0, 1)),
            ("Squad(7)", at(3, -5, 2)),
            ("NearestOpponent", at(7, 1, 1)),
            ("NearestToMe", at(-4, -4, 1)),
            ("Slot", at(2, 4, 1)),
            // An empty squad has no centroid and falls back to the role's slot.
            ("Squad(6)", at(2, 4, 1)),
        ] {
            let written = entry("Pursue", target, "Point");
            let intents = fielder_intents(
                Node {
                    squad_cycle: cycle,
                    verbs: &[&written],
                    ..Node::default()
                },
                &mut world,
            );
            assert_eq!(intents[0].position, expected, "{target}");
        }
    }

    /// The proposal breaks `NearestOpponent` ties on the lowest canonical body index. Two
    /// opponents at exactly 5 r from the ball — a 3-4-5 pair mirrored on `x` — tie to the raw
    /// Q16.16 word, and the lower-indexed of the two must win.
    #[test]
    fn nearest_opponent_ties_break_on_the_lowest_canonical_body_index() {
        let ball = at(0, 0, 1);
        let mut world = World::new(10);
        world.set_position(world.objective_index(), ball);
        let lower = body(&world, Team::One, 3);
        let higher = body(&world, Team::One, 5);
        assert!(lower < higher);
        world.set_position(lower, at(3, 4, 1));
        world.set_position(higher, at(-3, 4, 1));

        let written = entry("Pursue", "NearestOpponent", "Point");
        let intents = fielder_intents(
            Node {
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        assert_eq!(intents[0].position, at(3, 4, 1));
    }

    /// `NearestOpponent` and `NearestToMe` are one construction against two references, and the
    /// seventh target earns its slot only where the two genuinely disagree. Here the game ball sits
    /// downfield with one opponent beside it while a second stands on the resolving body, so
    /// ball-nearest and me-nearest are different bodies.
    ///
    /// A third opponent parked on a *second* member of the same squad reads the other half of the
    /// verdict: `NearestOpponent` is one anchor the whole squad shares, while `NearestToMe` takes a
    /// minimum per body and hands each member a different one.
    #[test]
    fn nearest_to_me_ranks_against_the_resolving_body_rather_than_the_game_ball() {
        let ball = at(12, 0, 1);
        let by_the_ball = at(12, 1, 1);
        let by_the_first = at(-1, 0, 1);
        let by_the_second = at(-1, -9, 1);

        // `ten_a_side` parks team zero's local one on the origin plane; local two joins it out on
        // the wing, and the two share squad zero under the default cycle.
        let mut world = ten_a_side(ball);
        world.set_position(body(&world, Team::Zero, 2), at(0, -10, 1));
        world.set_position(body(&world, Team::One, 4), by_the_ball);
        world.set_position(body(&world, Team::One, 6), by_the_first);
        world.set_position(body(&world, Team::One, 7), by_the_second);

        for (target, first, second) in [
            ("NearestOpponent", by_the_ball, by_the_ball),
            ("NearestToMe", by_the_first, by_the_second),
        ] {
            let written = entry("Pursue", target, "Point");
            let intents = fielder_intents(
                Node {
                    verbs: &[&written],
                    ..Node::default()
                },
                &mut world,
            );
            assert_eq!(intents[0].position, first, "{target} at local one");
            assert_eq!(intents[1].position, second, "{target} at local two");
        }
    }

    /// `NearestToMe` breaks ties on the same rule against its own reference. Two opponents mirrored
    /// about the resolving body — a 3-4-5 pair either side of it — tie to the raw Q16.16 word, and
    /// the lower-indexed of the two must win. The game ball is parked where it would pick the
    /// *higher* of the pair, so a resolution that leaked back to `NearestOpponent` reads the wrong
    /// body rather than passing by accident.
    #[test]
    fn nearest_to_me_ties_break_on_the_lowest_canonical_body_index() {
        let mut world = ten_a_side(at(-12, 0, 1));
        let lower = body(&world, Team::One, 3);
        let higher = body(&world, Team::One, 5);
        assert!(lower < higher);
        world.set_position(lower, at(3, 4, 1));
        world.set_position(higher, at(-3, 4, 1));

        let written = entry("Pursue", "NearestToMe", "Point");
        let intents = fielder_intents(
            Node {
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        assert_eq!(intents[0].position, at(3, 4, 1));
    }

    /// Verdict B's boundary read from the other side: only the authored template turns, so every
    /// world-derived reference must reach a team-one body verbatim. Team zero's turn is the
    /// identity and the covariance fixture keeps the ball on the turn's fixed axis, so neither can
    /// see a reference turned twice — this fixture parks each reference off the axis, where a
    /// doubly-turned ball, body, or centroid negates two components and a doubly-turned goal
    /// swaps ends.
    #[test]
    fn every_world_derived_reference_reaches_team_one_unturned() {
        let cycle = "[0, 0, 0, 0, 0, 0, 0, 7]";
        let ball = at(8, 3, 1);
        let mut world = World::new(10);
        world.set_position(world.objective_index(), ball);
        let resolving = body(&world, Team::One, 1);
        world.set_position(resolving, at(0, -6, 1));
        let lone_member = body(&world, Team::One, 7);
        world.set_position(lone_member, at(3, -5, 2));
        let closest_to_the_ball = body(&world, Team::Zero, 3);
        world.set_position(closest_to_the_ball, at(7, 4, 1));
        let closest_to_the_body = body(&world, Team::Zero, 5);
        world.set_position(closest_to_the_body, at(1, -7, 1));

        for (target, expected) in [
            ("GameBall", ball),
            ("OwnGoal", at(16, 0, 1)),
            ("OpponentGoal", at(-16, 0, 1)),
            ("Squad(7)", at(3, -5, 2)),
            ("NearestOpponent", at(7, 4, 1)),
            ("NearestToMe", at(1, -7, 1)),
        ] {
            let written = entry("Pursue", target, "Point");
            let playbook = compiled(&[Node {
                squad_cycle: cycle,
                verbs: &[&written],
                ..Node::default()
            }]);
            let mut intents = OracleIntentBatch::with_len(world.view().len());
            playbook.resolve([0, 0], &Arena::default(), &mut world, &mut intents);
            assert_eq!(intents.intents[resolving].position, expected, "{target}");
        }
    }

    /// `Align` takes no construction axis, so forward is the attacking `+x` and lateral is `-y`: a
    /// positive alternating step walks toward `-y` and depth walks toward `-x`.
    #[test]
    fn a_pod_tiles_a_squad_into_consecutive_pods_laid_out_laterally() {
        let written = entry("Align", "Slot", "Pod(2, 3, 2.0)");
        let mut world = World::new(100);
        let intents = fielder_intents(
            Node {
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        let placed = |ordinal: usize| intents[ordinal].position;

        // Pod zero is three across about the aim, then drops a rank.
        assert_eq!(placed(0), at(2, 6, 1));
        assert_eq!(placed(1), at(2, 4, 1));
        assert_eq!(placed(2), at(2, 2, 1));
        assert_eq!(placed(3), at(0, 6, 1));

        // Pod one lays out laterally at the `(file + 1) * gap` stride, leaving one empty gap
        // between the pods; pod two takes the same step to the other side.
        assert_eq!(placed(6), at(2, -2, 1));
        assert_eq!(placed(12), at(2, 14, 1));
        let stride = Fx::from_i32(3 + 1) * Fx::from_i32(2);
        assert_eq!(placed(0).y - placed(6).y, stride);
        assert_eq!(placed(12).y - placed(0).y, stride);
    }

    #[test]
    fn a_wedge_expands_by_the_alternating_step_and_sets_each_body_back_by_its_magnitude() {
        let written = entry("Align", "Slot", "Wedge(2.0)");
        let mut world = World::new(10);
        let intents = fielder_intents(
            Node {
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        let placed: Vec<Vec3Fx> = intents
            .iter()
            .take(5)
            .map(|intent| intent.position)
            .collect();
        assert_eq!(
            placed,
            vec![
                at(2, 4, 1),
                at(0, 2, 1),
                at(0, 6, 1),
                at(-2, 0, 1),
                at(-2, 8, 1),
            ]
        );
    }

    /// `Arc` steps along the chord and renormalizes onto the circle of radius `qlen(aim - target)`.
    ///
    /// The fixture puts the aim four across and three deep of the target, so that radius is exactly
    /// five and the ordinal whose chord carries the spoke onto the `+x` axis lands on the circle
    /// exactly. Off the axis `qnorm` truncates toward zero, and the ordinal-zero literal below is
    /// that truncation written out: `4/5` and `3/5` become 52 428 and 39 321 raw, and five of each
    /// is 262 140 and 196 605.
    #[test]
    fn an_arc_steps_along_the_chord_and_lands_on_the_circle_through_the_aim() {
        let target = at(0, 0, 1);
        let spread = |gap: &str| {
            let written = entry("Align", "GameBall", gap);
            let mut world = World::new(10);
            world.set_position(world.objective_index(), target);
            fielder_intents(
                Node {
                    verbs: &[&written],
                    fielder: "(position: [4.0, 3.0, 1.0], spin: [0.0, 0.0, 0.0])",
                    ..Node::default()
                },
                &mut world,
            )
        };

        let wide = spread("Arc(3.0)");
        assert_eq!(
            wide[0].position,
            Vec3Fx::new(Fx::from_raw(262_140), Fx::from_raw(196_605), Fx::ONE)
        );
        assert_eq!(wide[1].position, at(5, 0, 1));
        assert_eq!(
            (wide[1].position - target).length(),
            Fx::from_i32(5),
            "the chord step is renormalized back onto the circle"
        );

        // Half the gap needs twice the step to reach the same point, which pins `step(3) == +2`.
        let narrow = spread("Arc(1.5)");
        assert_eq!(narrow[3].position, at(5, 0, 1));
    }

    #[test]
    fn every_form_collapses_to_point_when_its_frame_degenerates() {
        // `Cover` builds its axis along target → own goal. A ball directly above the goal mouth
        // leaves that axis with no floor-plane direction, so `qnorm` returns zero, lateral goes
        // with it, and no form has anywhere to displace a member to.
        let ball = at(-16, 0, 5);
        let aim = Vec3Fx::new(Fx::from_i32(-16), Fx::ZERO, Fx::from_i32(5) - COVER_GAP);
        for form in ["Point", "Pod(2, 3, 2.0)", "Wedge(2.0)", "Arc(3.0)"] {
            let written = entry("Cover", "GameBall", form);
            let mut world = ten_a_side(ball);
            let intents = fielder_intents(
                Node {
                    verbs: &[&written],
                    ..Node::default()
                },
                &mut world,
            );
            for (ordinal, intent) in intents.iter().enumerate().take(5) {
                assert_eq!(intent.position, aim, "{form} ordinal {ordinal}");
            }
        }

        // An `Arc` whose aim already sits on its target has no circle to spread along even where
        // the frame is sound: `qlen` is zero and the renormalization is zero in, zero out.
        let written = entry("Pursue", "GameBall", "Arc(3.0)");
        let ball = at(8, 0, 1);
        let mut world = ten_a_side(ball);
        let intents = fielder_intents(
            Node {
                verbs: &[&written],
                ..Node::default()
            },
            &mut world,
        );
        for (ordinal, intent) in intents.iter().enumerate().take(5) {
            assert_eq!(intent.position, ball, "Arc ordinal {ordinal}");
        }
    }

    /// One authored play reads the same for both teams. The frame convention is a half turn about
    /// `+z`, so the *whole* resolution has to be covariant under it: the authored slot, the
    /// world-derived aim point built off it, and the formation hung off that. Every verb, every
    /// world-reading target, and a form on a squad that fills more than its anchor all run here, and
    /// each team's intent must come out the other's turned.
    ///
    /// That is a statement about the play file, so the world must not smuggle an asymmetry into it.
    /// `World::new` spawns the two rosters as `x`-mirror images, which the retired `x`-only frame
    /// made symmetric and a half turn does not: `Jam` and the two nearest-opponent targets read
    /// body positions, and against an `x`-mirrored roster they answer honestly different questions.
    /// The fixture therefore turns team one's roster onto team zero's. The game ball spawns on the
    /// turn's fixed axis, and the two goal mouths are each other's images under the turn just as
    /// the own/opponent labels are, so neither needs moving.
    #[test]
    fn one_authored_play_resolves_by_a_half_turn_for_the_two_teams() {
        let cycle = "[0, 1, 2, 3, 4, 5, 6, 7]";
        let front = [
            entry("Align", "Slot", "Point"),
            // Squad one holds locals one and nine at this roster, so its ordinal-one member is
            // displaced by the form rather than sitting on the anchor.
            entry("Pursue", "GameBall", "Wedge(2.0)"),
            entry("Drive", "GameBall", "Point"),
            entry("Clear", "GameBall", "Point"),
            entry("Cover", "NearestOpponent", "Point"),
            entry("Zone", "GameBall", "Point"),
            entry("Sweep", "GameBall", "Point"),
            entry("Block", "NearestOpponent", "Point"),
        ];
        let back = [
            entry("Lead", "GameBall", "Point"),
            entry("Jam", "NearestOpponent", "Arc(2.0)"),
            entry("Guard", "GameBall", "Point"),
            entry("Cover", "NearestToMe", "Point"),
        ];
        let align = entry("Align", "Slot", "Point");
        let playbook = compiled(&[
            Node {
                squad_cycle: cycle,
                verbs: &borrowed(&front),
                ..Node::default()
            },
            Node {
                name: "b",
                squad_cycle: cycle,
                goalie_verb: &align,
                verbs: &borrowed(&back),
                ..Node::default()
            },
        ]);

        let mut world = World::new(10);
        for local in 0..10u8 {
            let turned = half_turned(world.view().positions[body(&world, Team::Zero, local)]);
            world.set_position(body(&world, Team::One, local), turned);
        }

        let mut intents = OracleIntentBatch::with_len(world.view().len());
        for cursor in 0..playbook.nodes().len() {
            playbook.resolve(
                [cursor, cursor],
                &Arena::default(),
                &mut world,
                &mut intents,
            );
            for local in 0..10u8 {
                let zero = intents.intents[body(&world, Team::Zero, local)];
                let one = intents.intents[body(&world, Team::One, local)];
                assert_eq!(
                    one.position,
                    half_turned(zero.position),
                    "node {cursor} local {local} position"
                );
                assert_eq!(
                    one.spin,
                    half_turned(zero.spin),
                    "node {cursor} local {local} spin"
                );
            }
        }

        // The covariance above would hold vacuously if the form never displaced anybody, so the
        // wedge's ordinal-one member is pinned outright: one gap along its own team's lateral and
        // one gap back along its own forward, from an anchor that is the ball either way.
        playbook.resolve([0, 0], &Arena::default(), &mut world, &mut intents);
        let placed = |team, local| intents.intents[body(&world, team, local)].position;
        let ball = world.view().positions[world.objective_index()];
        assert_eq!(placed(Team::Zero, 1), ball);
        assert_eq!(
            placed(Team::Zero, 9),
            ball + at(-2, -2, 0),
            "team zero's lateral is `-y` and its depth `-x`"
        );
        assert_eq!(
            placed(Team::One, 9),
            ball + at(2, 2, 0),
            "team one's frame is the same one turned"
        );
    }

    /// Verdict 1 reaches the assignments, not just the squad numbers: one pass over the pair of
    /// cursors must read each team's own node for its verb entry and its role template too. Two
    /// nodes whose fielder templates differ in every component resolve together, so a cursor read
    /// from the wrong team lands on the wrong template rather than merely the wrong squad.
    #[test]
    fn each_teams_intent_resolves_against_its_own_cursor() {
        let playbook = compiled(&[
            Node::default(),
            Node {
                name: "b",
                fielder: "(position: [5.0, 6.0, 1.0], spin: [0.0, 0.0, 0.0])",
                ..Node::default()
            },
        ]);
        let mut world = World::new(10);
        let mut intents = OracleIntentBatch::with_len(world.view().len());
        playbook.resolve([0, 1], &Arena::default(), &mut world, &mut intents);

        let read = |team, local| intents.intents[body(&world, team, local)].position;
        assert_eq!(read(Team::Zero, 1), at(2, 4, 1), "node `a`'s template");
        assert_eq!(read(Team::One, 1), at(-5, -6, 1), "node `b`'s, turned");
    }

    /// A backend that publishes one standing edge-logit lane per team, so a fixture can gate a
    /// `CoachEdge` port without standing a coach population up behind it.
    #[derive(Debug, Clone, Copy)]
    struct StandingCoach {
        logits: [[Fx; PORT_COUNT]; 2],
    }

    impl ControllerBackend for StandingCoach {
        fn act(&mut self, _request: ActRequest<'_>, commands: &mut MotorCommandBatch) {
            commands.clear();
        }

        fn learn(&mut self, _tick: u64, _rewards: &RewardBatch) {}

        fn edge_logits(&self, team: Team) -> [Fx; PORT_COUNT] {
            self.logits[team.index()]
        }

        fn checkpoint(&self, output: &mut Vec<u8>) {
            output.clear();
        }

        fn restore(&mut self, input: &[u8]) -> Result<(), CheckpointError> {
            if input.is_empty() {
                Ok(())
            } else {
                Err(CheckpointError::Malformed)
            }
        }

        fn controller_hash(&self) -> u64 {
            0
        }

        fn learning_hash(&self) -> u64 {
            0
        }
    }

    /// A two-node sheet whose first node leaves only on a coach edge and whose second returns
    /// unconditionally, so the cursor's position after each tick reports exactly which ticks that
    /// `CoachEdge` port was live on. The two nodes assign different squads, so the resolved
    /// assignment reports which cursor each team resolved against.
    fn gated() -> Playbook {
        compiled(&[
            Node {
                name: "hold",
                edges: "[(to: 1, trigger: CoachEdge), (to: 0, trigger: Always)]",
                coach_gate: "0.25",
                ..Node::default()
            },
            Node {
                name: "commit",
                edges: "[(to: 0, trigger: Always)]",
                squad_cycle: "[7]",
                ..Node::default()
            },
        ])
    }

    /// Above `hold`'s quarter gate.
    const CLEARING: Fx = Fx::from_raw(20_480);
    /// Below it, and deliberately not zero: a lane that speaks and is refused.
    const BLOCKED: Fx = Fx::from_raw(8_192);

    /// Verdict 1: a team's coach logits gate only that team's transitions, and each team resolves
    /// against its own cursor. Both teams sit on the same node with the same port, and swapping
    /// which team's lane clears the gate swaps which cursor moves.
    #[test]
    fn each_teams_cursor_follows_only_its_own_edge_lane() {
        for team in Team::ALL {
            let mut logits = [[BLOCKED; PORT_COUNT]; 2];
            logits[team.index()][0] = CLEARING;
            let mut game = Match::new(MatchConfig::default(), gated(), StandingCoach { logits });

            game.tick();
            assert_eq!(
                [game.play_node(Team::Zero), game.play_node(Team::One)],
                [0, 0],
                "tick zero is the coach's own pulse, whose lanes step 2 cannot read yet"
            );

            game.tick();
            assert_eq!(game.play_node(team), 1);
            assert_eq!(game.play_node(team.opponent()), 0);

            let squads = game.world().view().squads;
            assert_eq!(squads[body(game.world(), team, 1)], 7);
            assert_eq!(squads[body(game.world(), team.opponent(), 1)], 0);
        }
    }

    /// Coach edge rule 2: a `CoachEdge` port is evaluated only on the body tick immediately after a
    /// pulse and is false on the other three, so one pulse cannot drive four transitions. `commit`
    /// returns unconditionally, so every scan of `hold` is reported by the cursor that follows it.
    #[test]
    fn a_coach_edge_port_is_false_on_every_tick_but_the_one_after_a_pulse() {
        let logits = [[CLEARING; PORT_COUNT]; 2];
        let mut game = Match::new(MatchConfig::default(), gated(), StandingCoach { logits });
        for tick in 0..8u64 {
            assert_eq!(game.world().tick(), tick);
            game.tick();
            assert_eq!(
                [game.play_node(Team::Zero), game.play_node(Team::One)],
                [usize::from(tick % 4 == 1); 2],
                "scan at tick {tick}"
            );
        }
    }

    /// The proposal writes every ball trigger with an inclusive comparison — `≥` for `BallPast`
    /// and `BallAloft`, `≤` for `BallBehind` — so a ball resting exactly on the operand fires the
    /// port. A strict comparison would differ by one raw Q16.16 unit and diverge replay streams.
    #[test]
    fn ball_triggers_fire_inclusively_at_their_exact_operands() {
        let state = GraphState::default();
        let quiet = [Fx::ZERO; PORT_COUNT];
        let scan = |edges: &str, ball: Vec3Fx, team: Team| {
            let playbook = compiled(&[
                Node {
                    edges,
                    ..Node::default()
                },
                Node {
                    name: "b",
                    edges: "[(to: 1, trigger: Always)]",
                    ..Node::default()
                },
            ]);
            let mut world = World::new(10);
            world.set_position(world.objective_index(), ball);
            next_cursor(&playbook.nodes()[0], 0, team, &state, &world, quiet)
        };

        let past = "[(to: 1, trigger: BallPast(2.0)), (to: 0, trigger: Always)]";
        assert_eq!(scan(past, at(2, 0, 1), Team::Zero), 1, "BallPast at 2.0");
        assert_eq!(
            scan(
                past,
                Vec3Fx::new(Fx::from_raw(131_071), Fx::ZERO, Fx::ONE),
                Team::Zero
            ),
            0,
            "BallPast one raw unit short"
        );
        // The operand is authored in the attacking frame, so team one reads the mirrored ball.
        assert_eq!(scan(past, at(-2, 0, 1), Team::One), 1, "BallPast mirrored");

        let behind = "[(to: 1, trigger: BallBehind(-8.0)), (to: 0, trigger: Always)]";
        assert_eq!(
            scan(behind, at(-8, 0, 1), Team::Zero),
            1,
            "BallBehind at -8.0"
        );
        assert_eq!(
            scan(
                behind,
                Vec3Fx::new(Fx::from_raw(-524_287), Fx::ZERO, Fx::ONE),
                Team::Zero
            ),
            0,
            "BallBehind one raw unit past"
        );

        let aloft = "[(to: 1, trigger: BallAloft(0.5)), (to: 0, trigger: Always)]";
        assert_eq!(
            scan(aloft, Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::HALF), Team::Zero),
            1
        );
        assert_eq!(
            scan(
                aloft,
                Vec3Fx::new(Fx::ZERO, Fx::ZERO, Fx::from_raw(32_767)),
                Team::Zero
            ),
            0
        );
    }

    /// `Possession` is a half-open window: relation holds while the touch is younger than
    /// `POSSESSION_TICKS` and lapses to `Neutral` exactly at it, `Neutral` also covering the
    /// empty window before any touch.
    #[test]
    fn possession_lapses_to_neutral_exactly_at_the_window_and_covers_an_empty_one() {
        let touched = 10;
        let state = GraphState {
            entered: [0; 2],
            touched,
            toucher: Some(Team::Zero),
        };
        let oldest_inside = touched + POSSESSION_TICKS - 1;
        assert_eq!(
            state.possession(Team::Zero, oldest_inside),
            Relation::Teammate
        );
        assert_eq!(
            state.possession(Team::One, oldest_inside),
            Relation::Opponent
        );
        assert_eq!(
            state.possession(Team::Zero, touched + POSSESSION_TICKS),
            Relation::Neutral
        );
        assert_eq!(
            GraphState::default().possession(Team::One, 500),
            Relation::Neutral
        );
    }

    #[test]
    fn a_lane_that_only_meets_the_gate_does_not_clear_it() {
        let logits = [[ratio(1, 4); PORT_COUNT]; 2];
        let mut game = Match::new(MatchConfig::default(), gated(), StandingCoach { logits });
        for _ in 0..4 {
            game.tick();
            assert_eq!(
                [game.play_node(Team::Zero), game.play_node(Team::One)],
                [0, 0],
                "a logit must exceed the gate, not meet it"
            );
        }
    }
}
