//! Frame-independent witness golden: a fixed short match at a pinned seed and roster, with its
//! two component witnesses and its checkpoint bytes pinned to committed constants.
//!
//! ## What these constants are, and are not, evidence of
//!
//! They are evidence that **this build reproduces this trajectory**: the same seed and roster,
//! driven the same number of ticks through the real [`Match`] driver, still fold to the same
//! `controller_hash`, the same `learning_hash`, and the same checkpoint bytes — and that a restore
//! of those bytes lands back on both witnesses. That is a regression net under the whole
//! encode → step → learn → checkpoint path, and it is the layer that catches an accidental change
//! to arithmetic, fold order, iteration order, or wire framing.
//!
//! They are **not** evidence that any lane means anything. This file asserts no lane index, no
//! perception value, no encoding quantity, and no mapping between a world quantity and the lane
//! that carries it. It cannot: the observation encoding frame is still an open decision, and the
//! full replay goldens that will pin lane semantics are blocked behind it. Committing a lane
//! opinion here would make that decision hostage to a test written before it.
//!
//! The practical consequence for whoever re-baselines this file: **a moved constant means the
//! encoding, arithmetic, or fold changed — it never means this test was wrong.** Find the change,
//! decide whether it was intended, and only then re-run the generator below. In particular, the
//! observation encoding frame decision *will* move these constants when it lands, and that is
//! expected rather than a regression.
//!
//! ## Regenerating
//!
//! Print the current values, confirm the move was intended, then paste:
//!
//! ```sh
//! cargo test -p zoomieball-controller --test witness_golden -- --nocapture golden_report
//! ```
//!
//! ## Native/WASI agreement
//!
//! The fixture below is deliberately identical to `zoomieball-headless 10 60` — same roster, same
//! seed, same `MatchConfig`, same tick count — so the witnesses pinned here are the ones that
//! binary prints, on whichever target it is built for:
//!
//! ```sh
//! cargo run -p zoomieball-headless -- 10 60 --hashes | tail -2
//! cargo build --release -p zoomieball-headless --target wasm32-wasip1
//! node --no-warnings scripts/run-wasi.mjs \
//!     target/wasm32-wasip1/release/zoomieball-headless.wasm 10 60 --hashes | tail -2
//! ```
//!
//! Both streams end with the tick-60 witness line and a `checkpoint bytes=… fold=…` line carrying
//! the same four constants this file commits, so native ≡ wasm32-wasip1 is a byte comparison
//! against the same numbers rather than against a separately maintained expectation.

use zoomieball_controller::ZoomieBackend;
use zoomieball_core::controller::ControllerBackend;
use zoomieball_core::hash::{OFFSET_BASIS, fold_bytes};
use zoomieball_core::{Match, MatchConfig, Playbook};

/// Roster, seed, and tick count of `zoomieball-headless 10 60`.
const ROSTER: usize = 10;
const SEED: u64 = 0x005a_001e_ba11;
const TICKS: u32 = 60;

/// `controller_hash()` after [`TICKS`] ticks of the pinned fixture.
const CONTROLLER_WITNESS: u64 = 0x00ff_c739_f54a_18cc;
/// `learning_hash()` after [`TICKS`] ticks of the pinned fixture.
const LEARNING_WITNESS: u64 = 0x66b3_8050_17e6_d107;
/// Length of the ZBCT envelope captured at that point.
const CHECKPOINT_BYTES: usize = 766_746;
/// FNV-1a fold of that envelope, which pins the payload framing the length alone cannot.
const CHECKPOINT_FOLD: u64 = 0xa912_2dba_dd68_5b7b;

/// The pinned fixture, driven to completion through the real driver so `act` and `learn` both run.
fn played() -> Match<ZoomieBackend> {
    let playbook = Playbook::compile_ron(include_str!("../../../assets/default-playbook.ron"))
        .expect("the shipped playbook compiles");
    let mut game = Match::new(
        MatchConfig {
            active_per_team: ROSTER,
            ..MatchConfig::default()
        },
        playbook,
        ZoomieBackend::new(ROSTER, SEED),
    );
    for _ in 0..TICKS {
        game.tick();
    }
    game
}

/// The captured envelope and its fold, the pair the byte-level constants pin.
fn captured(game: &Match<ZoomieBackend>) -> (Vec<u8>, u64) {
    let mut bytes = Vec::new();
    game.controller().checkpoint(&mut bytes);
    let fold = fold_bytes(OFFSET_BASIS, &bytes);
    (bytes, fold)
}

#[test]
fn golden_the_pinned_match_reproduces_both_component_witnesses() {
    let game = played();

    assert_eq!(
        game.controller().body_pulses(),
        u64::from(TICKS),
        "the fixture must drive every tick through act"
    );
    assert_eq!(
        game.controller().next_tick(),
        u64::from(TICKS),
        "the resume cursor must sit one past the last tick act saw"
    );
    assert_eq!(
        (
            game.controller().controller_hash(),
            game.controller().learning_hash(),
        ),
        (CONTROLLER_WITNESS, LEARNING_WITNESS)
    );
}

#[test]
fn golden_the_pinned_match_reproduces_its_checkpoint_bytes() {
    let (bytes, fold) = captured(&played());

    assert_eq!((bytes.len(), fold), (CHECKPOINT_BYTES, CHECKPOINT_FOLD));
}

/// The witnesses survive the round trip, so the constants above pin a checkpoint that restores to
/// the state it was taken from rather than merely one that decodes.
#[test]
fn golden_restoring_the_pinned_checkpoint_reproduces_both_component_witnesses() {
    let (bytes, _) = captured(&played());

    let mut restored = ZoomieBackend::new(ROSTER, SEED);
    restored
        .restore(&bytes)
        .expect("the pinned envelope decodes");

    assert_eq!(
        (restored.controller_hash(), restored.learning_hash()),
        (CONTROLLER_WITNESS, LEARNING_WITNESS)
    );
    assert_eq!(restored.next_tick(), u64::from(TICKS));
}

/// Print every committed constant in paste-ready form. Ignored by default because it asserts
/// nothing — it exists so a re-baseline never means hand-computing a checksum.
#[test]
#[ignore = "generator for the constants above, not a check"]
fn golden_report() {
    let game = played();
    let (bytes, fold) = captured(&game);

    println!(
        "const CONTROLLER_WITNESS: u64 = {:#018x};",
        game.controller().controller_hash()
    );
    println!(
        "const LEARNING_WITNESS: u64 = {:#018x};",
        game.controller().learning_hash()
    );
    println!("const CHECKPOINT_BYTES: usize = {};", bytes.len());
    println!("const CHECKPOINT_FOLD: u64 = {fold:#018x};");
}
