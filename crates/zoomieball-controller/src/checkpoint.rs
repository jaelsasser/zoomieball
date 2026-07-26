//! The ZBCT checkpoint envelope: the Zoomieball-local roster ABI header and the backend-local
//! transients sibling Zoomie knows nothing about, wrapped around one length-prefixed `zoomie-wire`
//! [`LiveCheckpoint`] that owns the three populations, their specs, configs, learning rules, and
//! capability manifests.
//!
//! ## Why the population payload is no longer this crate's format
//!
//! The bespoke codec this replaced wrote raw `i32` vectors per member and validated nothing beyond
//! vector length: a checkpoint written against different arithmetic decoded cleanly and desynced
//! silently afterwards. [`SparseArchitecture`] recomputes the expected `CapabilityManifest` from
//! the decoded spec/config/rule and rejects a mismatch, so a capability divergence now surfaces as
//! a decode error at the boundary instead of as a trajectory that quietly stops matching. The rule
//! comes back from the checkpoint rather than from the live pool for the same reason: the
//! exploration seed is part of the learned team's identity, and a resume that kept the local rule
//! would not be the bit-identical continuation the ABI promises. Those dials fold into the
//! learning witness (`witness.rs`), so which rule survived a restore is observable rather than a
//! matter of trust — `checkpoint_restore_adopts_the_checkpoint_rule_rather_than_the_local_one` is
//! what catches the semantics silently reverting.
//!
//! ## Framing
//!
//! [`decode_live`] rejects trailing bytes, so it has to be handed an exact slice — hence the `u32`
//! length prefix rather than letting the payload run to the end of the envelope, which the
//! transient tail already occupies. The tail is fixed width, so a restore knows the exact total
//! length the envelope must have and rejects both a truncation and a trailing byte.
//!
//! | offset | bytes | field |
//! |---:|---:|---|
//! | 0 | 4 | `ZBCT` |
//! | 4 | 18 | [`CheckpointHeader`] |
//! | 22 | 4 | little-endian `u32` wire payload length |
//! | 26 | *n* | `zoomie-wire` `ZNETLIVE` pack (three populations plus the resume cursor) |
//! | 26 + *n* | 592 | per team: 8 squad mailboxes, 8 edge logits, 2 team rewards |
//! | 618 + *n* | 24 | body-pulse, coach-pulse, and learning-pass counters |
//!
//! No migration reader exists and none is planned: an envelope that does not decode is rejected,
//! never upgraded.

use zoomie_pop::{Population, SparseCtrnn};
use zoomie_wire::{
    ArchitectureId, CheckpointCursor, LiveCheckpoint, SparseArchitecture, WireLimits, decode_live,
    encode_live,
};
use zoomieball_core::controller::{CheckpointError, CheckpointHeader};

/// Local checkpoint format tag (`ZBCT`); `ControllerBackend::restore` rejects any payload that
/// does not open with it before touching the roster ABI header.
pub(crate) const CHECKPOINT_MAGIC: &[u8; 4] = b"ZBCT";

/// First envelope byte after the magic and the roster ABI header.
pub(crate) const PAYLOAD_OFFSET: usize = 22;

/// The fielder pool (`reservoir_64_48_8`, 18 or 198 members). These three identities are persisted
/// state: a reshuffle silently reinterprets every checkpoint ever written, so they are append-only
/// and ordered to match the witness fold's registration order.
const FIELDER_ARCHITECTURE: ArchitectureId = ArchitectureId::from_raw(0);
/// The goalie pool (`reservoir_96_64_8`, 2 members).
const GOALIE_ARCHITECTURE: ArchitectureId = ArchitectureId::from_raw(1);
/// The coach pool (the local 128/96/72 fixture, 2 members).
const COACH_ARCHITECTURE: ArchitectureId = ArchitectureId::from_raw(2);

/// Whole-envelope cap for the wire payload: 32 MiB.
///
/// The 100v100 roster is the sizing case, and within it the 198-member fielder pool.
/// `reservoir_64_48_8` carries `E = 3_968` edges over `D = 56` dynamic nodes, and a member stores
/// `2E + 2` parameter words (weights, anchors, the two exploration-key halves) and `D + E + 1`
/// state words (node states, eligibility, the credit age): `11_963` `i32` words in all, the same
/// total partition the two witnesses fold. Postcard spends at most 5 bytes on a zigzag varint, so
/// 59,815 bytes a member and 11.3 MiB across 198 of them. The two goalies (`reservoir_96_64_8`,
/// 7,936 edges) add at most 0.23 MiB, the two coaches (the local 128/96/72 fixture, 10,752 edges)
/// 0.31 MiB, and the three shared specs, rules, manifests, and per-member identities another
/// ~0.07 MiB: ~11.9 MiB is the ceiling. A fresh 100v100 envelope measures 5.2 MiB, because seeded
/// magnitudes zigzag into three bytes and transients are still zero; the cap is sized for the
/// ceiling, not the measurement, so a fully burned-in roster cannot creep past it. 32 MiB clears
/// that ceiling by 2.6x while staying half the crate default, so a roster that outgrows it fails a
/// `checkpoint` call loudly instead of silently inheriting a bound nobody here chose.
const WIRE_LIMITS: WireLimits = WireLimits::new(32 * 1024 * 1024);

/// Append the roster ABI header verbatim, one field at a time in declaration order.
pub(crate) fn write_header(output: &mut Vec<u8>, header: CheckpointHeader) {
    output.extend_from_slice(&header.lane_abi.to_le_bytes());
    output.extend_from_slice(&header.physics_abi.to_le_bytes());
    output.extend_from_slice(&header.reward_abi.to_le_bytes());
    output.extend_from_slice(&header.schedule_abi.to_le_bytes());
    output.extend_from_slice(&header.active_per_team.to_le_bytes());
}

/// Append the length-prefixed `zoomie-wire` live payload capturing `pools` in persisted
/// architecture order (fielders, goalies, coaches) at `next_tick`.
pub(crate) fn write_payload(
    output: &mut Vec<u8>,
    next_tick: u64,
    pools: [&Population<SparseCtrnn>; 3],
) {
    let checkpoint = LiveCheckpoint::capture(
        CheckpointCursor::from_next_tick(next_tick),
        &[
            (FIELDER_ARCHITECTURE, pools[0]),
            (GOALIE_ARCHITECTURE, pools[1]),
            (COACH_ARCHITECTURE, pools[2]),
        ],
    )
    .expect("every role pool arms a learning rule over an ascending validated roster");
    let payload = encode_live(&checkpoint, WIRE_LIMITS)
        .expect("the 100v100 roster fits the declared wire cap");
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("a capped payload length fits u32")
            .to_le_bytes(),
    );
    output.extend_from_slice(&payload);
}

/// Decode the length-prefixed live payload at the envelope cursor, handing [`decode_live`] the
/// exact slice it demands.
pub(crate) fn read_payload(reader: &mut Reader<'_>) -> Result<LiveCheckpoint, CheckpointError> {
    let length = usize::try_from(reader.u32()?).map_err(|_| CheckpointError::Malformed)?;
    let payload = reader.bytes(length)?;
    decode_live(payload, WIRE_LIMITS).map_err(|error| CheckpointError::Payload(error.to_string()))
}

/// Rebuild the three role populations from a decoded checkpoint, rejecting any architecture whose
/// identity, topology, bounds, or roster differs from the pool it would replace.
///
/// Nothing is mutated here: the caller receives three complete populations and swaps them in, so a
/// rejection anywhere leaves the backend exactly as it was.
pub(crate) fn restore_pools(
    checkpoint: &LiveCheckpoint,
    current: [&Population<SparseCtrnn>; 3],
) -> Result<[Population<SparseCtrnn>; 3], CheckpointError> {
    let architectures: &[SparseArchitecture; 3] = checkpoint
        .architectures()
        .try_into()
        .map_err(|_| payload("checkpoint architecture count differs"))?;
    let [fielders, goalies, coaches] = architectures;
    Ok([
        restore_pool(FIELDER_ARCHITECTURE, fielders, current[0])?,
        restore_pool(GOALIE_ARCHITECTURE, goalies, current[1])?,
        restore_pool(COACH_ARCHITECTURE, coaches, current[2])?,
    ])
}

/// Rebuild one role population, after proving the persisted architecture still describes the pool
/// it is about to replace.
fn restore_pool(
    expected: ArchitectureId,
    architecture: &SparseArchitecture,
    current: &Population<SparseCtrnn>,
) -> Result<Population<SparseCtrnn>, CheckpointError> {
    if architecture.id() != expected {
        return Err(payload("checkpoint architecture identities differ"));
    }
    if architecture.spec() != current.spec() || architecture.config() != *current.config() {
        return Err(payload("checkpoint architecture topology differs"));
    }
    if !architecture
        .members()
        .iter()
        .map(|(id, _)| *id)
        .eq(current.ids().iter().copied())
    {
        return Err(payload("checkpoint controller identities differ"));
    }
    Ok(architecture.restore_population())
}

/// A cursor over a checkpoint byte slice, decoding fixed-width little-endian fields in order.
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    /// Start reading `input` at byte offset `cursor`.
    pub(crate) const fn new(input: &'a [u8], cursor: usize) -> Self {
        Self { input, cursor }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], CheckpointError> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| CheckpointError::Malformed)
    }

    /// Borrow the next `length` bytes without copying them.
    pub(crate) fn bytes(&mut self, length: usize) -> Result<&'a [u8], CheckpointError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(CheckpointError::Malformed)?;
        let bytes = self
            .input
            .get(self.cursor..end)
            .ok_or(CheckpointError::Malformed)?;
        self.cursor = end;
        Ok(bytes)
    }

    /// Decode the next little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32, CheckpointError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    /// Decode the next little-endian `i32`.
    pub(crate) fn i32(&mut self) -> Result<i32, CheckpointError> {
        Ok(i32::from_le_bytes(self.take()?))
    }

    /// Decode the next little-endian `u64`.
    pub(crate) fn u64(&mut self) -> Result<u64, CheckpointError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    /// Whether every byte of the input has been consumed.
    pub(crate) fn finished(&self) -> bool {
        self.cursor == self.input.len()
    }
}

/// One backend-specific rejection reason, spelled the way `CheckpointError` wants it.
fn payload(message: &str) -> CheckpointError {
    CheckpointError::Payload(message.to_owned())
}

#[cfg(test)]
mod tests {
    use zoomieball_core::controller::ControllerBackend;
    use zoomieball_core::world::Team;
    use zoomieball_core::{Match, MatchConfig};

    use super::*;
    use crate::backend::ZoomieBackend;
    use crate::fixture::{mutate_member, playbook, rearm};
    use crate::pool::net_id;

    /// A match `ticks` in and the checkpoint bytes taken from it.
    fn captured(seed: u64, ticks: usize) -> (Match<ZoomieBackend>, Vec<u8>) {
        let mut game = Match::new(
            MatchConfig::default(),
            playbook(),
            ZoomieBackend::new(10, seed),
        );
        for _ in 0..ticks {
            game.tick();
        }
        let mut bytes = Vec::new();
        game.controller().checkpoint(&mut bytes);
        (game, bytes)
    }

    /// The envelope's payload length prefix, read back where a framing test needs to corrupt it.
    fn payload_length(bytes: &[u8]) -> usize {
        Reader::new(bytes, PAYLOAD_OFFSET)
            .u32()
            .expect("a captured envelope carries its length prefix") as usize
    }

    #[test]
    fn checkpoint_round_trip_restores_all_population_witnesses() {
        let (mut game, bytes) = captured(11, 4);
        let expected = (
            game.controller().controller_hash(),
            game.controller().learning_hash(),
        );
        for _ in 0..3 {
            game.tick();
        }
        game.controller_mut().restore(&bytes).unwrap();
        assert_eq!(
            (
                game.controller().controller_hash(),
                game.controller().learning_hash(),
            ),
            expected
        );
    }

    #[test]
    fn checkpoint_abi_mismatch_fails_before_mutation() {
        let mut controller = ZoomieBackend::new(10, 13);
        let original = controller.controller_hash();
        let mut bytes = Vec::new();
        controller.checkpoint(&mut bytes);
        bytes[4] ^= 1;
        assert!(matches!(
            controller.restore(&bytes),
            Err(CheckpointError::AbiMismatch { .. })
        ));
        assert_eq!(controller.controller_hash(), original);
    }

    /// Eligibility and the credit age are learning-fold words the bespoke codec carried by hand;
    /// the wire payload has to carry them just as exactly. Five ticks lands the capture one step
    /// past the learn pass that consumed them, so the credit under test is live, not cleared.
    #[test]
    fn checkpoint_captured_mid_learning_restores_eligibility_and_credit_age_exactly() {
        let (game, bytes) = captured(23, 5);
        let id = net_id(Team::Zero, 1);
        let expected = game
            .controller()
            .fielders
            .pop
            .extract(id)
            .expect("the fielder identity is resident");
        assert!(
            expected.eligibility.iter().any(|&word| word != 0),
            "the fixture must carry accrued eligibility"
        );
        assert_ne!(
            expected.credit_age, 0,
            "the fixture must carry a live credit age"
        );

        let mut fresh = ZoomieBackend::new(10, 23);
        fresh.restore(&bytes).unwrap();

        assert_eq!(
            fresh.fielders.pop.extract(id).expect("restored identity"),
            expected
        );
    }

    /// The resume cursor and the pulse counters are in neither witness by design, so the
    /// witness-comparing round trip above cannot cover them. Seven ticks gives all four counters
    /// distinct values, so a transposed assignment fails here too, not just a dropped one.
    #[test]
    fn checkpoint_round_trip_restores_the_resume_cursor_and_the_pulse_counters() {
        let (game, bytes) = captured(47, 7);
        let expected = (
            game.controller().next_tick,
            game.controller().body_pulses(),
            game.controller().coach_pulses(),
            game.controller().learn_passes,
        );
        assert_eq!(
            expected,
            (7, 7, 2, 1),
            "the fixture must resume off zero with four distinguishable counters"
        );

        let mut fresh = ZoomieBackend::new(10, 47);
        fresh.restore(&bytes).unwrap();

        assert_eq!(
            (
                fresh.next_tick,
                fresh.body_pulses(),
                fresh.coach_pulses(),
                fresh.learn_passes,
            ),
            expected
        );
    }

    /// The sentence the wire migration is staked on: the rule comes back from the checkpoint, not
    /// from the pool being restored into. Only a backend rearmed with foreign dials can tell the
    /// two apart — its spec, config, and roster still match, so restore accepts it, and the
    /// learning witness (which folds the dials) is what reports which rule survived.
    #[test]
    fn checkpoint_restore_adopts_the_checkpoint_rule_rather_than_the_local_one() {
        let mut source = ZoomieBackend::new(10, 43);
        rearm(&mut source.fielders.pop, 0x5EED_0004);
        let mut bytes = Vec::new();
        source.checkpoint(&mut bytes);

        let mut local = ZoomieBackend::new(10, 43);
        assert_ne!(
            local.learning_hash(),
            source.learning_hash(),
            "the fixture's dials must differ from the dials being restored into"
        );

        local.restore(&bytes).unwrap();

        assert_eq!(local.learning_hash(), source.learning_hash());
        assert_eq!(local.controller_hash(), source.controller_hash());
    }

    /// A wire-level corruption is the payload's failure, not the envelope's, and must never reach
    /// a panic on the way to saying so.
    #[test]
    fn checkpoint_a_corrupted_wire_payload_rejects_as_a_payload_error() {
        let (mut game, mut bytes) = captured(29, 4);
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        let middle = PAYLOAD_OFFSET + 4 + payload_length(&bytes) / 2;
        bytes[middle] ^= 1;

        assert!(matches!(
            game.controller_mut().restore(&bytes),
            Err(CheckpointError::Payload(_))
        ));
        assert_eq!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    /// `decode_live` rejects trailing bytes, so the envelope's length prefix is what keeps the
    /// wire slice exact — truncation, trailing bytes, and a lying prefix all have to bounce.
    #[test]
    fn checkpoint_framing_rejects_truncation_trailing_bytes_and_a_lying_length_prefix() {
        let (mut game, bytes) = captured(31, 4);

        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(game.controller_mut().restore(&truncated).is_err());

        let mut extended = bytes.clone();
        extended.push(0);
        assert_eq!(
            game.controller_mut().restore(&extended),
            Err(CheckpointError::Malformed)
        );

        let mut lying = bytes.clone();
        let inflated = u32::try_from(payload_length(&bytes) + 1).unwrap();
        lying[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4].copy_from_slice(&inflated.to_le_bytes());
        assert!(game.controller_mut().restore(&lying).is_err());

        game.controller_mut()
            .restore(&bytes)
            .expect("the unmangled envelope still restores");
    }

    /// Restore validates and builds everything before it assigns anything, so a rejection is not a
    /// half-swapped backend.
    #[test]
    fn checkpoint_a_rejected_restore_leaves_both_witnesses_unmoved() {
        let (mut game, bytes) = captured(37, 4);
        for _ in 0..3 {
            game.tick();
        }
        mutate_member(
            &mut game.controller_mut().fielders.pop,
            net_id(Team::Zero, 1),
            |member| member.weights[0] ^= 1,
        );
        let controller = game.controller().controller_hash();
        let learning = game.controller().learning_hash();

        let mut rejected = bytes;
        let last = rejected.len() - 1;
        rejected[last] ^= 1;
        rejected.push(0);

        assert!(game.controller_mut().restore(&rejected).is_err());
        assert_eq!(game.controller().controller_hash(), controller);
        assert_eq!(game.controller().learning_hash(), learning);
    }

    /// The 100v100 arithmetic on [`WIRE_LIMITS`] is a claim about a roster this backend can
    /// actually build, so build it.
    #[test]
    fn checkpoint_the_full_roster_envelope_fits_the_declared_wire_cap() {
        let controller = ZoomieBackend::new(100, 5);
        let mut bytes = Vec::new();
        controller.checkpoint(&mut bytes);
        assert!(
            bytes.len() < WIRE_LIMITS.max_bytes(),
            "a 100v100 envelope of {} bytes does not clear the {} byte cap",
            bytes.len(),
            WIRE_LIMITS.max_bytes()
        );
    }
}
