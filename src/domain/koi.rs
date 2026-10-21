//! Koi, after Ahmet Dedeler's ascii-koi (built on ascii-sharks): the shark's
//! steering on a looser, slower spine so they glide and curl, each patrolling
//! its own ring, turning early at the edges, and never swimming through one
//! another.

#![expect(
    clippy::cast_precision_loss,
    reason = "ids, counts and node indices are far below 2^23"
)]
#![expect(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::math::{TAU, angle_diff, clamp, rand};
use super::pond::{Body, Fish, Hunt, NODES, Pond, nonzero};
use super::swim::{clear_middle, follow, pursue, straight_spine, wander};

/// Koi varieties, all in the classic warm palette. Kohaku: coral red patches
/// on white. Orange: orange with cream saddles. Benigoi: solid red. Tancho:
/// white with one red crown on the head. Butterfly: orange and white with
/// long flowing fins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KoiKind {
    Kohaku,
    Orange,
    Benigoi,
    Tancho,
    Butterfly,
}

impl KoiKind {
    pub const ALL: [Self; 5] = [
        Self::Kohaku,
        Self::Orange,
        Self::Benigoi,
        Self::Tancho,
        Self::Butterfly,
    ];

    /// The name upstream's `?koi=` takes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kohaku => "kohaku",
            Self::Orange => "orange",
            Self::Benigoi => "benigoi",
            Self::Tancho => "tancho",
            Self::Butterfly => "butterfly",
        }
    }

    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Upstream's `?koi=` cast: the first is biggest, the rest step down.
#[must_use]
pub fn cast(kinds: &[KoiKind]) -> Vec<KoiSpec> {
    kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| KoiSpec {
            kind,
            size: (i as f32).mul_add(-0.07, 1.0),
        })
        .collect()
}

/// Which koi to put in the water; size is relative to the biggest.
#[derive(Debug, Clone, Copy)]
pub struct KoiSpec {
    pub kind: KoiKind,
    pub size: f32,
}

/// The upstream pond: four koi at different sizes and depths.
pub const DEFAULT_KOI: [KoiSpec; 4] = [
    KoiSpec {
        kind: KoiKind::Kohaku,
        size: 1.0,
    },
    KoiSpec {
        kind: KoiKind::Orange,
        size: 0.88,
    },
    KoiSpec {
        kind: KoiKind::Butterfly,
        size: 0.82,
    },
    KoiSpec {
        kind: KoiKind::Kohaku,
        size: 0.76,
    },
];

/// Half-width a koi keeps clear around its spine, as a fraction of length
/// (body plus fins).
const BODY_CLEARANCE: f32 = 0.12;

#[derive(Debug, Clone)]
pub struct Koi {
    pub id: i32,
    pub kind: KoiKind,
    pub length: f32,
    /// Spine positions, snout first: [x0, y0, x1, y1, ...].
    pub nodes: [f32; NODES * 2],
    pub heading: f32,
    pub speed: f32,
    /// Tail-beat phase; drives the body wave in the renderer.
    pub phase: f32,
    /// Smoothed angular velocity (rad/s); the koi banks its fins into turns.
    pub turn: f32,
    hunger: f32,
    wander: f32,
    /// Which patrol ring this koi swims, 0 (inner) to 1 (outer).
    ring: f32,
    target: Option<usize>,
    retarget_at: f32,
    /// 0 far and dim to 1 near and bright.
    pub depth: f32,
}

impl Body for Koi {
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

/// Patrol rings spread evenly between just outside the middle of the screen
/// and the edge-turn margin, so each koi keeps its own lane.
fn ring_radius(pond: &Pond, ring: f32, l: f32) -> f32 {
    let min_side = pond.width.min(pond.height);
    let inner = min_side * 0.16;
    let outer = inner.max(l.mul_add(-0.32, min_side / 2.0) - 20.0);
    (outer - inner).mul_add(ring, inner)
}

/// Clearance around node `k` of a koi of length `l`: wide at the head and
/// fins, narrowing to the tail.
fn clearance(l: f32, k: usize) -> f32 {
    l * BODY_CLEARANCE * (k as f32 / (NODES - 1) as f32).mul_add(-0.9, 1.3)
}

/// Pond koi circle together the same way, each on its own ring, so they
/// follow one another instead of swimming through each other.
pub fn seed(specs: &[KoiSpec], pond: &mut Pond) -> Vec<Koi> {
    let (w, h) = (pond.width, pond.height);
    let min_side = w.min(h);
    let base_length = clamp(min_side * 0.46, 170.0, 440.0);
    let count = specs.len();
    let mut koi: Vec<Koi> = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            let length = base_length * spec.size;
            let ring = if count > 1 {
                (i as f32 * 0.618) % 1.0
            } else {
                0.5
            };
            let orbit_radius = ring_radius(pond, ring, length);
            let rng = &mut pond.rng;
            let a = (i as f32 / count as f32).mul_add(TAU, rand(rng, -0.2, 0.2));
            let x = (a.cos() * orbit_radius * (w / min_side)).mul_add(0.9, w / 2.0);
            let y = a.sin().mul_add(orbit_radius, h / 2.0);
            let heading = a + std::f32::consts::FRAC_PI_2;
            Koi {
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
                ring,
                target: None,
                retarget_at: 0.0,
                depth: spec.size,
            }
        })
        .collect();
    // Smaller (farther) koi draw first so the big one swims over them.
    koi.sort_by(|a, b| a.length.total_cmp(&b.length));
    koi
}

pub fn step(koi: &mut [Koi], pond: &mut Pond, dt: f32) {
    for i in 0..koi.len() {
        let mut k = koi[i].clone();
        let (hunting, closing, crowded) = steer(&mut k, i, koi, pond, dt);
        swim(&mut k, hunting, closing, crowded, dt);
        // Feed.
        if k.hunger > 0.15 {
            let eaten = pond.feed((k.nodes[0], k.nodes[1]), k.length.mul_add(0.05, 8.0));
            k.hunger = (eaten as f32).mul_add(-0.16, k.hunger).max(0.0);
        }
        koi[i] = k;
    }
    separate_bodies(koi);
    keep_inside(koi, pond);
}

/// Turns koi `sh`, number `i` of `school`, toward where it wants to go.
/// Returns whether it is hunting, how close the chase is, and how crowded
/// it is by the others.
fn steer(sh: &mut Koi, i: usize, school: &[Koi], pond: &mut Pond, dt: f32) -> (bool, f32, f32) {
    let (w, h, time) = (pond.width, pond.height, pond.time);
    let l = sh.length;
    let head = (sh.nodes[0], sh.nodes[1]);
    sh.hunger = dt.mul_add(0.025, sh.hunger).min(1.0);

    // Momentum, plus a wander.
    let (mut dy, mut dx) = sh.heading.sin_cos();
    let (wx, wy) = wander(pond, sh.heading, &mut sh.wander, dt);
    dx += wx;
    dy += wy;

    // Food too close to the edge isn't worth it: chasing it would carry the
    // koi out of view, so a chase is also given up once the fish slips there.
    let edge_m = l.mul_add(0.35, 30.0);
    if time >= sh.retarget_at {
        sh.target = pond.find_prey(Hunt {
            head,
            heading: sh.heading,
            range: l.mul_add(0.9, 160.0),
            current: sh.target,
            margin: Some(edge_m),
        });
        sh.retarget_at = time + 0.45;
    }
    let reachable = |t: &Fish| t.x > edge_m && t.x < w - edge_m && t.y > edge_m && t.y < h - edge_m;
    let chase = sh
        .target
        .map(|t| &pond.fish[t])
        .filter(|t| t.alive && reachable(t) && sh.hunger > 0.3)
        .map(|t| pursue(t, head, l, sh.hunger));
    // The pull of a chase is applied after the avoidance check, so it can
    // ease off when another koi is in the way.
    let ((hunt_x, hunt_y), closing) = chase.unwrap_or(((0.0, 0.0), 0.0));
    let hunting = chase.is_some();
    if !hunting {
        // Patrol: a lazy circuit around the middle. The ring never reaches the
        // edge-turn margin, so patrolling and staying in view don't fight.
        let min_side = w.min(h);
        let rx = (head.0 - w / 2.0) / (w / min_side);
        let ry = head.1 - h / 2.0;
        let r = nonzero(rx.hypot(ry), 1.0);
        let radius = ring_radius(pond, sh.ring, l)
            * 0.08f32.mul_add(time.mul_add(0.05, sh.id as f32).sin(), 1.0);
        let radial = clamp((radius - r) / radius, -1.0, 1.0);
        dx = ((rx / r) * radial).mul_add(0.9, -ry / r).mul_add(0.55, dx);
        dy = ((ry / r) * radial).mul_add(0.9, rx / r).mul_add(0.55, dy);
    }

    let ((ax, ay), crowded) = give_room(sh, i, school);
    // Food can wait: chasing a fish never overrides not swimming into a friend.
    dx += hunt_x.mul_add(0.85f32.mul_add(-crowded, 1.0), ax);
    dy += hunt_y.mul_add(0.85f32.mul_add(-crowded, 1.0), ay);
    let (mx, my) = clear_middle(pond, head, hunting);
    let ((vx, vy), walled) = stay_in_view(sh, pond);
    dx += mx + vx;
    dy += my + vy;

    // Koi turn tighter than sharks, but still can't pivot on the spot. A fish
    // about to bump another one, or the edge, is allowed to turn hard.
    let agility = (if hunting { 1.9 + closing } else { 1.0 })
        .min(sh.speed / (l * 0.18))
        .max(crowded * 1.7)
        .max(walled * 1.6);
    let max_turn = agility * dt;
    let turned = clamp(angle_diff(sh.heading, dy.atan2(dx)), -max_turn, max_turn);
    sh.heading += turned;
    sh.turn = (turned / dt - sh.turn).mul_add((dt * 3.0).min(1.0), sh.turn);
    (hunting, closing, crowded)
}

/// Gives the other koi room: checks the snout and a point well ahead of it
/// against the others' bodies, to turn away early. The pushes are summed and
/// capped, so a near miss means a calm bend, never a snap. Returns the push
/// and how crowded the koi is (0 clear to 1 touching).
fn give_room(sh: &Koi, i: usize, school: &[Koi]) -> ((f32, f32), f32) {
    let l = sh.length;
    let head = (sh.nodes[0], sh.nodes[1]);
    let (sin, cos) = sh.heading.sin_cos();
    let ahead = (
        cos.mul_add(sh.speed * 2.4, head.0),
        sin.mul_add(sh.speed * 2.4, head.1),
    );
    let mut crowded = 0.0f32;
    let (mut avoid_x, mut avoid_y) = (0.0, 0.0);
    for other in school
        .iter()
        .enumerate()
        .filter(|&(j, _)| j != i)
        .map(|(_, o)| o)
    {
        let range = ((l + other.length) * BODY_CLEARANCE).mul_add(3.0, 50.0);
        for k in (0..NODES).step_by(2) {
            let (qx, qy) = (other.nodes[k * 2], other.nodes[k * 2 + 1]);
            for (px, py) in [head, ahead] {
                let (ox, oy) = (px - qx, py - qy);
                let d = ox.hypot(oy);
                if d >= range || d < 1e-3 {
                    continue;
                }
                let near = 1.0 - d / range;
                avoid_x = (ox / d).mul_add(near, avoid_x);
                avoid_y = (oy / d).mul_add(near, avoid_y);
                crowded = crowded.max(near);
            }
        }
    }
    let avoid = avoid_x.hypot(avoid_y);
    if avoid <= 1e-3 {
        return ((0.0, 0.0), crowded);
    }
    let push = (avoid * 1.4).min(3.0);
    ((avoid_x / avoid * push, avoid_y / avoid * push), crowded)
}

/// Stays in view: starts turning well before the edge, judged from the
/// snout, from where it is about to be, and from the mid-body and tail (with
/// smaller margins), since those swing wide of the snout's path. Returns the
/// push and how near the nearest edge is (0 clear to 1 at it).
fn stay_in_view(sh: &Koi, pond: &Pond) -> ((f32, f32), f32) {
    let (w, h, l) = (pond.width, pond.height, sh.length);
    let margin = l.mul_add(0.45, 30.0);
    let (sin, cos) = sh.heading.sin_cos();
    let look = (
        cos.mul_add(sh.speed * 1.2, sh.nodes[0]),
        sin.mul_add(sh.speed * 1.2, sh.nodes[1]),
    );
    let mid = (NODES * 2 / 5) * 2;
    let back = (NODES * 3 / 4) * 2;
    let probes = [
        (sh.nodes[0], sh.nodes[1], margin),
        (look.0, look.1, margin),
        (sh.nodes[mid], sh.nodes[mid + 1], l.mul_add(0.25, 20.0)),
        (sh.nodes[back], sh.nodes[back + 1], l.mul_add(0.2, 16.0)),
    ];
    let (mut dx, mut dy, mut walled) = (0.0, 0.0, 0.0f32);
    for (px, py, pm) in probes {
        for (dist, nx, ny) in [
            (px, 1.0, 0.0),
            (w - px, -1.0, 0.0),
            (py, 0.0, 1.0),
            (h - py, 0.0, -1.0),
        ] {
            if dist >= pm {
                continue;
            }
            let near = clamp(1.0 - dist / pm, 0.0, 1.5);
            dx = (nx * near * near).mul_add(6.0, dx);
            dy = (ny * near * near).mul_add(6.0, dy);
            walled = walled.max(near.min(1.0));
        }
    }
    ((dx, dy), walled)
}

/// Swims koi `sh` forward: a lazy drift, a burst only for a nearby snack,
/// slower when crowded.
fn swim(sh: &mut Koi, hunting: bool, closing: f32, crowded: f32, dt: f32) {
    let cruise = sh.length.mul_add(0.035, 24.0);
    let chase = if hunting {
        cruise * closing.mul_add(1.2, 1.3)
    } else {
        cruise
    };
    let target_speed = chase * 0.25f32.mul_add(-crowded, 1.0);
    sh.speed = (target_speed - sh.speed).mul_add((dt * 0.9).min(1.0), sh.speed);
    // Slow tail beats: a lazy sway, faster only in a dash.
    sh.phase = dt.mul_add(1.1 + sh.speed / 40.0, sh.phase);
    let (sin, cos) = sh.heading.sin_cos();
    sh.nodes[0] = (cos * sh.speed).mul_add(dt, sh.nodes[0]);
    sh.nodes[1] = (sin * sh.speed).mul_add(dt, sh.nodes[1]);
    // Koi are bendier than sharks, so their turns read as soft S-curves.
    follow(&mut sh.nodes, sh.heading, sh.length, 0.24);
}

/// Last resort after steering: wherever two koi still touch, push the
/// touching spine nodes apart a little each step, so one never swims over
/// another.
fn separate_bodies(koi: &mut [Koi]) {
    for i in 0..koi.len() {
        let (head, tail) = koi.split_at_mut(i + 1);
        let a = &mut head[i];
        for b in tail {
            let reach = (a.length + b.length) * (1.0 + BODY_CLEARANCE);
            if (a.nodes[0] - b.nodes[0]).hypot(a.nodes[1] - b.nodes[1]) > reach {
                continue;
            }
            // The first few nodes are left alone: moving the snout would snap the heading.
            for p in 3..NODES {
                let wa = clearance(a.length, p);
                for q in 3..NODES {
                    let min = wa + clearance(b.length, q);
                    let ox = b.nodes[q * 2] - a.nodes[p * 2];
                    let oy = b.nodes[q * 2 + 1] - a.nodes[p * 2 + 1];
                    let d2 = ox.mul_add(ox, oy * oy);
                    if d2 >= min * min || d2 < 1e-6 {
                        continue;
                    }
                    let d = d2.sqrt();
                    // A light nudge each step (12% of the overlap per side):
                    // bodies ease apart over a few frames instead of jumping.
                    let k = ((min - d) / d) * 0.12;
                    a.nodes[p * 2] = f32::mul_add(ox, -k, a.nodes[p * 2]);
                    a.nodes[p * 2 + 1] = f32::mul_add(oy, -k, a.nodes[p * 2 + 1]);
                    b.nodes[q * 2] = f32::mul_add(ox, k, b.nodes[q * 2]);
                    b.nodes[q * 2 + 1] = f32::mul_add(oy, k, b.nodes[q * 2 + 1]);
                }
            }
        }
    }
}

/// Backstop for the edge steering: if a body still swings out mid-turn,
/// nudge those spine nodes back so the body and fins stay on screen.
fn keep_inside(koi: &mut [Koi], pond: &Pond) {
    for sh in koi {
        for k in 0..NODES {
            let pad = clearance(sh.length, k) + 4.0;
            // The head eases back (moving it hard would snap the heading); the
            // body is held firmly inside.
            let ease = if k < 3 { 0.3 } else { 1.0 };
            let (x, y) = (sh.nodes[k * 2], sh.nodes[k * 2 + 1]);
            sh.nodes[k * 2] = (clamp(x, pad, pond.width - pad) - x).mul_add(ease, sh.nodes[k * 2]);
            sh.nodes[k * 2 + 1] =
                (clamp(y, pad, pond.height - pad) - y).mul_add(ease, sh.nodes[k * 2 + 1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pond() -> Pond {
        Pond::new(2560.0, 1440.0, 11.0, 16.0, fastrand::Rng::with_seed(9))
    }

    #[test]
    fn each_koi_gets_its_own_ring() {
        let mut pond = pond();
        let koi = seed(&DEFAULT_KOI, &mut pond);
        let mut rings: Vec<f32> = koi.iter().map(|k| k.ring).collect();
        rings.sort_by(f32::total_cmp);
        assert!(rings.windows(2).all(|w| w[1] - w[0] > 0.1), "{rings:?}");
        assert!(koi.windows(2).all(|w| w[0].length <= w[1].length));
    }

    #[test]
    fn koi_stay_on_screen_and_apart() {
        let mut pond = pond();
        let mut koi = seed(&DEFAULT_KOI, &mut pond);
        for frame in 0..60 * 90 {
            pond.time += 1.0 / 60.0;
            pond.rebuild_hash();
            pond.step_fish(&koi, 1.0 / 60.0);
            step(&mut koi, &mut pond, 1.0 / 60.0);
            pond.eaten.clear();
            pond.respawn();
            if frame < 60 * 5 {
                continue;
            }
            for k in &koi {
                for n in 3..NODES {
                    let (x, y) = (k.nodes[n * 2], k.nodes[n * 2 + 1]);
                    assert!(
                        x > 0.0 && x < 2560.0 && y > 0.0 && y < 1440.0,
                        "frame {frame}: ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn touching_bodies_are_nudged_apart() {
        let mut pond = pond();
        let mut koi = seed(&DEFAULT_KOI[..2], &mut pond);
        let nodes = straight_spine(1000.0, 700.0, 0.0, koi[0].length);
        koi[0].nodes = nodes;
        koi[1].nodes = straight_spine(1000.0, 705.0, 0.0, koi[1].length);
        let gap = |koi: &[Koi]| koi[1].nodes[21] - koi[0].nodes[21];
        let before = gap(&koi);
        separate_bodies(&mut koi);
        assert!(gap(&koi) > before, "{} -> {}", before, gap(&koi));
    }
}
