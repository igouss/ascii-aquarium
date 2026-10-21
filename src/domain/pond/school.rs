//! One step of every live fish: its neighbours, the predators it fears, the
//! walls, the current, and its own restlessness.

#![expect(
    clippy::cast_precision_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]
#![expect(
    clippy::many_single_char_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::{Body, Fish, Pond, nonzero};
use crate::domain::math::{angle_diff, clamp, current};
use crate::domain::species::{SPECIES, Species};

const SEPARATION_K: f32 = 420.0;
const ALIGN_K: f32 = 1.4;
const COHESION_K: f32 = 1.0;
const FLEE_K: f32 = 1100.0;
/// About 127 degrees either side of the heading is visible.
const BLIND_COS: f32 = -0.6;
/// Soft walls start this far in.
const WALL: f32 = 36.0;

/// What a fish sees of the fish around it.
#[derive(Debug, Default)]
struct Neighbourhood {
    /// Push away from fish inside its personal space.
    separation: (f32, f32),
    /// Summed velocity and offset of schoolmates.
    velocity: (f32, f32),
    offset: (f32, f32),
    schoolmates: u32,
    /// The most alarmed visible neighbour.
    alarm: f32,
}

impl Pond {
    pub fn step_fish(&mut self, predators: &[impl Body], dt: f32) {
        let calm_decay = (-dt * 0.7).exp();
        let contagion = (dt * 6.0).min(1.0);

        for i in 0..self.fish.len() {
            if !self.fish[i].alive {
                continue;
            }
            let f = self.fish[i].clone();
            let sp = SPECIES[f.species];
            let speed = nonzero(f.vx.hypot(f.vy), 1.0);
            let heading = (f.vx / speed, f.vy / speed);

            let near = self.neighbourhood(i, heading);
            let (mut ax, mut ay) = steer_with_school(&f, &sp, &near);
            let (flee_x, flee_y, fear) = flee(&f, predators);
            ax += flee_x;
            ay += flee_y;

            // Alarm: direct threat sets it, neighbours pass it along (slightly
            // weaker each hop), and it fades on its own.
            let mut alarm = (f.alarm * calm_decay).max(fear);
            let relayed = near.alarm * 0.86;
            if relayed > alarm {
                alarm = (relayed - alarm).mul_add(contagion, alarm);
            }

            let (wall_x, wall_y) = self.walls(f.x, f.y);
            // Ambient current and a little individual restlessness.
            let (cur_x, cur_y) = current(f.x, f.y, self.time);
            ax += (self.rng.f32() - 0.5).mul_add(sp.jitter, f32::mul_add(cur_x, 0.5, wall_x));
            ay += (self.rng.f32() - 0.5).mul_add(sp.jitter, f32::mul_add(cur_y, 0.5, wall_y));
            // Swim level, except when bolting from danger.
            ay = (f.vy * sp.vertical_drag).mul_add(-0.7f32.mul_add(-alarm, 1.0), ay);
            // Relax toward a preferred speed that rises with panic.
            let preferred = sp.cruise * 1.3f32.mul_add(alarm, 1.0);
            ax = ((preferred - speed) * heading.0).mul_add(1.6, ax);
            ay = ((preferred - speed) * heading.1).mul_add(1.6, ay);

            let (w, h) = (self.width, self.height);
            swim(&mut self.fish[i], &sp, alarm, (ax, ay), dt);
            let f = &mut self.fish[i];
            f.x = clamp(f.x, -60.0, w + 60.0);
            f.y = clamp(f.y, -60.0, h + 60.0);
        }
    }

    /// The visible fish around fish `i`, looking along `heading`, from the
    /// spatial hash buckets next to its own.
    fn neighbourhood(&self, i: usize, heading: (f32, f32)) -> Neighbourhood {
        let f = &self.fish[i];
        let r2 = self.perception * self.perception;
        // Alarmed fish want more room: the school flashes apart around a
        // predator and closes back up once the threat fades.
        let spread = 1.8f32.mul_add(f.alarm, 1.0);
        let sep_x = self.sep_x[f.species] * spread;
        let sep_y = self.sep_y[f.species] * spread;
        let mut near = Neighbourhood::default();
        let (bc, br) = self.bucket_cell(f.x, f.y);
        for r in br.saturating_sub(1)..=(br + 1).min(self.bucket_rows - 1) {
            for c in bc.saturating_sub(1)..=(bc + 1).min(self.bucket_cols - 1) {
                let mut cursor = self.bucket_head[r * self.bucket_cols + c];
                while let Some(j) = cursor {
                    cursor = self.bucket_next[j];
                    let o = &self.fish[j];
                    let (dx, dy) = (o.x - f.x, o.y - f.y);
                    let d2 = f32::mul_add(dy, dy, dx * dx);
                    // Not itself, not out of range, and not in the blind zone
                    // directly behind.
                    if j == i
                        || d2 >= r2
                        || d2 <= 1e-6
                        || f32::mul_add(dy, heading.1, dx * heading.0) / d2.sqrt() <= BLIND_COS
                    {
                        continue;
                    }
                    let d = d2.sqrt();
                    // Elliptical personal space: wide along x, tight along y.
                    let (qx, qy) = (dx / sep_x, dy / sep_y);
                    let q2 = f32::mul_add(qy, qy, qx * qx);
                    if q2 < 1.0 {
                        let q = nonzero(q2.sqrt(), 1e-3);
                        let k = (1.0 - q) / q;
                        near.separation.0 = f32::mul_add(qx, -k, near.separation.0);
                        near.separation.1 = f32::mul_add(qy, -k, near.separation.1);
                    }
                    if o.species == f.species {
                        near.velocity.0 += o.vx;
                        near.velocity.1 += o.vy;
                        near.offset.0 += dx;
                        near.offset.1 += dy;
                        near.schoolmates += 1;
                    } else if d < self.foreign_sep {
                        // Different species keep their own schools.
                        let k = ((1.0 - d / self.foreign_sep) / d) * 0.6;
                        near.separation.0 = f32::mul_add(dx, -k, near.separation.0);
                        near.separation.1 = f32::mul_add(dy, -k, near.separation.1);
                    }
                    near.alarm = near.alarm.max(o.alarm);
                }
            }
        }
        near
    }

    /// Soft walls: a push back toward the middle near each edge.
    fn walls(&self, x: f32, y: f32) -> (f32, f32) {
        let push = |v: f32, size: f32| {
            if v < WALL {
                (WALL - v) * 7.0
            } else if v > size - WALL {
                -(v - (size - WALL)) * 7.0
            } else {
                0.0
            }
        };
        (push(x, self.width), push(y, self.height))
    }
}

/// Separation, alignment and cohesion; mill-forming species also circle the
/// local centre, so the school spins into a bait ball.
fn steer_with_school(f: &Fish, sp: &Species, near: &Neighbourhood) -> (f32, f32) {
    let mut ax = near.separation.0 * SEPARATION_K;
    let mut ay = near.separation.1 * SEPARATION_K;
    if near.schoolmates > 0 {
        let n = near.schoolmates as f32;
        let (cx, cy) = near.offset;
        let align_k = ALIGN_K * sp.align;
        let cohesion_k = COHESION_K * sp.cohesion * 0.8f32.mul_add(-f.alarm, 1.0);
        ax += (cx / n).mul_add(cohesion_k, (near.velocity.0 / n - f.vx) * align_k);
        ay += (cy / n).mul_add(cohesion_k, (near.velocity.1 / n - f.vy) * align_k);
        if sp.mill > 0.0 {
            let cl = nonzero(cx.hypot(cy), 1.0);
            let pull = f.mill_dir * sp.mill * (1.0 - f.alarm);
            ax = (-cy / cl).mul_add(pull, ax);
            ay = (cx / cl).mul_add(pull, ay);
        }
    }
    (ax, ay)
}

/// The push away from every predator in range, and how afraid that makes
/// the fish. Distance is measured to the nearest point along the front half
/// of the body, so fish also give the flanks a wide berth.
fn flee(f: &Fish, predators: &[impl Body]) -> (f32, f32, f32) {
    let (mut ax, mut ay, mut fear) = (0.0, 0.0, 0.0f32);
    for body in predators {
        let nodes = body.nodes();
        let fear_r = f32::mul_add(body.length(), 0.6, 60.0);
        let (bx, by) = (0..=12)
            .step_by(3)
            .map(|k| (nodes[k * 2], nodes[k * 2 + 1]))
            .min_by(|a, b| dist2(f, *a).total_cmp(&dist2(f, *b)))
            .unwrap_or((nodes[0], nodes[1]));
        let d = dist2(f, (bx, by)).sqrt();
        if d >= fear_r || d < 1e-3 {
            continue;
        }
        let w = (1.0 - d / fear_r).powi(2);
        let away = ((f.x - bx) / d, (f.y - by) / d);
        // Fountain effect: break to whichever side of the path you're on.
        let (shy, shx) = body.heading().sin_cos();
        let side = if f32::mul_add(shy, -(f.x - nodes[0]), shx * (f.y - nodes[1])) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        ax = (f32::mul_add(shy * side, -0.9, away.0 * 0.7) * FLEE_K).mul_add(w, ax);
        ay = (f32::mul_add(shx * side, 0.9, away.1 * 0.7) * FLEE_K).mul_add(w, ay);
        fear = fear.max(w.sqrt());
    }
    (ax, ay, fear)
}

fn dist2(f: &Fish, (x, y): (f32, f32)) -> f32 {
    (f.y - y).mul_add(f.y - y, (f.x - x).powi(2))
}

/// Accelerates `f` by `accel` for `dt`, within the speeds its species and
/// `alarm` allow, and moves it.
fn swim(f: &mut Fish, sp: &Species, alarm: f32, accel: (f32, f32), dt: f32) {
    f.alarm = alarm;
    let before = f.vy.atan2(f.vx);
    f.vx = accel.0.mul_add(dt, f.vx);
    f.vy = accel.1.mul_add(dt, f.vy);
    let max_speed = sp.max_speed * 1.8f32.mul_add(alarm, 1.0);
    let s = f.vx.hypot(f.vy);
    if s > max_speed {
        f.vx *= max_speed / s;
        f.vy *= max_speed / s;
    } else if s < sp.min_speed && s > 1e-3 {
        f.vx *= sp.min_speed / s;
        f.vy *= sp.min_speed / s;
    }
    f.turn =
        (angle_diff(before, f.vy.atan2(f.vx)) / dt - f.turn).mul_add((dt * 8.0).min(1.0), f.turn);
    f.x = f.vx.mul_add(dt, f.x);
    f.y = f.vy.mul_add(dt, f.y);
}

#[cfg(test)]
mod tests {
    use super::super::{NODES, test_pond};
    use super::*;

    struct Still([f32; NODES * 2]);

    impl Body for Still {
        fn nodes(&self) -> &[f32; NODES * 2] {
            &self.0
        }
        fn length(&self) -> f32 {
            200.0
        }
        fn heading(&self) -> f32 {
            0.0
        }
    }

    #[test]
    fn fish_stay_near_the_water() {
        let mut pond = test_pond();
        let none: [Still; 0] = [];
        for _ in 0..600 {
            pond.time += 1.0 / 60.0;
            pond.rebuild_hash();
            pond.step_fish(&none, 1.0 / 60.0);
        }
        for f in &pond.fish {
            assert!((-60.0..=1660.0).contains(&f.x) && (-60.0..=960.0).contains(&f.y));
            assert!(f.vx.is_finite() && f.vy.is_finite());
        }
    }

    #[test]
    fn a_predator_alarms_the_fish_beside_it() {
        let mut pond = test_pond();
        let (x, y) = (pond.fish[0].x, pond.fish[0].y);
        let mut nodes = [0.0; NODES * 2];
        for k in 0..NODES {
            nodes[k * 2] = (k as f32).mul_add(-10.0, x - 10.0);
            nodes[k * 2 + 1] = y;
        }
        pond.rebuild_hash();
        pond.step_fish(&[Still(nodes)], 1.0 / 60.0);
        assert!(pond.fish[0].alarm > 0.5, "{}", pond.fish[0].alarm);
    }

    #[test]
    fn walls_push_back_only_near_the_edges() {
        let pond = test_pond();
        assert_eq!(pond.walls(800.0, 450.0), (0.0, 0.0));
        assert_eq!(pond.walls(30.0, 880.0), (42.0, -112.0));
    }

    #[test]
    fn speed_stays_within_the_species_limits() {
        let sp = SPECIES[0];
        let mut f = test_pond().fish[0].clone();
        f.species = 0;
        swim(&mut f, &sp, 0.0, (1e6, 0.0), 1.0 / 60.0);
        assert!((f.vx.hypot(f.vy) - sp.max_speed).abs() < 1e-2);
        f.vx = 0.1;
        f.vy = 0.0;
        swim(&mut f, &sp, 0.0, (0.0, 0.0), 1.0 / 60.0);
        assert!((f.vx.hypot(f.vy) - sp.min_speed).abs() < 1e-3);
    }
}
