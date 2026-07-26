//! The two layered witnesses [`ZoomieBackend`] publishes every tick, and the fold order that makes
//! them comparable across runs.
//!
//! `DESIGN.md`'s contract is that each component witness folds its own layer and no other, so a
//! divergence localizes to the single word that moved. Sibling Zoomie supplies the population half
//! already split, and the split is a total partition of every stored member word:
//! `Population::<SparseCtrnn>::inference_pair` folds live weights and integrated node states,
//! `learning_pair` folds each pool's rule dials, then match-boundary anchors, the exploration key,
//! per-edge eligibility, and the credit age. That partition covers everything the *pools* own and
//! nothing else, so this module owns the rest: it assigns the backend-local transients — the
//! trajectory-determining state that lives outside any pool — to the matching side, and pins the
//! order the two folds run in.
//!
//! ## Fold order is normative
//!
//! Both witnesses fold the three pools in registration order (fielders, goalies, coaches) through
//! `world_checksum` and absorb that word, then finish with their backend-local transients in one
//! fixed ascending traversal: team-major, and within a team squad-then-lane. FNV-1a is not
//! associative, so a reordered fold is a *different witness*, not the same value computed another
//! way — reordering this file is an ABI break that needs a version bump, never an in-place edit.
//!
//! ## Learning-owned state that inference reads folds on the learning side
//!
//! Two spans are owned by the learning layer and read back by inference, and both land on
//! [`learning_witness`]:
//!
//! - `team_rewards`, which `learn` writes and the next coach pulse encodes into coach input lanes
//!   120 and 121. This module folds it.
//! - each pool's `ExploratoryHebbRule` dials, which an armed `step` draws its node perturbation
//!   from (`exploration_seed` seeds the draw, `perturbation` sets its magnitude) — so they steer
//!   the integrated node states the *inference* witness folds, one step later. Sibling Zoomie's
//!   `learning_pair` folds them, so this module inherits them rather than folding them again;
//!   `checksum_a_rearmed_rule_dial_moves_the_learning_witness_alone` pins that inheritance from
//!   this side, so a witness that stopped carrying them upstream fails here.
//!
//! The consequence a caller has to carry: two backends agreeing on [`controller_witness`] are
//! *not* thereby agreeing on what they will do next. Compare one witness to ask *what moved*;
//! compare both to ask *will these two step alike*.
//!
//! ## An armed step writes the learning layer, so a plain tick moves both witnesses
//!
//! All three role pools arm `ExploratoryHebbRule`, which puts every one of them on sibling Zoomie's
//! Plastic profile: an ordinary `step` draws its node perturbation from the exploration key,
//! accrues the perturbed row's eligibility, and bumps the credit age — three learning-fold spans
//! written inside the step, before any learn pass runs. An inference-only tick therefore moves the
//! learning witness too, and "no learn pass ran" never licenses expecting it to stand still. What
//! localization still buys is the claim it was always making: one stored word that moves moves
//! exactly one witness. The converse — one witness standing still proving a whole layer stood still
//! — was only ever true of the defective fold this file replaced, which reached no population
//! learning word at all.
//!
//! ## The pulse counters fold into neither
//!
//! `body_pulses`, `coach_pulses`, and the stored resume tick are schedule position, not layer
//! state. Nothing in the backend reads them — the authoritative tick arrives on every `ActRequest`
//! — so two backends differing only there step alike forever, and omitting them cannot let two
//! genuinely different states collide. Folding them would instead move a witness on the mere
//! passage of time, manufacturing a difference between states that agree. They ride the checkpoint,
//! where an exact resume needs them, and no witness.
//!
//! `learn_passes` is on the learning side for the opposite reason: a pass that admits no credit can
//! leave every stored learning word exactly where it was, so the counter is the one word that
//! separates "no pass ran" from "a pass ran and moved nothing". No such gap exists on the inference
//! side, where every pulse integrates node state and the inference fold already witnesses it.

use zoomie_pop::world_checksum;
use zoomieball_core::hash::{OFFSET_BASIS, fold_i32, fold_u64};

use crate::backend::ZoomieBackend;

/// Fold the inference layer: the three pools' live weights and node states in registration order,
/// then the coach publications the next body pulse reads.
///
/// Agreement here is agreement on what the controller *is*. It is not agreement on what the
/// controller will next see: `team_rewards` reaches coach input lanes 120 and 121 and folds into
/// [`learning_witness`], so next-pulse parity needs both witnesses.
pub(crate) fn controller_witness(backend: &ZoomieBackend) -> u64 {
    let hash = fold_u64(
        OFFSET_BASIS,
        world_checksum(&[
            backend.fielders.pop.inference_pair(),
            backend.goalies.pop.inference_pair(),
            backend.coaches.pop.inference_pair(),
        ]),
    );
    let hash = backend
        .mailboxes
        .into_iter()
        .flatten()
        .flatten()
        .fold(hash, fold_i32);
    backend
        .edge_logits
        .into_iter()
        .flatten()
        .fold(hash, fold_i32)
}

/// Fold the learning layer: the three pools' rule dials, anchors, exploration keys, eligibility,
/// and credit ages in registration order, then the accumulated team rewards and the learning-pass
/// counter.
///
/// This witness carries `team_rewards` and — through sibling Zoomie's `learning_pair` — the rule
/// dials, both of which inference reads back: the first through the coach input lanes, the second
/// through the perturbation an armed step adds to its node states. See the module doc for why that
/// allocation is deliberate and what it costs a caller.
pub(crate) fn learning_witness(backend: &ZoomieBackend) -> u64 {
    let hash = fold_u64(
        OFFSET_BASIS,
        world_checksum(&[
            backend.fielders.pop.learning_pair(),
            backend.goalies.pop.learning_pair(),
            backend.coaches.pop.learning_pair(),
        ]),
    );
    let hash = backend
        .team_rewards
        .into_iter()
        .flatten()
        .fold(hash, fold_i32);
    fold_u64(hash, backend.learn_passes)
}

#[cfg(test)]
mod tests {
    use zoomie_core::NetId;
    use zoomieball_core::controller::{
        ActRequest, ControllerBackend, MotorCommandBatch, RewardBatch,
    };
    use zoomieball_core::perception::{ObservationBatch, SpatialIndex};
    use zoomieball_core::physics::PhysicsConfig;
    use zoomieball_core::playbook::OracleIntentBatch;
    use zoomieball_core::world::{Team, World};
    use zoomieball_core::{Match, MatchConfig};

    use super::*;
    use crate::fixture::{allocations, mutate_member, playbook, rearm};
    use crate::pool::net_id;

    /// A match four ticks in, so every pool carries integrated state, the coach mailboxes are
    /// populated, and one learning pass has already run before any witness is read.
    fn warm_match(seed: u64) -> Match<ZoomieBackend> {
        let mut game = Match::new(
            MatchConfig::default(),
            playbook(),
            ZoomieBackend::new(10, seed),
        );
        for _ in 0..4 {
            game.tick();
        }
        game
    }

    /// The first fielder identity, the member every stored-word edit below lands on.
    fn fielder() -> NetId {
        net_id(Team::Zero, 1)
    }

    #[test]
    fn checksum_a_mutated_live_weight_moves_the_controller_witness_alone() {
        let mut game = warm_match(31);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.weights[0] ^= 1;
            },
        );

        assert_ne!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    #[test]
    fn checksum_a_mutated_eligibility_word_moves_the_learning_witness_alone() {
        let mut game = warm_match(37);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.eligibility[0] ^= 1;
            },
        );

        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.controller().controller_hash(), controller);
    }

    #[test]
    fn checksum_a_mutated_anchor_word_moves_the_learning_witness_alone() {
        let mut game = warm_match(41);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.reference_weights[0] ^= 1;
            },
        );

        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.controller().controller_hash(), controller);
    }

    #[test]
    fn checksum_a_mutated_credit_age_word_moves_the_learning_witness_alone() {
        let mut game = warm_match(43);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.credit_age ^= 1;
            },
        );

        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.controller().controller_hash(), controller);
    }

    /// The dials are pool-level rather than stored member words, and the manifest folds only
    /// `(name, version)` entries, so no population witness and no manifest hash reaches them.
    #[test]
    fn checksum_a_rearmed_rule_dial_moves_the_learning_witness_alone() {
        let mut game = warm_match(67);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        rearm(&mut game.controller_mut().fielders.pop, 0x5EED_0001);

        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.controller().controller_hash(), controller);
    }

    /// The property the whole rule-dial fold exists for. Two backends whose stored words are
    /// bit-identical and whose exploration seeds differ take different trajectories from the very
    /// next step, because an armed step draws its node perturbation from those dials. Before the
    /// fold reached them both witnesses agreed here and the divergence arrived unannounced; the
    /// pinned property is that the learning witness separates them *first*, while they still
    /// agree on every stored word.
    #[test]
    fn checksum_rule_dials_separate_the_learning_witness_before_they_separate_the_trajectory() {
        let mut divergent = ZoomieBackend::new(10, 71);
        let mut baseline = ZoomieBackend::new(10, 71);
        rearm(&mut divergent.fielders.pop, 0x5EED_0002);
        rearm(&mut baseline.fielders.pop, 0x5EED_0003);

        assert_eq!(
            divergent.controller_hash(),
            baseline.controller_hash(),
            "the fixture must hold every stored word identical"
        );
        assert_ne!(
            divergent.learning_hash(),
            baseline.learning_hash(),
            "the learning witness must announce the dials that are about to diverge them"
        );

        let mut divergent = Match::new(MatchConfig::default(), playbook(), divergent);
        let mut baseline = Match::new(MatchConfig::default(), playbook(), baseline);
        divergent.tick();
        baseline.tick();

        assert_ne!(
            divergent.controller().controller_hash(),
            baseline.controller().controller_hash(),
            "one identical step must be enough to diverge them"
        );
    }

    #[test]
    fn checksum_a_mutated_mailbox_lane_moves_the_controller_witness_alone() {
        let mut game = warm_match(47);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        game.controller_mut().mailboxes[Team::One.index()][3][5] ^= 1;

        assert_ne!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    #[test]
    fn checksum_a_mutated_edge_logit_moves_the_controller_witness_alone() {
        let mut game = warm_match(53);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        game.controller_mut().edge_logits[Team::Zero.index()][7] ^= 1;

        assert_ne!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    #[test]
    fn checksum_a_mutated_team_reward_moves_the_learning_witness_alone() {
        let mut game = warm_match(59);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        game.controller_mut().team_rewards[Team::One.index()][0] ^= 1;

        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.controller().controller_hash(), controller);
    }

    /// Schedule position is not layer state: nothing reads the counters, so folding them would move
    /// a witness for no reason other than that time passed.
    #[test]
    fn checksum_the_pulse_counters_and_resume_tick_move_neither_witness() {
        let mut game = warm_match(61);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        let backend = game.controller_mut();
        backend.body_pulses += 7;
        backend.coach_pulses += 3;
        backend.next_tick += 11;

        assert_eq!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    /// Both edits are invisible to physics, so the physics hash cannot be either witness's source;
    /// and the diagnostic fold absorbs both components, so it stands in for neither.
    #[test]
    fn checksum_neither_witness_is_derivable_from_the_physics_hash_or_the_pipeline_fold() {
        let mut game = Match::new(
            MatchConfig::default(),
            playbook(),
            ZoomieBackend::new(10, 67),
        );
        let fold = game.tick();
        let physics = game.world().physics_hash();
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.weights[0] ^= 1;
            },
        );
        assert_ne!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
        assert_eq!(game.world().physics_hash(), physics);

        mutate_member(
            &mut game.controller_mut().fielders.pop,
            fielder(),
            |member| {
                member.weights[0] ^= 1;
                member.eligibility[0] ^= 1;
            },
        );
        assert_eq!(game.controller().controller_hash(), controller);
        assert_ne!(game.controller().learning_hash(), learning);
        assert_eq!(game.world().physics_hash(), physics);

        assert_ne!(fold.controller, fold.pipeline);
        assert_ne!(fold.learning, fold.pipeline);
        assert_ne!(fold.controller, u64::from(fold.physics));
        assert_ne!(fold.learning, u64::from(fold.physics));
    }

    /// Sibling Zoomie's Plastic read-set caveat, made concrete. All three pools arm a rule, so an
    /// armed `step` draws its node perturbation from the exploration key, accrues the perturbed
    /// row's eligibility, and bumps the credit age — three learning-fold spans written inside the
    /// step, before any learn pass runs. An inference-only tick therefore moves *both* witnesses,
    /// and localization survives as the mutation tests above state it (one word moves one witness),
    /// never as "no learn pass ran, so the learning witness stood still".
    #[test]
    fn an_inference_only_tick_moves_both_witnesses_because_an_armed_step_accrues_credit() {
        let mut game = Match::new(
            MatchConfig::default(),
            playbook(),
            ZoomieBackend::new(10, 19),
        );
        game.tick();
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();
        let before = game
            .controller()
            .fielders
            .pop
            .extract(fielder())
            .expect("the fielder identity is resident");

        game.tick();

        let after = game
            .controller()
            .fielders
            .pop
            .extract(fielder())
            .expect("the fielder identity is resident");
        assert_eq!(
            game.controller().learn_passes,
            0,
            "the fixture must not reach a learning pass"
        );
        assert_eq!(
            game.controller().team_rewards,
            [[0; 2]; 2],
            "no learning pass ran, so no team reward was written"
        );
        assert_ne!(
            game.controller().controller_hash(),
            controller,
            "an inference pulse must move the controller witness"
        );
        assert_ne!(
            game.controller().learning_hash(),
            learning,
            "an armed step is itself a learning-layer write"
        );
        assert_ne!(
            (&after.eligibility, after.credit_age),
            (&before.eligibility, before.credit_age),
            "the step's own eligibility and credit-age writes are what moved it"
        );
    }

    /// Every hot-loop buffer is caller-owned, so a warm pulse must reach the heap zero times. The
    /// perception frame is rebuilt outside the measured window because building it is not a pulse.
    #[test]
    fn checksum_neither_act_nor_learn_allocates_on_the_hot_path() {
        let mut world = World::new(10);
        let bodies = world.view().len();
        let playbook = playbook();
        let physics = PhysicsConfig::default();
        let mut intents = OracleIntentBatch::with_len(bodies);
        let mut spatial = SpatialIndex::new(bodies);
        let mut observations = ObservationBatch::with_capacity(bodies);
        let mut commands = MotorCommandBatch::with_len(bodies);
        let rewards = RewardBatch::with_len(bodies);
        let mut backend = ZoomieBackend::new(10, 71);

        playbook.resolve(0, &mut world, &mut intents);
        spatial.rebuild(world.view());
        observations.build(world.view(), &intents, &physics.arena, &spatial);
        let request = ActRequest {
            tick: 8,
            world: world.view(),
            observations: &observations,
            intents: &intents,
            play_node: 0,
            enabled_edges: 1,
            coach_due: true,
        };

        // A zero count is only evidence once the counter is known to be armed.
        assert!(
            allocations(|| drop(std::hint::black_box(Vec::<u8>::with_capacity(64)))) > 0,
            "the counting allocator is not installed, so a zero below would prove nothing"
        );

        // One unmeasured round so any first-call capacity growth lands outside the window.
        backend.act(request, &mut commands);
        backend.learn(8, &rewards);
        let measured = allocations(|| {
            backend.act(request, &mut commands);
            backend.learn(8, &rewards);
        });

        assert_eq!(measured, 0, "a warm act/learn pulse reached the heap");
    }
}
