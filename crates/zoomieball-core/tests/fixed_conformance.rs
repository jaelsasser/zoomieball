//! Public conformance vectors for the normative Q16.16 arithmetic kernels.

use std::panic::{UnwindSafe, catch_unwind};

use zoomieball_core::fixed::{isqrt64, mul64, qdiv, qmul};
use zoomieball_core::{Fx, Vec3Fx};

fn assert_panics(action: impl FnOnce() + UnwindSafe) {
    assert!(catch_unwind(action).is_err());
}

#[test]
fn mul64_is_exact_across_signed_extrema() {
    assert_eq!(mul64(i32::MIN, i32::MIN), 4_611_686_018_427_387_904);
    assert_eq!(mul64(i32::MIN, i32::MAX), -4_611_686_016_279_904_256);
    assert_eq!(mul64(i32::MAX, i32::MAX), 4_611_686_014_132_420_609);
    assert_eq!(mul64(i32::MIN, -1), 2_147_483_648);
}

#[test]
fn qmul_truncates_magnitude_before_restoring_sign() {
    assert_eq!(qmul(98_305, 32_768), 49_152);
    assert_eq!(qmul(-98_305, 32_768), -49_152);
    assert_eq!(qmul(98_305, -32_768), -49_152);
    assert_eq!(qmul(-98_305, -32_768), 49_152);
    assert_eq!(qmul(65_537, 65_537), 65_538);
    assert_eq!(qmul(-65_537, 65_537), -65_538);
    assert_eq!(qmul(1, 65_535), 0);
    assert_eq!(qmul(-1, 65_535), 0);
    assert_eq!(qmul(i32::MIN, 65_536), i32::MIN);
}

#[test]
fn qmul_rejects_an_unrepresentable_result() {
    assert_panics(|| {
        let _ = qmul(i32::MIN, i32::MIN);
    });
}

#[test]
fn qdiv_is_exact_and_truncates_toward_zero() {
    assert_eq!(qdiv(49_152, 32_768), 98_304);
    assert_eq!(qdiv(1, 3), 21_845);
    assert_eq!(qdiv(-1, 3), -21_845);
    assert_eq!(qdiv(1, -3), -21_845);
    assert_eq!(qdiv(-1, -3), 21_845);
    assert_eq!(qdiv(1, i32::MAX), 0);
    assert_eq!(qdiv(-1, i32::MAX), 0);
    assert_eq!(qdiv(i32::MIN, 65_536), i32::MIN);
}

#[test]
fn qdiv_rejects_a_zero_divisor() {
    assert_panics(|| {
        let _ = qdiv(1, 0);
    });
}

#[test]
fn qdiv_rejects_an_unrepresentable_result() {
    assert_panics(|| {
        let _ = qdiv(i32::MIN, -65_536);
    });
}

#[test]
fn isqrt64_returns_the_floor_at_square_boundaries() {
    for value in 0..10_000u64 {
        let root = u64::from(isqrt64(value));
        assert!(root * root <= value);
        assert!((root + 1) * (root + 1) > value);
    }

    assert_eq!(isqrt64(0), 0);
    assert_eq!(isqrt64(1), 1);
    assert_eq!(isqrt64(15), 3);
    assert_eq!(isqrt64(16), 4);
    assert_eq!(isqrt64(17), 4);
    assert_eq!(isqrt64(18_446_744_065_119_617_024), 4_294_967_294);
    assert_eq!(isqrt64(18_446_744_065_119_617_025), u32::MAX);
    assert_eq!(isqrt64(u64::MAX), u32::MAX);
}

#[test]
fn scalar_edges_use_twos_complement_wrapping() {
    assert_eq!((Fx::from_raw(i32::MAX) + Fx::from_raw(1)).raw(), i32::MIN);
    assert_eq!((Fx::from_raw(i32::MIN) - Fx::from_raw(1)).raw(), i32::MAX);
    assert_eq!((-Fx::from_raw(i32::MIN)).raw(), i32::MIN);
    assert_eq!(Fx::from_raw(i32::MIN).abs().raw(), i32::MIN);
}

#[test]
fn integer_construction_accepts_the_representable_endpoints() {
    assert_eq!(Fx::from_i32(-32_768).raw(), i32::MIN);
    assert_eq!(Fx::from_i32(32_767).raw(), 2_147_418_112);
}

#[test]
fn integer_construction_rejects_a_positive_overflow() {
    assert_panics(|| {
        let _ = Fx::from_i32(32_768);
    });
}

#[test]
fn integer_construction_rejects_a_negative_overflow() {
    assert_panics(|| {
        let _ = Fx::from_i32(-32_769);
    });
}

#[test]
fn vector_length_uses_the_unshifted_sum_of_squares() {
    let least_nonzero = Vec3Fx::splat(Fx::from_raw(1));
    assert_eq!(least_nonzero.length().raw(), 1);

    let three_four_five = Vec3Fx::new(Fx::from_raw(196_608), Fx::from_raw(262_144), Fx::ZERO);
    assert_eq!(three_four_five.length().raw(), 327_680);
}

#[test]
fn dot_product_accumulates_fractional_products_before_renormalizing() {
    let least_nonzero = Vec3Fx::new(Fx::from_raw(1), Fx::from_raw(1), Fx::ZERO);
    let halves = Vec3Fx::new(Fx::from_raw(32_768), Fx::from_raw(32_768), Fx::ZERO);

    assert_eq!(least_nonzero.dot(halves).raw(), 1);
}

#[test]
fn vector_normalization_is_exact_in_the_supported_domain() {
    let value = Vec3Fx::new(Fx::from_raw(-196_608), Fx::from_raw(262_144), Fx::ZERO);
    assert_eq!(
        value.normalized(),
        Vec3Fx::new(Fx::from_raw(-39_321), Fx::from_raw(52_428), Fx::ZERO)
    );
    assert_eq!(Vec3Fx::ZERO.normalized(), Vec3Fx::ZERO);
}

#[test]
fn cross_product_cancels_widened_products_before_renormalizing() {
    let left = Vec3Fx::new(Fx::ZERO, Fx::from_raw(i32::MAX), Fx::from_raw(i32::MAX));
    let right = Vec3Fx::new(
        Fx::ZERO,
        Fx::from_raw(2_147_418_111),
        Fx::from_raw(i32::MAX),
    );

    assert_eq!(
        left.cross(right),
        Vec3Fx::new(Fx::from_raw(i32::MAX), Fx::ZERO, Fx::ZERO)
    );
}
