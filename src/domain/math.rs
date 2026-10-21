//! Small numeric helpers shared by the simulation and the renderers.

#![expect(
    clippy::cast_possible_truncation,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]
#![expect(
    clippy::many_single_char_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use std::f32::consts::PI;

pub use std::f32::consts::TAU;

/// An RGB colour with float channels, blended before it is quantised.
pub type Rgb = [f32; 3];

#[must_use]
pub const fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[must_use]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * 2.0f32.mul_add(-t, 3.0)
}

/// Shortest signed angle from `a` to `b`, in (-PI, PI].
#[must_use]
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > PI {
        d -= TAU;
    }
    if d < -PI {
        d += TAU;
    }
    d
}

/// Cheap integer hash. Deterministic, so textures stay stable from frame to frame.
#[must_use]
pub const fn hash3_bits(x: i32, y: i32, z: i32) -> u32 {
    let mut h =
        x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ z.wrapping_mul(1_442_695_041);
    // JavaScript's >>> is a logical shift: shift the bits as unsigned.
    h = (h ^ (h.cast_unsigned() >> 13).cast_signed()).wrapping_mul(1_274_126_177);
    h ^= (h.cast_unsigned() >> 16).cast_signed();
    h.cast_unsigned()
}

/// [`hash3_bits`] mapped to [0, 1).
#[must_use]
pub fn hash3(x: i32, y: i32, z: i32) -> f32 {
    // Through f64: an f32 quotient can round the largest hashes up to 1.
    let unit: f64 = f64::from(hash3_bits(x, y, z)) / 4_294_967_296.0;
    (unit as f32).min(1.0 - f32::EPSILON)
}

/// Integer lattice coordinate for a float (floor, saturating).
#[must_use]
pub fn lattice(v: f32) -> i32 {
    v.floor() as i32
}

#[must_use]
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        (b[0] - a[0]).mul_add(t, a[0]),
        (b[1] - a[1]).mul_add(t, a[1]),
        (b[2] - a[2]).mul_add(t, a[2]),
    ]
}

/// Uniform in [lo, hi).
pub fn rand(rng: &mut fastrand::Rng, lo: f32, hi: f32) -> f32 {
    rng.f32().mul_add(hi - lo, lo)
}

/// Uniform integer in [lo, hi].
pub fn rand_int(rng: &mut fastrand::Rng, lo: i32, hi: i32) -> i32 {
    rng.i32(lo..=hi)
}

/// Either direction, evenly.
pub fn rand_sign(rng: &mut fastrand::Rng) -> f32 {
    if rng.bool() { 1.0 } else { -1.0 }
}

const K1: f32 = 1.0 / 260.0;
const K2: f32 = 1.0 / 310.0;
const K3: f32 = 1.0 / 190.0;
const CURRENT_SCALE: f32 = 3400.0;

/// Divergence-free ocean current: the curl of a slowly drifting stream
/// function psi(x, y, t). Because it has no sources or sinks, it moves the
/// schools around without bunching them up in one corner.
#[must_use]
pub fn current(x: f32, y: f32, t: f32) -> (f32, f32) {
    let a = t.mul_add(0.07, x * K1);
    let b = t.mul_add(-0.05, y * K2);
    let c = t.mul_add(0.11, (x + y) * K3);
    // u = dpsi/dy, v = -dpsi/dx with psi = sin(a)cos(b) + 0.5 sin(c)
    let u = (0.5 * K3).mul_add(c.cos(), -K2 * a.sin() * b.sin()) * CURRENT_SCALE;
    let v = (0.5 * K3).mul_add(-c.cos(), -K1 * a.cos() * b.cos()) * CURRENT_SCALE;
    (u, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn hash_matches_the_javascript_original() {
        // Math.imul-based hash3(1, 2, 3) * 2^32 in the TypeScript source.
        assert_eq!(hash3_bits(0, 0, 0), 0);
        assert_eq!(hash3_bits(1, 2, 3), js_hash3(1, 2, 3));
        assert_eq!(hash3_bits(-7, 52, 9), js_hash3(-7, 52, 9));
    }

    /// The TypeScript hash, transcribed on u32 arithmetic as Math.imul does it.
    fn js_hash3(x: i32, y: i32, z: i32) -> u32 {
        let imul = |a: i32, b: u32| a.cast_unsigned().wrapping_mul(b);
        let mut h = imul(x, 374_761_393) ^ imul(y, 668_265_263) ^ imul(z, 1_442_695_041);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        h ^ (h >> 16)
    }

    #[test]
    fn smoothstep_clamps_and_eases() {
        assert!(smoothstep(0.0, 1.0, -1.0).abs() < f32::EPSILON);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((smoothstep(0.0, 1.0, 2.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mix_interpolates_each_channel() {
        assert_eq!(
            mix([0.0, 10.0, 20.0], [10.0, 30.0, 20.0], 0.5),
            [5.0, 20.0, 20.0]
        );
    }

    proptest! {
        #[test]
        fn hash_is_in_the_unit_interval(x: i32, y: i32, z: i32) {
            let h = hash3(x, y, z);
            prop_assert!((0.0..1.0).contains(&h));
        }

        #[test]
        fn angle_diff_is_the_short_way_round(a in -20.0f32..20.0, b in -20.0f32..20.0) {
            let d = angle_diff(a, b);
            prop_assert!(d > -PI - 1e-4 && d <= PI + 1e-4);
            // a + d points the same way as b.
            prop_assert!(((a + d).cos() - b.cos()).abs() < 1e-3);
            prop_assert!(((a + d).sin() - b.sin()).abs() < 1e-3);
        }

        #[test]
        fn the_current_has_no_divergence(x in 0.0f32..3000.0, y in 0.0f32..2000.0, t in 0.0f32..1000.0) {
            let h = 0.5;
            let (ux1, _) = current(x + h, y, t);
            let (ux0, _) = current(x - h, y, t);
            let (_, vy1) = current(x, y + h, t);
            let (_, vy0) = current(x, y - h, t);
            let div = (ux1 - ux0) / (2.0 * h) + (vy1 - vy0) / (2.0 * h);
            prop_assert!(div.abs() < 1e-2, "divergence {div}");
        }
    }
}
