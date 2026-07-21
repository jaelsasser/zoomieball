//! Q16.16 scalar and three-dimensional vector arithmetic.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Signed Q16.16 value.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Fx(i32);

impl Fx {
    /// Number of fractional bits.
    pub const FRACTION_BITS: u32 = 16;
    /// Raw representation of one.
    pub const ONE_RAW: i32 = 1 << Self::FRACTION_BITS;
    /// Zero.
    pub const ZERO: Self = Self(0);
    /// One.
    pub const ONE: Self = Self(Self::ONE_RAW);
    /// One half.
    pub const HALF: Self = Self(Self::ONE_RAW / 2);

    /// Construct from the exact raw Q16.16 word.
    #[must_use]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Return the exact raw Q16.16 word.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Construct an exactly representable integer.
    #[must_use]
    pub const fn from_i32(value: i32) -> Self {
        Self(value.saturating_mul(Self::ONE_RAW))
    }

    /// Convert at the cosmetic rendering boundary.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Self::ONE_RAW as f32
    }

    /// Absolute value, saturating at the positive bound.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(self.0.saturating_abs())
    }

    /// Clamp between two fixed values.
    #[must_use]
    pub const fn clamp(self, min: Self, max: Self) -> Self {
        if self.0 < min.0 {
            min
        } else if self.0 > max.0 {
            max
        } else {
            self
        }
    }

    /// Return `-1`, `0`, or `1` as Q16.16.
    #[must_use]
    pub const fn signum(self) -> Self {
        Self(self.0.signum().saturating_mul(Self::ONE_RAW))
    }

    /// Widened multiply with one final Q16.16 shift.
    #[must_use]
    pub fn mul_wide(self, rhs: Self) -> Self {
        let product = i64::from(self.0) * i64::from(rhs.0);
        Self(clamp_i64_to_i32(product / i64::from(Self::ONE_RAW)))
    }

    /// Widened divide with the numerator shifted before division.
    #[must_use]
    pub fn div_wide(self, rhs: Self) -> Self {
        assert_ne!(rhs.0, 0, "fixed-point division by zero");
        let numerator = i64::from(self.0) * i64::from(Self::ONE_RAW);
        Self(clamp_i64_to_i32(numerator / i64::from(rhs.0)))
    }

    /// Integer square root of a nonnegative Q16.16 value.
    #[must_use]
    pub fn sqrt(self) -> Self {
        assert!(self.0 >= 0, "fixed-point square root of a negative value");
        let radicand = u128::from(self.0.cast_unsigned()) << Self::FRACTION_BITS;
        Self(i32::try_from(isqrt(radicand)).unwrap_or(i32::MAX))
    }
}

impl fmt::Debug for Fx {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Fx({:.4})", self.to_f32())
    }
}

impl Add for Fx {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl AddAssign for Fx {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for Fx {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl SubAssign for Fx {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul for Fx {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        self.mul_wide(rhs)
    }
}

impl Div for Fx {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        self.div_wide(rhs)
    }
}

impl Neg for Fx {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(self.0.saturating_neg())
    }
}

impl From<i32> for Fx {
    fn from(value: i32) -> Self {
        Self::from_i32(value)
    }
}

/// Three Q16.16 coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Vec3Fx {
    /// X coordinate.
    pub x: Fx,
    /// Y coordinate.
    pub y: Fx,
    /// Z coordinate.
    pub z: Fx,
}

impl Vec3Fx {
    /// Zero vector.
    pub const ZERO: Self = Self::splat(Fx::ZERO);
    /// Positive X unit vector.
    pub const X: Self = Self::new(Fx::ONE, Fx::ZERO, Fx::ZERO);
    /// Positive Y unit vector.
    pub const Y: Self = Self::new(Fx::ZERO, Fx::ONE, Fx::ZERO);
    /// Positive Z unit vector.
    pub const Z: Self = Self::new(Fx::ZERO, Fx::ZERO, Fx::ONE);

    /// Construct a vector.
    #[must_use]
    pub const fn new(x: Fx, y: Fx, z: Fx) -> Self {
        Self { x, y, z }
    }

    /// Construct a vector with equal coordinates.
    #[must_use]
    pub const fn splat(value: Fx) -> Self {
        Self::new(value, value, value)
    }

    /// Widened dot product with one final clamp.
    #[must_use]
    pub fn dot(self, rhs: Self) -> Fx {
        let sum = i64::from(self.x.raw()) * i64::from(rhs.x.raw())
            + i64::from(self.y.raw()) * i64::from(rhs.y.raw())
            + i64::from(self.z.raw()) * i64::from(rhs.z.raw());
        Fx::from_raw(clamp_i64_to_i32(sum / i64::from(Fx::ONE_RAW)))
    }

    /// Cross product.
    #[must_use]
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// Squared length in Q16.16.
    #[must_use]
    pub fn length_squared(self) -> Fx {
        self.dot(self)
    }

    /// Length in Q16.16.
    #[must_use]
    pub fn length(self) -> Fx {
        self.length_squared().sqrt()
    }

    /// Unit vector, or zero when the input has no direction.
    #[must_use]
    pub fn normalized(self) -> Self {
        let sum = i128::from(self.x.raw()) * i128::from(self.x.raw())
            + i128::from(self.y.raw()) * i128::from(self.y.raw())
            + i128::from(self.z.raw()) * i128::from(self.z.raw());
        if sum == 0 {
            return Self::ZERO;
        }
        let length = i64::try_from(isqrt(sum.cast_unsigned())).unwrap_or(i64::MAX);
        Self::new(
            Fx::from_raw(clamp_i64_to_i32(
                i64::from(self.x.raw()) * i64::from(Fx::ONE_RAW) / length,
            )),
            Fx::from_raw(clamp_i64_to_i32(
                i64::from(self.y.raw()) * i64::from(Fx::ONE_RAW) / length,
            )),
            Fx::from_raw(clamp_i64_to_i32(
                i64::from(self.z.raw()) * i64::from(Fx::ONE_RAW) / length,
            )),
        )
    }

    /// Coordinate-wise absolute value.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    /// Largest absolute coordinate.
    #[must_use]
    pub const fn max_abs_component(self) -> Fx {
        let absolute = self.abs();
        let xy = if absolute.x.raw() > absolute.y.raw() {
            absolute.x.raw()
        } else {
            absolute.y.raw()
        };
        Fx::from_raw(if xy > absolute.z.raw() {
            xy
        } else {
            absolute.z.raw()
        })
    }
}

impl Add for Vec3Fx {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for Vec3Fx {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for Vec3Fx {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign for Vec3Fx {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul<Fx> for Vec3Fx {
    type Output = Self;

    fn mul(self, rhs: Fx) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<Fx> for Vec3Fx {
    type Output = Self;

    fn div(self, rhs: Fx) -> Self::Output {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for Vec3Fx {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

pub(crate) fn clamp_i64_to_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        i32::try_from(value).expect("value was bounded to i32 immediately above")
    }
}

pub(crate) fn isqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let bits = u128::BITS - value.leading_zeros();
    let mut root = 1u128 << bits.div_ceil(2);
    loop {
        let next = u128::midpoint(root, value / root);
        if next >= root {
            return root;
        }
        root = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_uses_integer_arithmetic() {
        let value = Vec3Fx::new(Fx::from_i32(3), Fx::from_i32(4), Fx::ZERO).normalized();
        assert!((value.x.raw() - 3 * Fx::ONE_RAW / 5).abs() <= 1);
        assert!((value.y.raw() - 4 * Fx::ONE_RAW / 5).abs() <= 1);
        assert_eq!(value.z, Fx::ZERO);
    }

    #[test]
    fn widened_product_saturates_only_after_scaling() {
        let large = Fx::from_raw(i32::MAX);
        assert_eq!((large * large).raw(), i32::MAX);
    }

    #[test]
    fn integer_newton_sqrt_returns_the_floor_across_boundaries() {
        for value in 0..10_000u128 {
            let root = isqrt(value);
            assert!(root * root <= value);
            assert!((root + 1) * (root + 1) > value);
        }
        assert_eq!(isqrt(u128::MAX), u64::MAX.into());
    }
}
