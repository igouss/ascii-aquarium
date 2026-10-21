//! Koi body outlines, fins and skin noise, as fractions of length.

use crate::domain::math::{hash3, lattice, smoothstep};
use crate::domain::pond::nonzero;

/// Top-down body half-width at s (snout to tail tip), as a fraction of
/// length. `end` is where the body has narrowed into the tail stalk; the tail
/// fin is drawn past it. A rounded head, a slim torpedo, a long stalk.
pub(super) fn koi_body(s: f32, end: f32) -> f32 {
    let k = end / 0.7;
    let x = s / k;
    let w = if x < 0.08 {
        0.07 * ((x / 0.08) * std::f32::consts::FRAC_PI_2).sin().powf(0.55)
    } else if x < 0.27 {
        0.017f32.mul_add(smoothstep(0.08, 0.27, x), 0.07)
    } else if x < 0.66 {
        0.067f32.mul_add((1.0 - (x - 0.27) / 0.39).powf(1.05), 0.02)
    } else {
        0.02f32.mul_add(1.0 - smoothstep(0.66, 0.78, x), 0.004)
    };
    w * (k * 1.1).min(1.0)
}

/// Body outline and fin layout, as fractions of length.
#[derive(Debug, Clone, Copy)]
pub(super) struct Shape {
    /// Where the body has narrowed into the tail stalk.
    pub(super) end: f32,
    /// Pectoral fin: leading root, trailing root, tip position, and reach.
    pub(super) pectoral: [f32; 4],
    /// Pelvic fins: leading root, trailing root, tip, reach.
    pub(super) pelvic: [f32; 4],
    /// Tail fin: where it starts, how wide each lobe fans, the fork depth.
    pub(super) tail: [f32; 3],
    /// Fin breadth relative to length; long butterfly fins are slimmer.
    pub(super) fin_breadth: f32,
    /// Where the back is rounded enough to catch a soft sheen.
    pub(super) dorsal: [f32; 2],
}

pub(super) const STANDARD: Shape = Shape {
    end: 0.7,
    pectoral: [0.14, 0.19, 0.36, 0.12],
    pelvic: [0.41, 0.45, 0.52, 0.075],
    tail: [0.68, 0.1, 0.05],
    fin_breadth: 0.55,
    dorsal: [0.12, 0.6],
};

// Butterfly koi: the same body packed into the front 58%, then long flowing
// pectorals and a trailing veil of a tail.
pub(super) const BUTTERFLY: Shape = Shape {
    end: 0.58,
    pectoral: [0.11, 0.16, 0.4, 0.13],
    pelvic: [0.34, 0.38, 0.48, 0.08],
    tail: [0.55, 0.11, 0.055],
    fin_breadth: 0.42,
    dorsal: [0.1, 0.5],
};

/// Smooth value noise in [0, 1), seeded per koi so each wears its own pattern.
pub(super) fn noise(x: f32, y: f32, seed: i32) -> f32 {
    let (xi, yi) = (lattice(x), lattice(y));
    let (xf, yf) = (x - x.floor(), y - y.floor());
    let u = xf * xf * 2.0f32.mul_add(-xf, 3.0);
    let v = yf * yf * 2.0f32.mul_add(-yf, 3.0);
    let a = hash3(xi, yi, seed);
    let b = hash3(xi + 1, yi, seed);
    let c = hash3(xi, yi + 1, seed);
    let d = hash3(xi + 1, yi + 1, seed);
    (a - b - c + d).mul_add(u * v, (c - a).mul_add(v, (b - a).mul_add(u, a)))
}

/// Fin rays fanning out from a root: thin darker stripes, so fins read as fins.
pub(super) fn ray(du: f32, a: f32) -> f32 {
    if (a.atan2(du) * 26.0).sin() > 0.45 {
        0.1
    } else {
        0.0
    }
}

/// Whether (u along, a across) falls in a paddle-shaped fin from the root on
/// the spine to the tip. Returns 0 at the root to 1 at the tip.
pub(super) fn in_fin(u: f32, a: f32, root_u: f32, tip: (f32, f32), breadth: f32) -> Option<f32> {
    let (dx, dy) = (tip.0 - root_u, tip.1);
    let len = nonzero(dx.hypot(dy), 1.0);
    let (ex, ey) = (dx / len, dy / len);
    let half = len / 2.0;
    let qx = u - (root_u + dx / 2.0);
    let qy = a - dy / 2.0;
    let p = qx.mul_add(ex, qy * ey) / half;
    if !(-1.0..=1.0).contains(&p) {
        return None;
    }
    // Narrow at the root, fanning out to a rounded tip.
    let t = f32::midpoint(p, 1.0);
    let width = half * breadth * 0.55f32.mul_add((t * std::f32::consts::PI * 0.85).sin(), 0.45);
    let q = (-qx).mul_add(ey, qy * ex) / width;
    (p.mul_add(p, q * q) <= 1.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_continuous_across_lattice_lines() {
        let below = noise(2.999_9, 5.5, 1);
        let above = noise(3.000_1, 5.5, 1);
        assert!((below - above).abs() < 1e-3);
        assert!((noise(3.0, 5.0, 1) - hash3(3, 5, 1)).abs() < 1e-6);
    }
}
