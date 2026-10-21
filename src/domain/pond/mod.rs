//! The water and the bait fish in it: Reynolds boids (separation, alignment,
//! cohesion) in Couzin-style zones, with a blind spot behind each fish. Alarm
//! spreads from fish to fish through the school (a "wave of agitation"), so a
//! whole school bolts even when only its edge can see the predator. Fleeing
//! fish break sideways around the predator's path, which gives the fountain
//! effect seen in real bait balls. Eaten fish come back later as new schools
//! swimming in from the edges, which keeps the population steady.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]

use super::math::{clamp, rand, rand_int, rand_sign};
use super::species::{SPECIES, pick_species};

mod prey;
mod school;

pub use prey::Hunt;

/// Spine nodes per predator, snout first.
pub const NODES: usize = 20;

/// A predator as the fish see it.
pub trait Body {
    /// Spine positions, snout first: [x0, y0, x1, y1, ...].
    fn nodes(&self) -> &[f32; NODES * 2];
    fn length(&self) -> f32;
    fn heading(&self) -> f32;
}

#[derive(Debug, Clone)]
pub struct Fish {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub species: usize,
    /// 0 calm to 1 panicking. Spreads to neighbours.
    pub alarm: f32,
    pub alive: bool,
    pub respawn_at: f32,
    /// Smoothed angular velocity (rad/s); hard turns flash silver.
    pub turn: f32,
    /// 0.55 far to 1 near.
    pub depth: f32,
    /// Rotation sense shared by a school when it mills into a bait ball.
    pub mill_dir: f32,
}

/// Where a fish was eaten; the renderer lights it up.
#[derive(Debug, Clone, Copy)]
pub struct Eaten {
    pub x: f32,
    pub y: f32,
}

const HASH_PAD: i32 = 2;

#[derive(Debug)]
pub struct Pond {
    pub width: f32,
    pub height: f32,
    pub time: f32,
    pub fish: Vec<Fish>,
    pub eaten: Vec<Eaten>,
    pub rng: fastrand::Rng,
    /// Fish neighbourhood radius; the spatial hash bucket size matches it.
    pub perception: f32,
    sep_x: [f32; SPECIES.len()],
    sep_y: [f32; SPECIES.len()],
    foreign_sep: f32,
    bucket_cols: usize,
    bucket_rows: usize,
    bucket_head: Vec<Option<usize>>,
    bucket_count: Vec<u16>,
    bucket_next: Vec<Option<usize>>,
    last_spawn: f32,
}

impl Pond {
    /// A pond `width` by `height` pixels on a grid of `cell_w` by `cell_h`
    /// cells, stocked with fish.
    #[must_use]
    pub fn new(width: f32, height: f32, cell_w: f32, cell_h: f32, rng: fastrand::Rng) -> Self {
        let mut pond = Self {
            width,
            height,
            time: 0.0,
            fish: Vec::new(),
            eaten: Vec::new(),
            rng,
            // Spacing is set in grid cells so every fish lands in its own cell.
            perception: (cell_h * 3.0).max(cell_w * 5.5),
            sep_x: SPECIES.map(|sp| sp.spacing.0 * cell_w),
            sep_y: SPECIES.map(|sp| sp.spacing.1 * cell_h),
            foreign_sep: cell_w * 3.0,
            bucket_cols: 0,
            bucket_rows: 0,
            bucket_head: Vec::new(),
            bucket_count: Vec::new(),
            bucket_next: Vec::new(),
            last_spawn: 0.0,
        };
        pond.stock();
        pond
    }

    fn stock(&mut self) {
        let (w, h) = (self.width, self.height);
        let total = clamp((w * h / 4400.0).round(), 85.0, 340.0) as usize;
        while self.fish.len() < total {
            let species = pick_species(&mut self.rng);
            let sp = SPECIES[species];
            let drawn = rand_int(&mut self.rng, sp.school_size.0, sp.school_size.1);
            let size = usize::try_from(drawn)
                .unwrap_or(0)
                .min(total - self.fish.len())
                .max(1);
            let mill_dir = rand_sign(&mut self.rng);
            let cx = rand(&mut self.rng, w * 0.06, w * 0.94);
            let cy = rand(&mut self.rng, h * 0.08, h * 0.92);
            // Mostly level headings: fish cruise sideways far more than up or down.
            let base = if self.rng.bool() {
                0.0
            } else {
                std::f32::consts::PI
            };
            let dir = base + rand(&mut self.rng, -0.5, 0.5);
            let spread = (size as f32).sqrt();
            for _ in 0..size {
                let x = (rand(&mut self.rng, -1.0, 1.0) * self.sep_x[species]).mul_add(spread, cx);
                let y = (rand(&mut self.rng, -1.0, 1.0) * self.sep_y[species] * spread)
                    .mul_add(0.6, cy);
                let depth = rand(&mut self.rng, 0.55, 1.0);
                self.fish.push(Fish {
                    x,
                    y,
                    vx: dir.cos() * sp.cruise,
                    vy: dir.sin() * sp.cruise,
                    species,
                    alarm: 0.0,
                    alive: true,
                    respawn_at: 0.0,
                    turn: 0.0,
                    depth,
                    mill_dir,
                });
            }
        }
    }

    pub fn rebuild_hash(&mut self) {
        let size = self.perception;
        let pad = HASH_PAD as usize;
        self.bucket_cols = (self.width / size).ceil() as usize + pad * 2;
        self.bucket_rows = (self.height / size).ceil() as usize + pad * 2;
        let n = self.bucket_cols * self.bucket_rows;
        self.bucket_head.clear();
        self.bucket_head.resize(n, None);
        self.bucket_count.clear();
        self.bucket_count.resize(n, 0);
        self.bucket_next.clear();
        self.bucket_next.resize(self.fish.len(), None);
        for i in 0..self.fish.len() {
            let f = &self.fish[i];
            if !f.alive {
                continue;
            }
            let b = self.bucket_of(f.x, f.y);
            self.bucket_next[i] = self.bucket_head[b];
            self.bucket_head[b] = Some(i);
            self.bucket_count[b] = self.bucket_count[b].saturating_add(1);
        }
    }

    fn bucket_cell(&self, x: f32, y: f32) -> (usize, usize) {
        let size = self.perception;
        let c = clamp(
            (x / size).floor() + HASH_PAD as f32,
            0.0,
            (self.bucket_cols - 1) as f32,
        );
        let r = clamp(
            (y / size).floor() + HASH_PAD as f32,
            0.0,
            (self.bucket_rows - 1) as f32,
        );
        (c as usize, r as usize)
    }

    fn bucket_of(&self, x: f32, y: f32) -> usize {
        let (c, r) = self.bucket_cell(x, y);
        r * self.bucket_cols + c
    }
}

/// `v`, or `fallback` when `v` is zero (JavaScript's `v || fallback`).
#[must_use]
pub fn nonzero(v: f32, fallback: f32) -> f32 {
    if v == 0.0 { fallback } else { v }
}

#[cfg(test)]
pub(super) fn test_pond() -> Pond {
    Pond::new(1600.0, 900.0, 13.0, 19.0, fastrand::Rng::with_seed(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stocking_scales_with_area_within_bounds() {
        assert_eq!(test_pond().fish.len(), 327);
        let small = Pond::new(100.0, 100.0, 13.0, 19.0, fastrand::Rng::with_seed(1));
        assert_eq!(small.fish.len(), 85);
        let huge = Pond::new(5000.0, 3000.0, 13.0, 19.0, fastrand::Rng::with_seed(1));
        assert_eq!(huge.fish.len(), 340);
    }
}
