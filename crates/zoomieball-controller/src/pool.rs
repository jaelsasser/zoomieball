//! The per-role slab binding: [`RolePool`] pairs one `Population<SparseCtrnn>` with the
//! `LaneRows` its encode pass fills and its output pass reads, plus the body-index binding that
//! turns a population column back into a world body. Kept apart from `encode/` because it owns
//! construction and identity, not per-tick arithmetic: seeding a pool's members
//! (`RolePool::new`), the `NetId` packing every role shares (`net_id`), the fielder/goalie
//! identity and column derivations (`player_ids`, `player_indices`), and the local 128/96/72
//! coach reservoir fixture (`coach_spec`) that has no sibling-crate spec to borrow from.

use std::collections::BTreeSet;

use zoomie_core::{LaneRows, NetId};
use zoomie_math::fixed::ONE;
use zoomie_math::rng::Seed;
use zoomie_pop::{
    EdgeTag, ExploratoryHebbParams, ExploratoryHebbRule, Population, SparseCtrnn,
    SparseCtrnnConfig, SparseCtrnnMember, SparseCtrnnSpec,
};
use zoomieball_core::world::Team;

const WEIGHT_SCALE: i32 = ONE / 2;

/// One role's population, the lane buffers its encode/decode passes reuse every tick, and the
/// world bodies its columns bind to (empty for the coach pool, which has none).
pub(crate) struct RolePool {
    pub(crate) pop: Population<SparseCtrnn>,
    pub(crate) inputs: LaneRows,
    pub(crate) outputs: LaneRows,
    pub(crate) body_indices: Vec<usize>,
    pub(crate) dt: i32,
}

impl RolePool {
    /// Seed a fresh population from `spec`/`config`, one exploratory-Hebbian member per `ids`
    /// entry in order, and cache the lane buffers its pulses will reuse.
    pub(crate) fn new(
        spec: SparseCtrnnSpec,
        config: SparseCtrnnConfig,
        ids: &[NetId],
        body_indices: Vec<usize>,
        seed: u64,
        dt: i32,
    ) -> Self {
        let params = ExploratoryHebbParams {
            exploration_seed: seed ^ 0x004c_4541_524e,
            ..ExploratoryHebbParams::default()
        };
        let rule =
            ExploratoryHebbRule::new(params, &config).expect("the shipped learning rule is valid");
        let mut pop = Population::new(spec, config, Some(rule));
        let members = ids
            .iter()
            .copied()
            .map(|id| {
                (
                    id,
                    SparseCtrnnMember::seeded(Seed(seed), id, WEIGHT_SCALE, pop.spec(), &config),
                )
            })
            .collect();
        pop.insert_batch(members)
            .expect("role identities and seeded members are valid");
        let inputs = LaneRows::new(pop.input_lanes(), pop.len());
        let outputs = LaneRows::new(pop.output_lanes(), pop.len());
        Self {
            pop,
            inputs,
            outputs,
            body_indices,
            dt,
        }
    }
}

/// `(input, recurrent, output)` geometry for one role's spec.
pub(crate) fn topology(spec: &SparseCtrnnSpec) -> (usize, usize, usize) {
    (
        spec.input_count(),
        spec.recurrent_count(),
        spec.output_count(),
    )
}

/// The local 128/96/72 coach reservoir fixture: no sibling-crate spec matches the timing and
/// topology this backend's coach pool needs, so it is built here from a fixed pseudo-random
/// wiring rather than borrowed.
pub(crate) fn coach_spec() -> SparseCtrnnSpec {
    const INPUTS: usize = 128;
    const RECURRENT: usize = 96;
    const OUTPUTS: usize = 72;
    const FAN_IN: usize = 64;
    let dynamic = RECURRENT + OUTPUTS;
    let mut offsets = Vec::with_capacity(dynamic + 1);
    let mut sources = Vec::with_capacity(dynamic * FAN_IN);
    let mut tags = Vec::with_capacity(dynamic * FAN_IN);
    offsets.push(0);
    for target in 0..dynamic {
        let mut row = BTreeSet::new();
        for slot in 0..32 {
            row.insert((target * 37 + slot * 17) % INPUTS);
            row.insert(INPUTS + (target * 29 + slot * 11) % RECURRENT);
        }
        sources.extend(row);
        tags.resize(
            sources.len(),
            if target < RECURRENT {
                EdgeTag::Reservoir
            } else {
                EdgeTag::Head
            },
        );
        offsets.push(sources.len());
    }
    let leaks = (0..dynamic)
        .map(|target| {
            if target < RECURRENT {
                ONE / 32
            } else {
                ONE / 8
            }
        })
        .collect();
    SparseCtrnnSpec::new(INPUTS, RECURRENT, OUTPUTS, offsets, sources, leaks, tags)
        .expect("the 128/96/72 coach fixture is statically valid")
}

/// Every `(team, local)` identity in `locals` for both teams, team-major.
pub(crate) fn player_ids(active: usize, locals: std::ops::Range<usize>) -> Vec<NetId> {
    let ids: Vec<_> = Team::ALL
        .into_iter()
        .flat_map(|team| locals.clone().map(move |local| net_id(team, local)))
        .collect();
    assert_eq!(ids.len(), 2 * (active - 1));
    ids
}

/// The flat body-array column for every `(team, local)` identity in `locals`, team-major.
pub(crate) fn player_indices(active: usize, locals: std::ops::Range<usize>) -> Vec<usize> {
    Team::ALL
        .into_iter()
        .flat_map(|team| {
            locals
                .clone()
                .map(move |local| team.index() * active + local)
        })
        .collect()
}

/// Pack a team and local roster slot into one stable identity (`team * 101 + local`).
pub(crate) fn net_id(team: Team, local: usize) -> NetId {
    NetId::from_raw(u64::try_from(team.index() * 101 + local).expect("roster identity fits u64"))
}
