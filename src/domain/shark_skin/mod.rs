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
#![expect(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use crate::domain::frame::{Frame, Grid};
use crate::domain::math::{Rgb, clamp, hash3, smoothstep};
use crate::domain::pond::nonzero;
use crate::domain::raster::{Covered, Nearest, Pose, Sample, rasterize, skin_key};
use crate::domain::shark::{Shark, SharkKind};

mod shape;

use shape::{Shape, shape};

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

#[expect(clippy::too_many_arguments, reason = "a point and a triangle")]
fn in_triangle(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> bool {
    let d1 = (ax - bx).mul_add(-(py - by), (px - bx) * (ay - by));
    let d2 = (bx - cx).mul_add(-(py - cy), (px - cx) * (by - cy));
    let d3 = (cx - ax).mul_add(-(py - ay), (px - ax) * (cy - ay));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// One shark posed for this frame.
struct Posed {
    pose: Pose,
    kind: SharkKind,
    shape: Shape,
    id: i32,
    length: f32,
    fin_tip_left: f32,
    fin_tip_right: f32,
}

impl Posed {
    fn new(sh: &Shark) -> Self {
        let l = sh.length;
        let swim = clamp(sh.speed / 60.0, 0.4, 1.4);
        // Travelling body wave, growing toward the tail.
        let pose = Pose::new(&sh.nodes, |s| {
            let amp = l * 0.05 * 0.9f32.mul_add(s.powf(1.8), 0.1) * swim;
            amp * f32::mul_add(s, -5.2, sh.phase).sin()
        });
        let shape = shape(sh.kind);
        // Bank into turns: the inside fin foreshortens, the outside one spreads.
        let bank = clamp(sh.turn * 0.45, -0.35, 0.35);
        let flex = 0.06f32.mul_add((sh.phase * 0.5).sin(), 1.0);
        let reach = l * shape.pectoral[3] * flex;
        Self {
            pose,
            kind: sh.kind,
            shape,
            id: sh.id,
            length: l,
            fin_tip_right: reach * (1.0 - bank),
            fin_tip_left: reach * (1.0 + bank),
        }
    }

    /// Classifies one point. Body coordinates: s along the spine (0 snout,
    /// 1 tail), the distance a across it.
    fn sample(&self, px: f32, py: f32) -> Sample<(Part, f32)> {
        let at = self.pose.nearest(px, py);
        Sample {
            at,
            hit: self.classify(at),
        }
    }

    fn classify(&self, at: Nearest) -> Option<(Part, f32)> {
        let (a, s, l, shape) = (at.dist, at.s, self.length, &self.shape);
        let u = s * l;
        let w = (shape.half_width)(s) * l;

        if let Some(head) = shape.head
            && u < l * (head.sweep + head.depth + head.notch)
        {
            // Cephalofoil: a wide wing across the snout, its tips swept back
            // toward the eyes, and maybe a shallow notch mid front edge.
            let r = a / (l * head.span);
            let front = (l * head.sweep * r).mul_add(r, l * head.notch * (1.0 - r / 0.14).max(0.0));
            // Full depth almost to the tips, so the wing reads as a solid bar.
            let depth = l * head.depth * 0.2f32.mul_add(-r.powi(4), 1.0);
            if r <= 1.0 && u >= front && u <= front + depth {
                return Some(if r > 0.84 {
                    (Part::Eye, 0.0)
                } else {
                    let lip = if u < l.mul_add(0.012, front) {
                        0.1
                    } else {
                        0.0
                    };
                    (Part::Body, 0.2f32.mul_add(-r, 0.62) + lip)
                });
            }
        }

        if a <= w {
            return Some(self.on_body(at, nonzero(w, 1.0)));
        }

        let tip = if at.sign > 0.0 {
            self.fin_tip_right
        } else {
            self.fin_tip_left
        };
        let [p0, p1, p2, _] = shape.pectoral;
        if in_triangle(u, a, l * p0, 0.0, l * p1, 0.0, l * p2, tip)
            // Falcate (sickle) fins get a concave trailing edge.
            && !(shape.falcate
                && in_triangle(u, a, l * p1, 0.0, l * (p2 - p1).mul_add(0.33, p1), tip * 0.55, l * p2, tip))
        {
            let part = if self.marked(s, a * at.sign, 0.0) {
                Part::Mark
            } else {
                Part::Fin
            };
            return Some((part, 0.22f32.mul_add(1.0 - clamp(a / tip, 0.0, 1.0), 0.38)));
        }
        let [v0, v1, v2, vr] = shape.pelvic;
        let pelvic = in_triangle(u, a, l * v0, 0.0, l * v1, 0.0, l * v2, l * vr);
        // Forked caudal fin.
        let caudal = shape.caudal > 0.0
            && in_triangle(u, a, l * 0.86, 0.0, l * 0.95, 0.0, l, l * shape.caudal);
        (pelvic || caudal).then_some((Part::Fin, 0.36))
    }

    fn on_body(&self, at: Nearest, w: f32) -> (Part, f32) {
        let (a, s, shape) = (at.dist, at.s, &self.shape);
        let across = a / w;
        // Countershaded, as seen from above: a dark back that pales toward
        // the flanks, where the white belly shows, under a soft light from
        // the top-left.
        let (nx, ny) = (-at.tangent.1 * at.sign, at.tangent.0 * at.sign);
        let facing = nx.mul_add(-0.6, ny * -0.8) * across;
        let shade = facing.mul_add(0.12, across.powi(3).mul_add(0.36, 0.34));
        let within = |range: [f32; 2]| s > range[0] && s < range[1];
        if within(shape.dorsal) && across < 0.3 {
            (Part::Ridge, shade)
        } else if within(shape.second_dorsal) && across < 0.4 {
            (Part::Body, shade.max(0.82))
        } else if shape.eye.is_some_and(within) && across > 0.6 {
            (Part::Eye, 0.0)
        } else if within(shape.gills) && across > 0.58 {
            // Five gill slits.
            let slit =
                ((s - shape.gills[0]) / (shape.gills[1] - shape.gills[0]) * 5.0).floor() as i32;
            (Part::Body, if slit % 2 == 0 { shade * 0.45 } else { shade })
        } else if s < 0.05 {
            // Pale snout tip.
            (Part::Body, shade.max(0.7))
        } else if self.kind == SharkKind::Zebra
            && s > 0.08
            && s < 0.55
            && (across < 0.1 || (across - 0.6).abs() < 0.08)
        {
            // Zebra sharks carry pale ridges down the back.
            (Part::Body, shade.max(0.8))
        } else if self.marked(s, a * at.sign, across) {
            (Part::Mark, shade)
        } else {
            (Part::Body, shade)
        }
    }

    /// Whether a body point (s along, v across) falls on a dark marking.
    fn marked(&self, s: f32, v: f32, across: f32) -> bool {
        let l = self.length;
        match self.kind {
            SharkKind::Tiger => {
                // Wavy vertical bars over the back half, fading toward the head.
                if !(0.2..=0.9).contains(&s) || across > 0.85 {
                    return false;
                }
                let wobble = 0.35 * ((v / l).mul_add(25.0, s * 5.0)).sin();
                let bar = (s * std::f32::consts::TAU).mul_add(9.0, wobble).sin();
                bar > 0.4f32.mul_add(
                    smoothstep(0.8, 0.9, s),
                    0.5f32.mul_add(-smoothstep(0.2, 0.42, s), 0.65),
                )
            }
            SharkKind::Zebra => {
                // Leopard spots on a jittered grid, so they ride along with the skin.
                if s < 0.07 {
                    return false;
                }
                let gu = s / 0.06;
                let gv = v / (l * 0.045);
                let (cu, cv) = (gu.floor(), gv.floor());
                let (iu, iv) = (cu as i32, cv as i32);
                let jx = hash3(iu, iv, self.id).mul_add(0.5, 0.25);
                let jy = hash3(iu, iv, self.id + 11).mul_add(0.5, 0.25);
                (gv - cv - jy).mul_add(gv - cv - jy, (gu - cu - jx).powi(2)) < 0.08
            }
            _ => false,
        }
    }
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
}

impl SharkSkins {
    #[must_use]
    pub fn new(grid: Grid) -> Self {
        Self {
            grid,
            cells: vec![None; grid.len()],
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
        let grid = self.grid;
        for sh in sharks {
            let posed = Posed::new(sh);
            let cells = &mut self.cells;
            rasterize(
                grid,
                &posed.pose,
                sh.length * 0.36,
                |x, y| posed.sample(x, y),
                |cell: Covered<(Part, f32)>| {
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

    pub fn paint(&self, frame: &mut Frame) {
        for (i, skin) in self.cells.iter().enumerate() {
            let Some(skin) = *skin else { continue };
            paint_cell(frame, i, skin);
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

fn paint_cell(frame: &mut Frame, i: usize, skin: Skin) {
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
    // seen through deep water.
    let block = tinted(level * dim, tint, 4.0);
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
    use crate::domain::shark::{SharkSpec, seed};

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
        skins.paint(&mut frame);
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
