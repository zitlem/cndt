// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sine and cosine of an angle in degrees, the same to the last bit on
//! every platform (architecture rule 9).
//!
//! The standard library's `sin` and `cos` call the platform's maths
//! library, which may round differently on another system. These use only
//! addition, subtraction, multiplication and division, which IEEE 754
//! defines exactly: the angle is brought into 0° to 45° by exact steps,
//! then a fixed Taylor polynomial is evaluated in a fixed order. Multiples
//! of 90° give exactly 0 and ±1.

/// π to the nearest `f64`.
const PI: f64 = std::f64::consts::PI;

/// `(sin, cos)` of `degrees`. Within a few 10⁻¹⁶ of the true values;
/// exact at multiples of 90°.
#[must_use]
pub fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    if !degrees.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    // Into 0..360: `%` is exact.
    let mut d = degrees % 360.0;
    if d < 0.0 {
        d += 360.0;
    }
    // The quadrant, and the angle within it, 0..90.
    let quadrant = (d / 90.0).floor();
    let r = d - quadrant * 90.0;
    // Within the quadrant, from 0..45 by the complement.
    let (s, c) = if r < 45.0 {
        polynomial(r)
    } else if r > 45.0 {
        let (s, c) = polynomial(90.0 - r);
        (c, s)
    } else {
        // Exactly equal at 45°, as they are.
        (
            std::f64::consts::FRAC_1_SQRT_2,
            std::f64::consts::FRAC_1_SQRT_2,
        )
    };
    #[allow(clippy::cast_possible_truncation)]
    match quadrant as i64 {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// `(sin, cos)` of `r` degrees, 0 ≤ r < 45, by Taylor series to x¹⁸. At
/// 0 it gives exactly 0 and 1.
fn polynomial(r: f64) -> (f64, f64) {
    let x = r * (PI / 180.0);
    let x2 = x * x;
    // Horner's rule, highest term first, each step the same everywhere.
    let mut s = 0.0;
    let mut c = 0.0;
    for n in (0..=8).rev() {
        // sin: (−1)ⁿ x²ⁿ⁺¹ / (2n+1)!; cos: (−1)ⁿ x²ⁿ / (2n)!.
        s = s * x2 + sign(n) / factorial(2 * n + 1);
        c = c * x2 + sign(n) / factorial(2 * n);
    }
    (s * x, c)
}

const fn sign(n: u32) -> f64 {
    if n.is_multiple_of(2) { 1.0 } else { -1.0 }
}

fn factorial(n: u32) -> f64 {
    (1..=n).fold(1.0, |acc, k| acc * f64::from(k))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;

    #[test]
    fn exact_at_quarter_turns() {
        assert_eq!(sin_cos_degrees(0.0), (0.0, 1.0));
        assert_eq!(sin_cos_degrees(90.0), (1.0, 0.0));
        assert_eq!(sin_cos_degrees(180.0), (0.0, -1.0));
        assert_eq!(sin_cos_degrees(270.0), (-1.0, 0.0));
        assert_eq!(sin_cos_degrees(360.0), (0.0, 1.0));
        assert_eq!(sin_cos_degrees(-90.0), (-1.0, 0.0));
    }

    #[test]
    fn close_to_the_library_everywhere() {
        let mut d = -720.0;
        while d <= 720.0 {
            let (s, c) = sin_cos_degrees(d);
            // Against the library on the angle reduced exactly to 0..360:
            // its own radians of a large angle carry an error.
            let x = d.rem_euclid(360.0).to_radians();
            assert!((s - x.sin()).abs() < 1e-15, "sin {d}: {s} vs {}", x.sin());
            assert!((c - x.cos()).abs() < 1e-15, "cos {d}: {c} vs {}", x.cos());
            d += 0.37;
        }
        let (s, c) = sin_cos_degrees(45.0);
        assert!((s - c).abs() < 1e-16 && (s * s + c * c - 1.0).abs() < 1e-15);
        let (s, _) = sin_cos_degrees(30.0);
        assert!((s - 0.5).abs() < 1e-16, "{s}");
    }

    /// The exact bits for a few angles: if a change alters them, toolpaths
    /// on turned setups change too.
    #[test]
    fn fixed_bits() {
        assert_eq!(sin_cos_degrees(30.0).0.to_bits(), 4_602_678_819_172_646_911);
        assert_eq!(sin_cos_degrees(45.0).1.to_bits(), 0x3FE6_A09E_667F_3BCD);
    }
}
