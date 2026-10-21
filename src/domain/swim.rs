//! Steering both predators share: a follow-the-leader spine, a wandering
//! heading, pursuit of prey, and a clear middle.

#![expect(clippy::cast_precision_loss, reason = "node indices are below 20")]

use super::math::{angle_diff, clamp};
use super::pond::{Fish, NODES, Pond, nonzero};

/// A straight spine of `length` from (x, y) back along `heading`.
#[must_use]
pub fn straight_spine(x: f32, y: f32, heading: f32, length: f32) -> [f32; NODES * 2] {
    let mut nodes = [0.0; NODES * 2];
    let seg = length / (NODES - 1) as f32;
    let (sin, cos) = heading.sin_cos();
    for n in 0..NODES {
        nodes[n * 2] = (cos * seg).mul_add(-(n as f32), x);
        nodes[n * 2 + 1] = (sin * seg).mul_add(-(n as f32), y);
    }
    nodes
}

/// Follow-the-leader spine, with a stiffness limit of `max_bend` per joint so
/// the body bends into a C at most and never folds back on itself.
pub fn follow(nodes: &mut [f32; NODES * 2], heading: f32, length: f32, max_bend: f32) {
    let seg = length / (NODES - 1) as f32;
    let mut prev_angle = heading + std::f32::consts::PI;
    for k in 1..NODES {
        let px = nodes[(k - 1) * 2];
        let py = nodes[(k - 1) * 2 + 1];
        let angle = (nodes[k * 2 + 1] - py).atan2(nodes[k * 2] - px);
        let angle = prev_angle + clamp(angle_diff(prev_angle, angle), -max_bend, max_bend);
        nodes[k * 2] = angle.cos().mul_add(seg, px);
        nodes[k * 2 + 1] = angle.sin().mul_add(seg, py);
        prev_angle = angle;
    }
}

/// Wander: a slowly drifting offset on a circle ahead of the snout. Moves
/// `wander` and returns the pull it gives.
pub fn wander(pond: &mut Pond, heading: f32, wander: &mut f32, dt: f32) -> (f32, f32) {
    *wander = clamp((pond.rng.f32() - 0.5).mul_add(dt * 3.2, *wander), -1.1, 1.1);
    let (sin, cos) = (heading + *wander).sin_cos();
    (cos * 0.45, sin * 0.45)
}

/// Pursuit of `prey` from `head` by a predator of `length` and `hunger`: the
/// pull toward where the fish is about to be, and how close the chase is
/// (0 far to 1 at the mouth).
#[must_use]
pub fn pursue(prey: &Fish, head: (f32, f32), length: f32, hunger: f32) -> ((f32, f32), f32) {
    let tx = prey.vx.mul_add(0.35, prey.x) - head.0;
    let ty = prey.vy.mul_add(0.35, prey.y) - head.1;
    let d = nonzero(tx.hypot(ty), 1.0);
    let pull = 2.6f32.mul_add(hunger, 1.4);
    (
        (tx / d * pull, ty / d * pull),
        clamp(1.0 - d / (length * 0.9), 0.0, 1.0),
    )
}

/// Keeps the middle mostly clear: a soft elliptical no-swim zone, firmer
/// while patrolling than while `hunting`.
#[must_use]
pub fn clear_middle(pond: &Pond, head: (f32, f32), hunting: bool) -> (f32, f32) {
    let ex = (head.0 - pond.width / 2.0) / (pond.width * 0.3);
    let ey = (head.1 - pond.height / 2.0) / (pond.height * 0.16);
    let e2 = ex.mul_add(ex, ey * ey);
    if e2 >= 1.0 {
        return (0.0, 0.0);
    }
    let e = nonzero(e2.sqrt(), 1e-3);
    let push = (1.0 - e) * if hunting { 1.0 } else { 2.5 };
    (ex / e * push, ey / e * push)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spine_keeps_its_segment_length() {
        let mut nodes = straight_spine(100.0, 100.0, 0.0, 190.0);
        nodes[0] += 30.0;
        nodes[1] += 40.0;
        follow(&mut nodes, 0.9, 190.0, 0.16);
        for k in 1..NODES {
            let d = (nodes[k * 2] - nodes[(k - 1) * 2])
                .hypot(nodes[k * 2 + 1] - nodes[(k - 1) * 2 + 1]);
            assert!((d - 10.0).abs() < 1e-3, "segment {k}: {d}");
        }
    }

    #[test]
    fn the_middle_pushes_outward_and_only_inside_its_ellipse() {
        let pond = Pond::new(1000.0, 1000.0, 10.0, 15.0, fastrand::Rng::with_seed(1));
        assert_eq!(clear_middle(&pond, (900.0, 500.0), false), (0.0, 0.0));
        let (x, y) = clear_middle(&pond, (600.0, 500.0), false);
        assert!(x > 0.0 && y.abs() < 1e-6);
        let (hunting, _) = clear_middle(&pond, (600.0, 500.0), true);
        assert!(hunting < x, "hunters cut through");
    }
}
