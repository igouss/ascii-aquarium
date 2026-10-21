//! Sharks rasterised cell by cell with supersampled edges, as seen from
//! above: countershaded blocks, dark along the back and pale at the flanks,
//! a glyph texture as dense as the shadow, and outlines traced along the
//! silhouette.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "grid sizes, cell counts and ids are far below 2^23, and float-to-index casts follow a floor and a clamp to the grid"
)]

use crate::domain::frame::{Frame, Grid};
use crate::domain::math::{Rgb, clamp, hash3};
use crate::domain::raster::{Covered, rasterize, skin_key};
use crate::domain::shark::Shark;

mod posed;
mod shape;

use posed::Posed;

// Sparse glyphs on pale skin, dense glyphs in shadow, so the texture
// reinforces the shading instead of reading as letters.
const SKIN_RAMP: &[u8] = b".:-=+*#%@";
const EDGE_LIGHT: &[u8] = b".'`";
const MARK_GLYPHS: &[u8] = b"#%@";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Body,
    Edge,
    Fin,
    /// The first dorsal fin, seen from above as a blade along the spine.
    Ridge,
    Eye,
    /// Dark skin markings: tiger bars, zebra-shark spots.
    Mark,
}

/// What a cell shows of a shark.
#[derive(Debug, Clone, Copy)]
struct Skin {
    part: Part,
    shade: f32,
    key: u32,
    depth: f32,
    tint: Rgb,
    /// A line along the outline or, on the dorsal blade, the spine.
    stroke: char,
}

/// The sharks' cells, rasterised once per frame.
#[derive(Debug)]
pub struct SharkSkins {
    grid: Grid,
    cells: Vec<Option<Skin>>,
    /// How much of the light each cell of water loses to a shark above it.
    shadows: Vec<f32>,
}

/// The darkest a shark's shadow gets on the water.
const SHADOW: f32 = 0.5;

/// How much of its shadow a covered cell casts on itself and each neighbour.
const PENUMBRA: [(isize, isize, f32); 9] = [
    (0, 0, 1.0),
    (-1, 0, 0.6),
    (1, 0, 0.6),
    (0, -1, 0.6),
    (0, 1, 0.6),
    (-1, -1, 0.35),
    (1, -1, 0.35),
    (-1, 1, 0.35),
    (1, 1, 0.35),
];

impl SharkSkins {
    #[must_use]
    pub fn new(grid: Grid) -> Self {
        Self {
            grid,
            cells: vec![None; grid.len()],
            shadows: vec![0.0; grid.len()],
        }
    }

    #[must_use]
    pub fn covers(&self, i: usize) -> bool {
        self.cells[i].is_some()
    }

    /// Rasterises every shark, later ones over earlier ones. Fully covered
    /// cells become solid skin; partly covered cells become edge glyphs.
    pub fn rasterize(&mut self, sharks: &[Shark], t: f32) {
        self.cells.fill(None);
        self.shadows.fill(0.0);
        let grid = self.grid;
        for sh in sharks {
            let posed = Posed::new(sh);
            let (cells, shadows) = (&mut self.cells, &mut self.shadows);
            // Sunlight from the top-left: the shadow falls down and to the
            // right, farther for a shark nearer the surface.
            let fall = sh.length * sh.depth;
            let shift = (
                (fall * 0.06 / grid.cw).round() as usize,
                (fall * 0.08 / grid.ch).round() as usize,
            );
            rasterize(
                grid,
                &posed.pose,
                sh.length * 0.36,
                |x, y| posed.sample(x, y),
                |cell: Covered<(Part, f32)>| {
                    // A soft shadow: each cell shades its neighbours too,
                    // fading out over a cell.
                    let (col, row) = (cell.col + shift.0, cell.row + shift.1);
                    for (dc, dr, falloff) in PENUMBRA {
                        let (Some(c), Some(r)) =
                            (col.checked_add_signed(dc), row.checked_add_signed(dr))
                        else {
                            continue;
                        };
                        if c < grid.cols && r < grid.rows {
                            let shadow = &mut shadows[r * grid.cols + c];
                            *shadow = shadow.max(SHADOW * falloff * cell.coverage);
                        }
                    }
                    let key = skin_key(cell.centre.at, grid.cw, sh.id);
                    let skin = match cell.centre.hit {
                        Some((part, shade)) if cell.coverage >= 0.6 => {
                            let col = i32::try_from(cell.col).unwrap_or(0);
                            let row = i32::try_from(cell.row).unwrap_or(0);
                            let flicker = (hash3(col, row, (t * 6.0).floor() as i32) - 0.5) * 0.06;
                            (part, clamp(shade + flicker, 0.0, 1.0))
                        }
                        _ => (Part::Edge, cell.coverage),
                    };
                    cells[cell.index] = Some(Skin {
                        part: skin.0,
                        shade: skin.1,
                        key,
                        depth: sh.depth,
                        tint: posed.shape.tint,
                        // Outlines follow the silhouette; the dorsal blade
                        // follows the spine.
                        stroke: if skin.0 == Part::Edge {
                            stroke(cell.edge())
                        } else {
                            stroke(cell.centre.at.tangent)
                        },
                    });
                },
            );
        }
    }

    /// Darkens the water under each shark; call it before anything is drawn
    /// on the water, so fish swim over the shadows.
    pub fn paint_shadows(&self, frame: &mut Frame) {
        let water = frame.bg.rgb();
        for (i, &shadow) in self.shadows.iter().enumerate() {
            if shadow > 0.0 && self.cells[i].is_none() {
                frame.block(i, water.map(|c| c * (1.0 - shadow)));
            }
        }
    }

    pub fn paint(&self, frame: &mut Frame, t: f32) {
        for (i, skin) in self.cells.iter().enumerate() {
            let Some(skin) = *skin else { continue };
            let x = (i % self.grid.cols) as f32 * self.grid.cw;
            let y = (i / self.grid.cols) as f32 * self.grid.ch;
            paint_cell(frame, i, skin, caustic(x, y, t));
        }
    }
}

fn pick(set: &[u8], key: u32) -> char {
    char::from(set[(key >> 8) as usize % set.len()])
}

/// The line glyph closest to the direction `tangent` on screen, y down,
/// or a dot when it has none.
fn stroke((tx, ty): (f32, f32)) -> char {
    if tx == 0.0 && ty == 0.0 {
        return '.';
    }
    let angle = ty.atan2(tx).rem_euclid(std::f32::consts::PI) / std::f32::consts::PI;
    match angle {
        a if !(0.125..0.875).contains(&a) => '-',
        a if a < 0.375 => '\\',
        a if a < 0.625 => '|',
        _ => '/',
    }
}

fn tinted(level: f32, tint: Rgb, blue: f32) -> Rgb {
    [
        level * tint[0],
        level * tint[1],
        level.mul_add(tint[2], blue),
    ]
}

/// Sunlight focused by surface ripples into a drifting net of bright lines,
/// 0 (none) to 1, at pixel (x, y).
fn caustic(x: f32, y: f32, t: f32) -> f32 {
    // Two crossing ripple trains, each warped by the other axis; the light
    // gathers where their sum crosses zero.
    let across = y
        .mul_add(0.017, t * 0.6)
        .sin()
        .mul_add(1.3, x * 0.021)
        .sin();
    let along = x
        .mul_add(0.013, -t * 0.45)
        .sin()
        .mul_add(1.6, y.mul_add(0.019, t * 0.3))
        .sin();
    (across + along).abs().mul_add(-0.5, 1.0).powi(6)
}

fn paint_cell(frame: &mut Frame, i: usize, skin: Skin, light: f32) {
    let Skin {
        part,
        shade,
        key,
        depth,
        tint,
        stroke,
    } = skin;
    let dim = 0.3f32.mul_add(depth, 0.7);

    if part == Part::Edge {
        // Soft silhouette: no block, just a glyph. Mostly covered cells trace
        // the outline along the body; barely touched ones get a faint dot.
        let glyph = if shade >= 0.4 {
            stroke
        } else {
            pick(EDGE_LIGHT, key)
        };
        let g = shade.mul_add(70.0, 60.0) * dim;
        frame.glyph(i, glyph, tinted(g, tint, 6.0), 1.0);
        return;
    }

    let ramp = |shade: f32| {
        // A step denser now and then, so the skin has grain.
        let jitter = if key % 5 == 0 { 1.0 } else { 0.0 };
        let level = clamp(
            ((1.0 - shade) * 8.99).floor() + jitter,
            0.0,
            (SKIN_RAMP.len() - 1) as f32,
        );
        char::from(SKIN_RAMP[level as usize])
    };
    let (level, glyph) = match part {
        Part::Ridge => (shade.mul_add(110.0, 30.0), stroke),
        Part::Eye => (12.0, 'o'),
        Part::Mark => (shade.mul_add(22.0, 24.0), pick(MARK_GLYPHS, key)),
        Part::Fin => (shade.mul_add(120.0, 40.0), ramp(shade)),
        Part::Body | Part::Edge => (shade.mul_add(170.0, 40.0), ramp(shade)),
    };
    // Tinted per species; the classic tint is a cool blue-grey, like a back
    // seen through deep water. Caustics brighten the skin, most near the
    // surface.
    let sun = if part == Part::Eye {
        1.0
    } else {
        (0.45 * light).mul_add(depth, 1.0)
    };
    let block = tinted(level * dim * sun, tint, 4.0);
    frame.block(i, block);

    // The texture is the skin a shade darker, the eye a wet glint.
    let ink = match part {
        Part::Eye => [154.0, 160.0, 173.0],
        Part::Mark => tinted(70.0, tint, 0.0),
        Part::Ridge => block.map(|c| c * 1.6),
        _ => block.map(|c| c * 0.62),
    };
    frame.glyph(i, glyph, ink, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::frame::Color;
    use crate::domain::pond::Pond;
    use crate::domain::shark::{SharkKind, SharkSpec, seed};

    #[test]
    fn strokes_follow_the_direction_on_screen() {
        assert_eq!(stroke((1.0, 0.0)), '-');
        assert_eq!(stroke((-1.0, 0.05)), '-');
        assert_eq!(stroke((0.0, 1.0)), '|');
        assert_eq!(stroke((1.0, 1.0)), '\\', "falling to the right, y down");
        assert_eq!(stroke((1.0, -1.0)), '/');
        assert_eq!(stroke((-1.0, 1.0)), '/');
        assert_eq!(stroke((0.0, 0.0)), '.', "no direction");
    }

    #[test]
    fn a_shark_casts_a_shadow_on_the_water_beside_it() {
        let grid = Grid {
            cols: 160,
            rows: 80,
            cw: 7.0,
            ch: 10.0,
        };
        let mut pond = Pond::new(
            grid.width(),
            grid.height(),
            grid.cw,
            grid.ch,
            fastrand::Rng::with_seed(5),
        );
        let sharks = seed(
            &[SharkSpec {
                kind: SharkKind::Classic,
                size: 1.0,
            }],
            &mut pond,
        );
        let mut skins = SharkSkins::new(grid);
        skins.rasterize(&sharks, 0.0);
        let water = Color(40, 80, 120);
        let mut frame = Frame::new(grid.cols, grid.rows, water);
        skins.paint_shadows(&mut frame);
        let shaded: Vec<usize> = (0..grid.len())
            .filter(|&i| frame.cells[i].bg != water)
            .collect();
        assert!(!shaded.is_empty(), "no shadow");
        assert!(
            shaded.iter().all(|&i| !skins.covers(i)),
            "shadows only on open water"
        );
        let darkest = shaded.iter().map(|&i| frame.cells[i].bg.2).min();
        assert_eq!(darkest, Some(60), "at most half the light is lost");
    }

    #[test]
    fn a_shark_paints_solid_skin_inside_soft_edges() {
        let grid = Grid {
            cols: 120,
            rows: 50,
            cw: 13.0,
            ch: 19.0,
        };
        let mut pond = Pond::new(
            grid.width(),
            grid.height(),
            grid.cw,
            grid.ch,
            fastrand::Rng::with_seed(5),
        );
        let sharks = seed(
            &[SharkSpec {
                kind: SharkKind::Tiger,
                size: 1.0,
            }],
            &mut pond,
        );
        let mut skins = SharkSkins::new(grid);
        skins.rasterize(&sharks, 0.0);
        let parts: Vec<Part> = skins.cells.iter().flatten().map(|s| s.part).collect();
        for part in [Part::Body, Part::Edge, Part::Fin, Part::Ridge, Part::Eye] {
            assert!(
                parts.contains(&part),
                "no {part:?} in {} cells",
                parts.len()
            );
        }
        let mut frame = Frame::new(grid.cols, grid.rows, Color::default());
        skins.paint(&mut frame, 0.0);
        let blocks = frame
            .cells
            .iter()
            .filter(|c| c.bg != Color::default())
            .count();
        let solid = parts.iter().filter(|&&p| p != Part::Edge).count();
        assert_eq!(
            blocks, solid,
            "every solid cell is a block, edges are bare glyphs"
        );
    }
}
