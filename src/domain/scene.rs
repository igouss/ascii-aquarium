//! A whole scene: the pond, its predators, and how a frame of it is drawn.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "cell counts of a window, after a ceil and a max of 1"
)]

use super::frame::{Color, Frame, Grid};
use super::koi::{self, Koi, KoiSpec};
use super::koi_skin::{KoiSkins, Theme};
use super::pond::Pond;
use super::shark::{self, Shark, SharkSpec};
use super::shark_skin::SharkSkins;
use super::water::{DEEP, Palette, Water};

/// The simulation's fixed time step.
pub const STEP: f32 = 1.0 / 60.0;

#[derive(Debug)]
enum Cast {
    Sharks(Vec<Shark>, SharkSkins),
    Koi(Vec<Koi>, KoiSkins),
}

/// What a scene holds.
#[derive(Debug, Clone)]
pub enum Kind {
    Sharks(Vec<SharkSpec>),
    Koi(Vec<KoiSpec>, Theme),
}

impl Kind {
    /// The cell grid on a window `width` by `height` pixels, sized the way the
    /// originals size theirs: the koi need a finer grid for fins and tails.
    #[must_use]
    pub fn grid(&self, width: f32, height: f32) -> Grid {
        let (per, min, max) = match self {
            Self::Sharks(_) => (124.0, 8.0, 13.0),
            Self::Koi(..) => (150.0, 7.0, 11.0),
        };
        let cw = (width / per).round().clamp(min, max);
        let ch = (cw * 1.45).round();
        Grid {
            cols: (width / cw).ceil().max(1.0) as usize,
            rows: (height / ch).ceil().max(1.0) as usize,
            cw,
            ch,
        }
    }
}

#[derive(Debug)]
pub struct Scene {
    grid: Grid,
    pond: Pond,
    water: Water,
    palette: Palette,
    cast: Cast,
}

impl Scene {
    /// A fresh scene of `kind` filling `grid`.
    #[must_use]
    pub fn new(kind: &Kind, grid: Grid, rng: fastrand::Rng) -> Self {
        let mut pond = Pond::new(grid.width(), grid.height(), grid.cw, grid.ch, rng);
        let (cast, palette) = match kind {
            Kind::Sharks(specs) => (
                Cast::Sharks(shark::seed(specs, &mut pond), SharkSkins::new(grid)),
                DEEP,
            ),
            Kind::Koi(specs, theme) => (
                Cast::Koi(koi::seed(specs, &mut pond), KoiSkins::new(grid, *theme)),
                theme.palette(),
            ),
        };
        Self {
            grid,
            pond,
            water: Water::new(grid),
            palette,
            cast,
        }
    }

    #[must_use]
    pub const fn grid(&self) -> Grid {
        self.grid
    }

    /// Advances the simulation by one [`STEP`].
    pub fn step(&mut self) {
        let pond = &mut self.pond;
        pond.time += STEP;
        pond.rebuild_hash();
        match &mut self.cast {
            Cast::Sharks(sharks, _) => {
                pond.step_fish(sharks, STEP);
                shark::step(sharks, pond, STEP);
            }
            Cast::Koi(koi, _) => {
                pond.step_fish(koi, STEP);
                koi::step(koi, pond, STEP);
            }
        }
        pond.respawn();
    }

    /// Draws the scene into `frame`; `dt` is the time since the last frame,
    /// for the wake to spread and fade by.
    pub fn render(&mut self, frame: &mut Frame, dt: f32) {
        let t = self.pond.time;
        let (pond, water) = (&mut self.pond, &mut self.water);
        frame.clear(Color::from_rgb(self.palette.bg));
        match &mut self.cast {
            Cast::Sharks(sharks, skins) => {
                skins.rasterize(sharks, t);
                water.stir(pond, sharks, |i| sharks[i].speed);
                water.diffuse(dt);
                water.place_fish(pond, |i| skins.covers(i));
                water.paint(frame, &self.palette, |i| skins.covers(i));
                skins.paint(frame);
            }
            Cast::Koi(koi, skins) => {
                skins.rasterize(koi, t);
                water.stir(pond, koi, |i| koi[i].speed);
                water.diffuse(dt);
                water.place_fish(pond, |i| skins.covers(i));
                water.paint(frame, &self.palette, |i| skins.covers(i));
                skins.paint(frame, t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::koi::{DEFAULT_KOI, KoiKind};
    use crate::domain::shark::{DEFAULT_SHARKS, SharkKind};

    #[test]
    fn every_scene_runs_and_draws_something() {
        let grid = Grid {
            cols: 160,
            rows: 60,
            cw: 13.0,
            ch: 19.0,
        };
        let kinds = [
            Kind::Sharks(DEFAULT_SHARKS.to_vec()),
            Kind::Sharks(shark::cast(&SharkKind::ALL)),
            Kind::Koi(DEFAULT_KOI.to_vec(), Theme::Dark),
            Kind::Koi(koi::cast(&KoiKind::ALL), Theme::Light),
        ];
        for kind in kinds {
            let mut scene = Scene::new(&kind, grid, fastrand::Rng::with_seed(2));
            let mut frame = Frame::new(grid.cols, grid.rows, Color::default());
            for _ in 0..120 {
                scene.step();
                scene.render(&mut frame, STEP);
            }
            let bg = Color::from_rgb(kind_bg(&kind));
            let fish = frame.cells.iter().filter(|c| c.glyph == '-').count();
            let blocks = frame.cells.iter().filter(|c| c.bg != bg).count();
            assert!(fish > 50, "{kind:?}: {fish} fish");
            assert!(blocks > 100, "{kind:?}: {blocks} predator cells");
        }
    }

    #[test]
    fn grids_follow_the_originals() {
        let sharks = Kind::Sharks(DEFAULT_SHARKS.to_vec()).grid(2560.0, 1440.0);
        assert_eq!(
            sharks,
            Grid {
                cols: 197,
                rows: 76,
                cw: 13.0,
                ch: 19.0
            }
        );
        let koi = Kind::Koi(DEFAULT_KOI.to_vec(), Theme::Dark).grid(2560.0, 1440.0);
        assert_eq!(
            koi,
            Grid {
                cols: 233,
                rows: 90,
                cw: 11.0,
                ch: 16.0
            }
        );
        let small = Kind::Sharks(DEFAULT_SHARKS.to_vec()).grid(640.0, 480.0);
        assert_eq!((small.cw, small.ch), (8.0, 12.0));
    }

    fn kind_bg(kind: &Kind) -> [f32; 3] {
        match kind {
            Kind::Sharks(_) => DEEP.bg,
            Kind::Koi(_, theme) => theme.palette().bg,
        }
    }
}
