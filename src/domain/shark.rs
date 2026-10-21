//! Sharks, after stackedlol's ascii-sharks: Reynolds steering (wander,
//! pursuit, separation) on a follow-the-leader spine. Hunger builds over time
//! and drops when they eat. They prefer fish in sparse spots, since dense
//! schools confuse predators, so stragglers get picked off.

#![expect(
    clippy::cast_precision_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]
#![expect(
    clippy::many_single_char_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::math::{TAU, angle_diff, clamp, rand};
use super::pond::{Body, Hunt, NODES, Pond, nonzero};
use super::swim::{clear_middle, follow, pursue, straight_spine, wander};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharkKind {
    Classic,
    Hammerhead,
    Zebra,
    Tiger,
    GreatHammer,
    Mako,
    Thresher,
    Nurse,
}

impl SharkKind {
    pub const ALL: [Self; 8] = [
        Self::Classic,
        Self::Hammerhead,
        Self::Zebra,
        Self::Tiger,
        Self::GreatHammer,
        Self::Mako,
        Self::Thresher,
        Self::Nurse,
    ];

    /// The name upstream's `?sharks=` takes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Hammerhead => "hammerhead",
            Self::Zebra => "zebra",
            Self::Tiger => "tiger",
            Self::GreatHammer => "greathammer",
            Self::Mako => "mako",
            Self::Thresher => "thresher",
            Self::Nurse => "nurse",
        }
    }

    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Upstream's `?sharks=` cast: the first is biggest, the rest step down.
#[must_use]
pub fn cast(kinds: &[SharkKind]) -> Vec<SharkSpec> {
    kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| SharkSpec {
            kind,
            size: (i as f32).mul_add(-0.1, 1.0),
        })
        .collect()
}

/// Which shark to put in the water; size is relative to the biggest.
#[derive(Debug, Clone, Copy)]
pub struct SharkSpec {
    pub kind: SharkKind,
    pub size: f32,
}

/// The upstream landing hero: four of the same shark at different depths.
/// The screensaver swims the full cast instead; the tests use this one.
#[cfg(test)]
pub const DEFAULT_SHARKS: [SharkSpec; 4] = [
    SharkSpec {
        kind: SharkKind::Classic,
        size: 1.0,
    },
    SharkSpec {
        kind: SharkKind::Classic,
        size: 0.7,
    },
    SharkSpec {
        kind: SharkKind::Classic,
        size: 0.84,
    },
    SharkSpec {
        kind: SharkKind::Classic,
        size: 0.77,
    },
];

#[derive(Debug, Clone)]
pub struct Shark {
    pub id: i32,
    pub kind: SharkKind,
    pub length: f32,
    /// Spine positions, snout first: [x0, y0, x1, y1, ...].
    pub nodes: [f32; NODES * 2],
    pub heading: f32,
    pub speed: f32,
    /// Tail-beat phase; drives the body wave in the renderer.
    pub phase: f32,
    /// Smoothed angular velocity (rad/s); the shark banks its fins into turns.
    pub turn: f32,
    hunger: f32,
    wander: f32,
    orbit_dir: f32,
    orbit_radius: f32,
    target: Option<usize>,
    retarget_at: f32,
    /// 0 far and dim to 1 near and bright.
    pub depth: f32,
}

impl Body for Shark {
    fn nodes(&self) -> &[f32; NODES * 2] {
        &self.nodes
    }
    fn length(&self) -> f32 {
        self.length
    }
    fn heading(&self) -> f32 {
        self.heading
    }
}

/// Sharks set out on orbits around the middle, smaller (farther) ones first
/// in drawing order so the big one swims over them.
pub fn seed(specs: &[SharkSpec], pond: &mut Pond) -> Vec<Shark> {
    let (w, h) = (pond.width, pond.height);
    let min_side = w.min(h);
    let base_length = clamp(min_side * 0.44, 170.0, 430.0);
    let count = specs.len() as f32;
    let mut sharks: Vec<Shark> = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let rng = &mut pond.rng;
            let length = base_length * spec.size;
            let orbit_radius = min_side * rand(rng, 0.28, 0.4);
            let a = (i as f32 / count).mul_add(TAU, rand(rng, -0.4, 0.4));
            let orbit_dir = if i % 2 == 0 { 1.0 } else { -1.0 };
            let x = (a.cos() * orbit_radius * (w / min_side)).mul_add(0.9, w / 2.0);
            let y = a.sin().mul_add(orbit_radius, h / 2.0);
            let heading = f32::mul_add(orbit_dir, std::f32::consts::FRAC_PI_2, a);
            Shark {
                id: i32::try_from(i).unwrap_or(0) + 1,
                kind: spec.kind,
                length,
                nodes: straight_spine(x, y, heading, length),
                heading,
                speed: 30.0,
                phase: rand(rng, 0.0, TAU),
                turn: 0.0,
                hunger: rand(rng, 0.25, 0.75),
                wander: 0.0,
                orbit_dir,
                orbit_radius,
                target: None,
                retarget_at: 0.0,
                depth: spec.size,
            }
        })
        .collect();
    sharks.sort_by(|a, b| a.length.total_cmp(&b.length));
    sharks
}

pub fn step(sharks: &mut [Shark], pond: &mut Pond, dt: f32) {
    for i in 0..sharks.len() {
        let mut sh = sharks[i].clone();
        let (hunting, closing) = steer(&mut sh, i, sharks, pond, dt);
        swim(&mut sh, hunting, closing, dt);
        // Feed.
        if sh.hunger > 0.15 {
            let eaten = pond.feed((sh.nodes[0], sh.nodes[1]), sh.length.mul_add(0.05, 8.0));
            sh.hunger = (eaten as f32).mul_add(-0.16, sh.hunger).max(0.0);
        }
        sharks[i] = sh;
    }
}

/// Turns shark `sh`, number `i` of `sharks`, toward where it wants to go.
/// Returns whether it is hunting, and how close the chase is.
fn steer(sh: &mut Shark, i: usize, sharks: &[Shark], pond: &mut Pond, dt: f32) -> (bool, f32) {
    let (w, h, time) = (pond.width, pond.height, pond.time);
    let l = sh.length;
    let head = (sh.nodes[0], sh.nodes[1]);
    sh.hunger = dt.mul_add(0.03, sh.hunger).min(1.0);

    // Momentum, plus a wander.
    let (mut dy, mut dx) = sh.heading.sin_cos();
    let (wx, wy) = wander(pond, sh.heading, &mut sh.wander, dt);
    dx += wx;
    dy += wy;

    if time >= sh.retarget_at {
        sh.target = pond.find_prey(Hunt {
            head,
            heading: sh.heading,
            range: l.mul_add(0.9, 160.0),
            current: sh.target,
            margin: None,
        });
        sh.retarget_at = time + 0.45;
    }
    let chase = sh
        .target
        .map(|t| &pond.fish[t])
        .filter(|t| t.alive && sh.hunger > 0.3)
        .map(|t| pursue(t, head, l, sh.hunger));
    let (hunting, closing) = if let Some(((px, py), closing)) = chase {
        dx += px;
        dy += py;
        (true, closing)
    } else {
        // Patrol: a lazy circuit around the middle.
        let min_side = w.min(h);
        let rx = (head.0 - w / 2.0) / (w / min_side);
        let ry = head.1 - h / 2.0;
        let r = nonzero(rx.hypot(ry), 1.0);
        let radius = sh.orbit_radius * 0.15f32.mul_add(time.mul_add(0.05, sh.id as f32).sin(), 1.0);
        let radial = clamp((radius - r) / radius, -1.0, 1.0);
        dx = ((rx / r) * radial)
            .mul_add(0.9, (-ry / r) * sh.orbit_dir)
            .mul_add(0.55, dx);
        dy = ((ry / r) * radial)
            .mul_add(0.9, (rx / r) * sh.orbit_dir)
            .mul_add(0.55, dy);
        (false, 0.0)
    };

    // Give other sharks room.
    for other in sharks
        .iter()
        .enumerate()
        .filter(|&(j, _)| j != i)
        .map(|(_, o)| o)
    {
        let range = (l + other.length) * 0.45;
        for k in (0..NODES).step_by(4) {
            let (ox, oy) = (head.0 - other.nodes[k * 2], head.1 - other.nodes[k * 2 + 1]);
            let d = ox.hypot(oy);
            if d < range && d > 1e-3 {
                let push = (1.0 - d / range) * 3.0;
                dx = (ox / d).mul_add(push, dx);
                dy = (oy / d).mul_add(push, dy);
            }
        }
    }

    let (mx, my) = clear_middle(pond, head, hunting);
    dx += mx;
    dy += my;

    // Turn back once the snout leaves the frame, so bodies dip off-screen
    // now and then but the sharks never wander away for good.
    let over = (-head.0).max(head.0 - w).max(-head.1).max(head.1 - h);
    if over > -l * 0.05 {
        let pull = clamp(l.mul_add(0.05, over) / (l * 0.35), 0.0, 1.0) * 3.5;
        let (to_x, to_y) = (w / 2.0 - head.0, h / 2.0 - head.1);
        let d = nonzero(to_x.hypot(to_y), 1.0);
        dx = (to_x / d).mul_add(pull, dx);
        dy = (to_y / d).mul_add(pull, dy);
    }

    // Sharks can't pivot on the spot: the turning circle grows with speed.
    let agility = (if hunting { 1.7 + closing } else { 0.8 }).min(sh.speed / (l * 0.22));
    let max_turn = agility * dt;
    let turned = clamp(angle_diff(sh.heading, dy.atan2(dx)), -max_turn, max_turn);
    sh.heading += turned;
    sh.turn = (turned / dt - sh.turn).mul_add((dt * 3.0).min(1.0), sh.turn);
    (hunting, closing)
}

/// Swims shark `sh` forward at a speed set by the chase.
fn swim(sh: &mut Shark, hunting: bool, closing: f32, dt: f32) {
    let cruise = sh.length.mul_add(0.04, 30.0);
    let target_speed = if hunting {
        cruise * closing.mul_add(1.5, 1.45)
    } else {
        cruise
    };
    sh.speed = (target_speed - sh.speed).mul_add((dt * 1.1).min(1.0), sh.speed);
    sh.phase = dt.mul_add(2.0 + sh.speed / 24.0, sh.phase);
    let (sin, cos) = sh.heading.sin_cos();
    sh.nodes[0] = (cos * sh.speed).mul_add(dt, sh.nodes[0]);
    sh.nodes[1] = (sin * sh.speed).mul_add(dt, sh.nodes[1]);
    follow(&mut sh.nodes, sh.heading, sh.length, 0.16);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pond() -> Pond {
        Pond::new(2560.0, 1440.0, 13.0, 19.0, fastrand::Rng::with_seed(3))
    }

    #[test]
    fn seeding_sizes_the_sharks_and_draws_the_small_ones_first() {
        let mut pond = pond();
        let sharks = seed(&DEFAULT_SHARKS, &mut pond);
        // 1440 * 0.44 = 633.6, capped at 430.
        for (shark, want) in sharks.iter().zip([301.0, 331.1, 361.2, 430.0]) {
            assert!(
                (shark.length - want).abs() < 1e-3,
                "{} vs {want}",
                shark.length
            );
        }
    }

    #[test]
    fn sharks_stay_around_the_screen_and_eat() {
        let mut pond = pond();
        let mut sharks = seed(&DEFAULT_SHARKS, &mut pond);
        let alive = |pond: &Pond| pond.fish.iter().filter(|f| f.alive).count();
        let before = alive(&pond);
        let mut eaten = 0;
        for _ in 0..60 * 60 {
            pond.time += 1.0 / 60.0;
            pond.rebuild_hash();
            pond.step_fish(&sharks, 1.0 / 60.0);
            step(&mut sharks, &mut pond, 1.0 / 60.0);
            eaten += pond.eaten.drain(..).count();
            pond.respawn();
            for sh in &sharks {
                let (x, y) = (sh.nodes[0], sh.nodes[1]);
                assert!(
                    x > -sh.length
                        && x < 2560.0 + sh.length
                        && y > -sh.length
                        && y < 1440.0 + sh.length
                );
            }
        }
        assert!(eaten > 0, "a minute without a meal");
        assert!(
            alive(&pond) + 40 >= before,
            "the population holds: {} of {before}",
            alive(&pond)
        );
    }
}
