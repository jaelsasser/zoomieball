//! Shared sensory primitives the body and coach encoders both build their retinas from: the
//! eight-octant `receptor` index, `inverse_depth` ray weighting, the `body_group`/`coach_group`
//! semantic-tag bucketing each encoder sizes its own retina width by, and the saturating
//! `LaneRows` writers (`clear_column`, `write_signed`) and `i64`-to-lane narrowing (`clamp_i64`)
//! every column write goes through. Kept out of `body.rs`/`coach.rs` because the pending
//! observation-encoding-frame conformance bite (see `../../TODO.md`) rewrites exactly these
//! primitives without touching either encoder's per-population wiring.

use zoomie_core::LaneRows;
use zoomie_math::fixed::ONE;
use zoomieball_core::fixed::{Fx, Vec3Fx};
use zoomieball_core::perception::{RayObservation, Relation, SemanticTag};
use zoomieball_core::world::Role;

/// Zero every lane of one input column ahead of a fresh encode pass.
pub(crate) fn clear_column(rows: &mut LaneRows, column: usize) {
    for lane in 0..rows.lanes() {
        rows.row_mut(lane)[column] = 0;
    }
}

/// Write a signed `[-ONE, ONE]` value into one lane, remapped into the unsigned `[0, ONE]` a
/// squash input expects.
pub(crate) fn write_signed(rows: &mut LaneRows, lane: usize, column: usize, raw: i32) {
    rows.row_mut(lane)[column] = i32::midpoint(raw.clamp(-ONE, ONE), ONE);
}

/// The body/goalie retina's semantic bucket for one observed ray.
pub(crate) fn body_group(tag: SemanticTag) -> usize {
    match (tag.relation, tag.role) {
        (Relation::Neutral, _) => 0,
        (Relation::Teammate, Role::Goalie) => 1,
        (Relation::Teammate, _) => 2,
        (Relation::Opponent, _) => 3,
        (Relation::Arena | Relation::Goal, _) => 4,
    }
}

/// The coach union retina's semantic bucket for one observed ray.
pub(crate) fn coach_group(tag: SemanticTag) -> usize {
    match tag.relation {
        Relation::Neutral => 0,
        Relation::Arena => 1,
        Relation::Goal => 2,
        Relation::Opponent => 3 + usize::from(tag.role == Role::Goalie),
        Relation::Teammate => 5 + usize::from(tag.squad % 5),
    }
}

/// The eight-octant receptor index for a signed direction.
pub(crate) fn receptor(direction: Vec3Fx) -> usize {
    usize::from(direction.x.raw() >= 0) << 2
        | usize::from(direction.z.raw() >= 0) << 1
        | usize::from(direction.y.raw() >= 0)
}

/// `ONE / depth`, saturated to `[0, ONE]`; a ray at or behind the origin reads as maximally near.
pub(crate) fn inverse_depth(depth: Fx) -> i32 {
    if depth.raw() <= 0 {
        return ONE;
    }
    clamp_i64(i64::from(ONE) * i64::from(ONE) / i64::from(depth.raw())).clamp(0, ONE)
}

/// Inverse-depth ray weight, signed by whether the tag reads as friendly or hostile.
pub(crate) fn signed_weight(ray: &RayObservation) -> i32 {
    let weight = inverse_depth(ray.depth);
    match ray.tag.relation {
        Relation::Teammate | Relation::Goal | Relation::Neutral => weight,
        Relation::Opponent | Relation::Arena => -weight,
    }
}

/// The four-way surface/air contact charge code, evenly spaced across `[-ONE, ONE]`.
pub(crate) const fn charge_code(surface: bool, air: bool) -> i32 {
    match (surface, air) {
        (false, false) => -ONE,
        (false, true) => -ONE / 3,
        (true, false) => ONE / 3,
        (true, true) => ONE,
    }
}

/// Narrow a saturating `i64` accumulator to `i32`, clamping instead of wrapping at either bound.
pub(crate) fn clamp_i64(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}
