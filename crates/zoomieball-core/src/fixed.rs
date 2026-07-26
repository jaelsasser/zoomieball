//! Q16.16 scalar and three-dimensional vector arithmetic.

use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Return the exact signed product of two canonical words.
#[must_use]
pub fn mul64(lhs: i32, rhs: i32) -> i64 {
    i64::from(lhs) * i64::from(rhs)
}

/// Multiply two raw Q16.16 words with truncation toward zero.
///
/// # Panics
///
/// Panics when the exact Q16.16 result does not fit in `i32`.
#[must_use]
pub fn qmul(lhs: i32, rhs: i32) -> i32 {
    let magnitude = u64::from(lhs.unsigned_abs()) * u64::from(rhs.unsigned_abs());
    signed_magnitude(magnitude >> Fx::FRACTION_BITS, (lhs < 0) ^ (rhs < 0))
}

/// Divide two raw Q16.16 words with truncation toward zero.
///
/// # Panics
///
/// Panics when `rhs` is zero or the exact Q16.16 result does not fit in `i32`.
#[must_use]
pub fn qdiv(lhs: i32, rhs: i32) -> i32 {
    assert_ne!(rhs, 0, "fixed-point division by zero");
    let numerator = u64::from(lhs.unsigned_abs()) << Fx::FRACTION_BITS;
    let magnitude = numerator / u64::from(rhs.unsigned_abs());
    signed_magnitude(magnitude, (lhs < 0) ^ (rhs < 0))
}

/// Return `floor(sqrt(value))` for the complete `u64` input domain.
#[must_use]
pub fn isqrt64(value: u64) -> u32 {
    if value < 2 {
        return u32::try_from(value).expect("values below two fit u32");
    }

    let bits = u64::BITS - value.leading_zeros();
    let mut root = 1u64 << bits.div_ceil(2);
    loop {
        let next = u64::midpoint(root, value / root);
        if next >= root {
            return u32::try_from(root).expect("the square root of u64 fits u32");
        }
        root = next;
    }
}

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
    ///
    /// # Panics
    ///
    /// Panics when `value` lies outside the Q16.16 integer range.
    #[must_use]
    pub const fn from_i32(value: i32) -> Self {
        assert!(
            value >= -32_768 && value <= 32_767,
            "integer does not fit Q16.16"
        );
        Self(value * Self::ONE_RAW)
    }

    /// Convert at the cosmetic rendering boundary.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Self::ONE_RAW as f32
    }

    /// Two's-complement wrapping absolute value.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(self.0.wrapping_abs())
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
        Self(self.0.signum() * Self::ONE_RAW)
    }

    /// Widened multiply with one final Q16.16 shift.
    #[must_use]
    pub fn mul_wide(self, rhs: Self) -> Self {
        Self(qmul(self.0, rhs.0))
    }

    /// Widened divide with the numerator shifted before division.
    #[must_use]
    pub fn div_wide(self, rhs: Self) -> Self {
        Self(qdiv(self.0, rhs.0))
    }

    /// Integer square root of a nonnegative Q16.16 value.
    #[must_use]
    pub fn sqrt(self) -> Self {
        assert!(self.0 >= 0, "fixed-point square root of a negative value");
        let radicand = u64::from(self.0.cast_unsigned()) << Self::FRACTION_BITS;
        Self(i32::try_from(isqrt64(radicand)).expect("Q16.16 square root fits i32"))
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
        Self(self.0.wrapping_add(rhs.0))
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
        Self(self.0.wrapping_sub(rhs.0))
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
        Self(qmul(self.0, rhs.0))
    }
}

impl Div for Fx {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        Self(qdiv(self.0, rhs.0))
    }
}

impl Neg for Fx {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(self.0.wrapping_neg())
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

    /// Widened dot product with one final Q16.16 shift.
    #[must_use]
    pub fn dot(self, rhs: Self) -> Fx {
        let sum = i128::from(mul64(self.x.raw(), rhs.x.raw()))
            + i128::from(mul64(self.y.raw(), rhs.y.raw()))
            + i128::from(mul64(self.z.raw(), rhs.z.raw()));
        Fx::from_raw(renormalize(sum))
    }

    /// Cross product.
    #[must_use]
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            Fx::from_raw(renormalize(
                i128::from(mul64(self.y.raw(), rhs.z.raw()))
                    - i128::from(mul64(self.z.raw(), rhs.y.raw())),
            )),
            Fx::from_raw(renormalize(
                i128::from(mul64(self.z.raw(), rhs.x.raw()))
                    - i128::from(mul64(self.x.raw(), rhs.z.raw())),
            )),
            Fx::from_raw(renormalize(
                i128::from(mul64(self.x.raw(), rhs.y.raw()))
                    - i128::from(mul64(self.y.raw(), rhs.x.raw())),
            )),
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
        let squared = [self.x.raw(), self.y.raw(), self.z.raw()]
            .into_iter()
            .map(|component| {
                u64::try_from(mul64(component, component))
                    .expect("a squared component is nonnegative")
            })
            .sum();
        Fx::from_raw(i32::try_from(isqrt64(squared)).expect("bounded vector length fits Q16.16"))
    }

    /// Unit vector, or zero when the input has no direction.
    #[must_use]
    pub fn normalized(self) -> Self {
        let length = self.length();
        if length == Fx::ZERO {
            return Self::ZERO;
        }
        Self::new(
            Fx::from_raw(qdiv(self.x.raw(), length.raw())),
            Fx::from_raw(qdiv(self.y.raw(), length.raw())),
            Fx::from_raw(qdiv(self.z.raw(), length.raw())),
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

fn signed_magnitude(magnitude: u64, negative: bool) -> i32 {
    let magnitude = i64::try_from(magnitude).expect("fixed-point magnitude fits i64");
    let value = if negative { -magnitude } else { magnitude };
    i32::try_from(value).expect("fixed-point result fits i32")
}

fn renormalize(value: i128) -> i32 {
    i32::try_from(value / i128::from(Fx::ONE_RAW)).expect("fixed-point result fits i32")
}
