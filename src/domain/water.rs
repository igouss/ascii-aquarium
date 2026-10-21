//! What both scenes draw under their predators: a wake field that diffuses
//! where things swim and lights small dots, red heat where a fish was eaten,
//! and the fish as minimal dashes, one per cell, glinting on hard turns.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]
#![expect(
    clippy::many_single_char_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::frame::{Frame, Grid};
use super::math::{Rgb, clamp};
use super::pond::{Body, NODES, Pond};

const ALPHA_LEVELS: f32 = 6.0;
const FISH_ALPHAS: [f32; 5] = [0.28, 0.42, 0.58, 0.74, 0.9];
const FIELD_MAX: f32 = 1.6;
/// Wake below this stays dark.
const WAKE_THRESHOLD: f32 = 0.45;

/// The colours of the water and what swims in it, but not the predators.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Rgb,
    /// Fish dash colour; the alpha comes from depth and agitation.
    pub fish: Rgb,
    /// Wake dot colour.
    pub wake: Rgb,
}

pub const DEEP: Palette = Palette {
    bg: [0.0, 0.0, 0.0],
    fish: [230.0, 238.0, 255.0],
    wake: [110.0, 230.0, 255.0],
};

#[derive(Debug)]
pub struct Water {
    grid: Grid,
    energy: Vec<f32>,
    scratch: Vec<f32>,
    heat: Vec<f32>,
    /// Brightness level of the fish drawn in each cell this frame.
    fish: Vec<Option<usize>>,
}

impl Water {
    #[must_use]
    pub fn new(grid: Grid) -> Self {
        let n = grid.len();
        Self {
            grid,
            energy: vec![0.0; n],
            scratch: vec![0.0; n],
            heat: vec![0.0; n],
            fish: vec![None; n],
        }
    }

    fn stamp(field: &mut [f32], grid: Grid, x: f32, y: f32, radius: i32, amount: f32) {
        let c0 = (x / grid.cw).floor() as i32;
        let r0 = (y / grid.ch).floor() as i32;
        for r in r0 - radius..=r0 + radius {
            for c in c0 - radius..=c0 + radius {
                let (Ok(cu), Ok(ru)) = (usize::try_from(c), usize::try_from(r)) else {
                    continue;
                };
                if cu >= grid.cols || ru >= grid.rows {
                    continue;
                }
                let d = ((c - c0) as f32).hypot((r - r0) as f32 * 1.4);
                let radius = radius as f32;
                if d > radius + 0.5 {
                    continue;
                }
                let i = ru * grid.cols + cu;
                field[i] = amount
                    .mul_add(1.0 - d / (radius + 1.0), field[i])
                    .min(FIELD_MAX);
            }
        }
    }

    /// Lights up where fish were eaten, and where everything swims.
    pub fn stir(&mut self, pond: &mut Pond, predators: &[impl Body], speed: impl Fn(usize) -> f32) {
        let grid = self.grid;
        for e in pond.eaten.drain(..) {
            Self::stamp(&mut self.heat, grid, e.x, e.y, 2, 1.0);
            Self::stamp(&mut self.energy, grid, e.x, e.y, 3, 0.9);
        }
        for f in pond.fish.iter().filter(|f| f.alive) {
            if let Some(i) = grid.index(f.x, f.y) {
                self.energy[i] = (self.energy[i] + f.alarm.mul_add(0.03, 0.012)).min(FIELD_MAX);
            }
        }
        for (p, body) in predators.iter().enumerate() {
            // The tail sweeps the most water.
            let nodes = body.nodes();
            for k in NODES - 6..NODES {
                if let Some(i) = grid.index(nodes[k * 2], nodes[k * 2 + 1]) {
                    self.energy[i] =
                        (self.energy[i] + speed(p).mul_add(0.0012, 0.05)).min(FIELD_MAX);
                }
            }
        }
    }

    /// Blur and decay, so wakes spread out like ripples and fade.
    pub fn diffuse(&mut self, dt: f32) {
        let Grid { cols, rows, .. } = self.grid;
        let decay = (-dt * 1.5).exp();
        let e = &self.energy;
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                let l = if c > 0 { e[i - 1] } else { e[i] };
                let rt = if c + 1 < cols { e[i + 1] } else { e[i] };
                let u = if r > 0 { e[i - cols] } else { e[i] };
                let d = if r + 1 < rows { e[i + cols] } else { e[i] };
                self.scratch[i] = e[i].mul_add(4.0, l + rt + u + d) * 0.125 * decay;
            }
        }
        std::mem::swap(&mut self.energy, &mut self.scratch);
        let heat_decay = (-dt * 2.2).exp();
        for h in &mut self.heat {
            *h *= heat_decay;
        }
    }

    /// Snaps each fish to a cell, not one `covered` by a predator, and picks
    /// its brightness: far fish are dim, near fish brighter, and turning or
    /// panicking fish glint. Nearer fish claim a cell first.
    pub fn place_fish(&mut self, pond: &Pond, covered: impl Fn(usize) -> bool) {
        self.fish.fill(None);
        for near in [true, false] {
            for f in pond
                .fish
                .iter()
                .filter(|f| f.alive && (f.depth > 0.78) == near)
            {
                let Some(i) = self.grid.index(f.x, f.y) else {
                    continue;
                };
                if self.fish[i].is_some() || covered(i) {
                    continue;
                }
                let glint = clamp(f.turn.abs() / 3.5, 0.0, 1.0);
                let v = 0.3f32.mul_add(glint.max(f.alarm), 0.3 + 0.4 * (f.depth - 0.55) / 0.45);
                let levels = FISH_ALPHAS.len() as f32;
                self.fish[i] = Some(clamp((v * levels).floor(), 0.0, levels - 1.0) as usize);
            }
        }
    }

    /// Paints wake dots, heat and fish over a cleared frame. Wake skips
    /// `covered` cells and fish.
    pub fn paint(&self, frame: &mut Frame, palette: &Palette, covered: impl Fn(usize) -> bool) {
        for i in 0..self.grid.len() {
            let e = self.energy[i];
            if e > WAKE_THRESHOLD && !covered(i) && self.fish[i].is_none() {
                let level = clamp(
                    ((e - WAKE_THRESHOLD) * 5.0).floor(),
                    0.0,
                    ALPHA_LEVELS - 1.0,
                );
                frame.glyph(i, '·', palette.wake, (level + 1.0) / ALPHA_LEVELS);
            }
            let h = self.heat[i];
            if h >= 0.08 {
                let glyph = if h > 0.6 { '*' } else { '·' };
                if h > 0.5 {
                    frame.glyph(i, glyph, [255.0, 120.0, 60.0], h);
                } else {
                    frame.glyph(i, glyph, [220.0, 40.0, 40.0], h * 1.6);
                }
            }
            if let Some(level) = self.fish[i] {
                frame.glyph(i, '-', palette.fish, FISH_ALPHAS[level]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::frame::Color;

    const GRID: Grid = Grid {
        cols: 20,
        rows: 10,
        cw: 10.0,
        ch: 15.0,
    };

    #[test]
    fn diffusion_spreads_and_fades() {
        let mut water = Water::new(GRID);
        let centre = 5 * 20 + 10;
        water.energy[centre] = 1.0;
        water.diffuse(1.0 / 60.0);
        let decay = (-1.5f32 / 60.0).exp();
        assert!(0.5f32.mul_add(-decay, water.energy[centre]).abs() < 1e-6);
        assert!(0.125f32.mul_add(-decay, water.energy[centre + 1]).abs() < 1e-6);
        assert!(water.energy[centre + 2].abs() < f32::EPSILON);
        let total: f32 = water.energy.iter().sum();
        assert!(
            (total - decay).abs() < 1e-5,
            "blur conserves, decay fades: {total}"
        );
    }

    #[test]
    fn strong_wake_lights_a_dot_but_not_under_a_predator() {
        let mut water = Water::new(GRID);
        water.energy[0] = 1.0;
        water.energy[1] = 1.0;
        water.energy[2] = WAKE_THRESHOLD;
        let mut frame = Frame::new(GRID.cols, GRID.rows, Color::default());
        water.paint(&mut frame, &DEEP, |i| i == 1);
        assert_eq!(frame.cells[0].glyph, '·');
        assert_eq!(frame.cells[1].glyph, ' ');
        assert_eq!(frame.cells[2].glyph, ' ');
    }

    #[test]
    fn heat_glows_hot_then_cools() {
        let mut water = Water::new(GRID);
        water.heat[0] = 1.0;
        water.heat[1] = 0.3;
        water.heat[2] = 0.07;
        let mut frame = Frame::new(GRID.cols, GRID.rows, Color::default());
        water.paint(&mut frame, &DEEP, |_| false);
        assert_eq!(frame.cells[0].glyph, '*');
        assert_eq!(frame.cells[0].fg, Color(255, 120, 60));
        assert_eq!(frame.cells[1].glyph, '·');
        assert_eq!(frame.cells[2].glyph, ' ');
    }
}
