//! Koi rasterised cell by cell with supersampled edges: blocks in warm koi
//! colours shaded by the light, dense glyphs, ray-striped fins, and glinting
//! scales. Koi bodies, fins, patterns and colours by Ahmet Dedeler.

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
use crate::domain::koi::{Koi, KoiKind};
use crate::domain::math::{Rgb, clamp, hash3, mix, smoothstep};
use crate::domain::pond::nonzero;
use crate::domain::raster::{Covered, Nearest, Pose, Sample, rasterize, skin_key};
use crate::domain::water::{DEEP, Palette};

mod shape;

use shape::{BUTTERFLY, STANDARD, Shape, in_fin, koi_body, noise, ray};

/// Palettes for the koi's water.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Off-white paper, cooled a touch toward pond water so white koi skin
    /// still reads against it. The upstream default.
    Light,
    Dark,
}

impl Theme {
    #[must_use]
    pub const fn palette(self) -> Palette {
        match self {
            Self::Light => Palette {
                bg: [226.0, 232.0, 236.0],
                fish: [34.0, 36.0, 42.0],
                wake: [70.0, 150.0, 165.0],
            },
            Self::Dark => DEEP,
        }
    }

    /// How far distant koi fade into the water.
    const fn haze(self) -> f32 {
        match self {
            Self::Light => 0.35,
            Self::Dark => 0.3,
        }
    }
}

/// Scale glints: warm only, so the skin sparkles without leaving the koi palette.
const GLINTS: [Rgb; 4] = [
    [255.0, 250.0, 240.0],
    [255.0, 231.0, 184.0],
    [255.0, 210.0, 122.0],
    [255.0, 243.0, 226.0],
];

const SKIN_RAMP: &[u8] = b":;t%SX8#@";
// Fins take sparse glyphs so they read lighter than the body they hang off.
const FIN_GLYPHS: &[u8] = b":;'.:";
const EDGE_HEAVY: &[u8] = b":;";
const EDGE_LIGHT: &[u8] = b".',`";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Body,
    Edge,
    Fin,
}

/// Koi skin colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    White,
    /// Kohaku hi: a coral red.
    Red,
    Orange,
}

impl Tone {
    /// Shadow, mid and highlight. Shadows stay warm (peach, brick) instead
    /// of going grey, so a koi reads as colour all the way to its flanks.
    const fn ramp(self) -> [Rgb; 3] {
        match self {
            Self::White => [
                [230.0, 196.0, 176.0],
                [251.0, 241.0, 230.0],
                [255.0, 254.0, 250.0],
            ],
            Self::Red => [
                [190.0, 46.0, 40.0],
                [230.0, 72.0, 56.0],
                [252.0, 132.0, 106.0],
            ],
            Self::Orange => [
                [210.0, 98.0, 20.0],
                [243.0, 140.0, 36.0],
                [255.0, 192.0, 112.0],
            ],
        }
    }

    fn at(self, shade: f32) -> Rgb {
        let [d, m, l] = self.ramp();
        if shade < 0.5 {
            mix(d, m, shade * 2.0)
        } else {
            mix(m, l, (shade - 0.5) * 2.0)
        }
    }
}

/// One koi posed for this frame.
struct Posed {
    pose: Pose,
    kind: KoiKind,
    shape: Shape,
    id: i32,
    length: f32,
    fin_tip: [f32; 2],
    fin_swing: [f32; 2],
}

type Hit = (Part, f32, Tone);

impl Posed {
    fn new(sh: &Koi) -> Self {
        let l = sh.length;
        let swim = clamp(sh.speed / 45.0, 0.55, 1.5);
        let shape = if sh.kind == KoiKind::Butterfly {
            BUTTERFLY
        } else {
            STANDARD
        };
        let tail_start = shape.tail[0];
        // A long, lazy wave: most of the sway is in the back half, and the
        // tail fin only trails a little behind it instead of whipping.
        let pose = Pose::new(&sh.nodes, |s| {
            let trail = 0.25f32.mul_add(smoothstep(tail_start, 1.0, s), 1.0);
            let amp = l * 0.045 * 0.94f32.mul_add(s.powf(1.8), 0.06) * swim * trail;
            amp * f32::mul_add(s, -4.2, sh.phase).sin()
        });
        // Bank into turns: the inside fin tucks in, the outside one spreads.
        // Koi also paddle their pectorals slowly, out of step with each other.
        let bank = clamp(sh.turn * 0.45, -0.35, 0.35);
        let reach = l * shape.pectoral[3];
        let paddle_r = (sh.phase * 0.55).sin();
        let paddle_l = sh.phase.mul_add(0.55, 1.9).sin();
        Self {
            pose,
            kind: sh.kind,
            shape,
            id: sh.id,
            length: l,
            fin_tip: [
                reach * 0.12f32.mul_add(paddle_l, 1.0) * (1.0 + bank),
                reach * 0.12f32.mul_add(paddle_r, 1.0) * (1.0 - bank),
            ],
            fin_swing: [0.025 * paddle_l, 0.025 * paddle_r],
        }
    }

    fn sample(&self, px: f32, py: f32) -> Sample<Hit> {
        let at = self.pose.nearest(px, py);
        Sample {
            at,
            hit: self.classify(at),
        }
    }

    fn classify(&self, at: Nearest) -> Option<Hit> {
        let (a, s, l, shape) = (at.dist, at.s, self.length, &self.shape);
        let u = s * l;
        let w = koi_body(s, shape.end) * l;
        // Nothing behind the tail tip: the spine's round end cap would blunt the fork.
        if at.past_tail > l * 0.008 {
            return None;
        }

        if a <= w {
            let across = a / nonzero(w, 1.0);
            // Rounded back lit from the top-left; flanks fall off into shadow.
            let (nx, ny) = (-at.tangent.1 * at.sign, at.tangent.0 * at.sign);
            let facing = nx.mul_add(-0.6, ny * -0.8) * across;
            let mut shade = facing.mul_add(0.24, (across * across).mul_add(-0.45, 0.62));
            // A soft sheen down the spine where the back rounds toward the light.
            if s > shape.dorsal[0] && s < shape.dorsal[1] && across < 0.3 {
                shade = 0.1f32.mul_add(1.0 - across / 0.3, shade);
            }
            // Smooth, pale snout.
            if s < 0.04 {
                shade = shade.max(0.72);
            }
            return Some((Part::Body, shade, self.skin_at(s, a * at.sign / l, across)));
        }

        // Tail fin: two rounded lobes fanning out from the stalk, forked in the middle.
        let [t0, spread, fork] = shape.tail;
        if s > t0 {
            let k = (s - t0) / (1.0 - t0);
            let round = if k > 0.8 {
                ((k - 0.8) / 0.2)
                    .mul_add(-((k - 0.8) / 0.2), 1.0)
                    .max(0.0)
                    .sqrt()
            } else {
                1.0
            };
            let outer = l * spread.mul_add(smoothstep(0.0, 0.85, k).powf(0.75), 0.014) * round;
            let notch = l * fork * smoothstep(0.6, 1.0, k);
            if a <= outer && a >= notch {
                let shade =
                    0.3f32.mul_add(1.0 - a / nonzero(outer, 1.0), 0.4) - ray(l.mul_add(-t0, u), a);
                return Some((Part::Fin, shade, self.fin_at(k)));
            }
        }

        let side = usize::from(at.sign > 0.0);
        let [p0, p1, p2, pr] = shape.pectoral;
        let (tip, swing) = (self.fin_tip[side], self.fin_swing[side]);
        let pectoral_root = l * (p0 + p1) * 0.5;
        let pectoral = in_fin(
            u,
            a,
            pectoral_root,
            (l * (p2 + swing), tip),
            shape.fin_breadth,
        );
        let fin = pectoral.map(|t| (t, pectoral_root)).or_else(|| {
            let [v0, v1, v2, vr] = shape.pelvic;
            let root = l * (v0 + v1) * 0.5;
            let reach = l * vr * (tip / (l * pr));
            in_fin(
                u,
                a,
                root,
                (l * swing.mul_add(0.4, v2), reach),
                shape.fin_breadth,
            )
            .map(|t| (t, root))
        });
        fin.map(|(t, root)| {
            (
                Part::Fin,
                0.3f32.mul_add(1.0 - t, 0.38) - ray(u - root, a),
                self.fin_at(t),
            )
        })
    }

    /// Skin colour at a body point: s along, vn across as a signed fraction
    /// of length, across from 0 at the spine to 1 at the flank. Patterns are
    /// seeded noise anchored to the body, so they ride along with it.
    fn skin_at(&self, s: f32, vn: f32, across: f32) -> Tone {
        let id = self.id * 13;
        // Pattern space: broad patches, longer than they are wide, like real markings.
        let (px, py) = (s * 4.6, vn * 7.0);
        // Colour sits on the back, clear of the nose and the tail stalk.
        let band = 0.4f32.mul_add(
            -(1.0 - smoothstep(0.015, 0.05, s)),
            0.22f32.mul_add(
                -across * across,
                0.35f32.mul_add(-smoothstep(0.58, 0.72, s), 0.16),
            ),
        );
        // Real patterns come in steps: two or three patches down the back with
        // white gaps between. A slow wave sets the steps, noise breaks up their edges.
        let steps = |freq: f32, salt: i32| {
            0.5f32.mul_add(
                (s.mul_add(freq, hash3(self.id, salt, 17)) * std::f32::consts::TAU).cos(),
                0.5,
            )
        };
        let patch = |seed: i32, freq: f32, salt: i32| {
            0.45f32.mul_add(noise(px, py, id + seed), 0.55 * steps(freq, salt)) + band
        };
        match self.kind {
            // White fish, red steps: roughly 30-60% red.
            KoiKind::Kohaku => {
                if patch(0, 2.6, 3) > 0.6 {
                    Tone::Red
                } else {
                    Tone::White
                }
            }
            // Mostly orange, with white saddles, white flank edges, and a pale nose.
            KoiKind::Orange if s >= 0.03 && across <= 0.85 && patch(3, 2.2, 5) > 0.46 => {
                Tone::Orange
            }
            KoiKind::Butterfly if s >= 0.03 && across <= 0.85 && patch(7, 2.4, 7) > 0.48 => {
                Tone::Orange
            }
            KoiKind::Benigoi if s >= 0.025 => Tone::Red,
            KoiKind::Tancho => {
                // One round red crown on the head, nothing else.
                let (ds, dv) = ((s - 0.1) / 0.06, vn / 0.05);
                if ds.mul_add(ds, dv * dv) < 1.0 {
                    Tone::Red
                } else {
                    Tone::White
                }
            }
            KoiKind::Orange | KoiKind::Butterfly | KoiKind::Benigoi => Tone::White,
        }
    }

    /// Fin colour at t (0 root to 1 tip): body colour bleeding into pale tips.
    fn fin_at(&self, t: f32) -> Tone {
        match self.kind {
            KoiKind::Orange if t < 0.75 => Tone::Orange,
            KoiKind::Benigoi if t < 0.6 => Tone::Red,
            KoiKind::Butterfly if t < 0.45 => Tone::Orange,
            KoiKind::Kohaku if t < 0.2 => Tone::Red,
            _ => Tone::White,
        }
    }
}

/// What a cell shows of a koi.
#[derive(Debug, Clone, Copy)]
struct Skin {
    part: Part,
    shade: f32,
    tone: Tone,
    key: u32,
    depth: f32,
}

/// The koi's cells, rasterised once per frame.
#[derive(Debug)]
pub struct KoiSkins {
    grid: Grid,
    theme: Theme,
    cells: Vec<Option<Skin>>,
}

impl KoiSkins {
    #[must_use]
    pub fn new(grid: Grid, theme: Theme) -> Self {
        Self {
            grid,
            theme,
            cells: vec![None; grid.len()],
        }
    }

    #[must_use]
    pub fn covers(&self, i: usize) -> bool {
        self.cells[i].is_some()
    }

    /// Rasterises every koi, later ones over earlier ones. Fully covered
    /// cells become solid skin; partly covered cells become edge glyphs.
    pub fn rasterize(&mut self, koi: &[Koi], t: f32) {
        self.cells.fill(None);
        let grid = self.grid;
        for sh in koi {
            let posed = Posed::new(sh);
            let cells = &mut self.cells;
            rasterize(
                grid,
                &posed.pose,
                sh.length * 0.3,
                |x, y| posed.sample(x, y),
                |cell: Covered<Hit>| {
                    let key = skin_key(cell.centre.at, grid.cw, sh.id);
                    let tone = cell.first.2;
                    let (part, shade) = match cell.centre.hit {
                        Some((part, shade, _)) if cell.coverage >= 0.6 => {
                            let col = i32::try_from(cell.col).unwrap_or(0);
                            let row = i32::try_from(cell.row).unwrap_or(0);
                            let flicker = (hash3(col, row, (t * 6.0).floor() as i32) - 0.5) * 0.06;
                            (part, clamp(shade + flicker, 0.0, 1.0))
                        }
                        _ => (Part::Edge, cell.coverage),
                    };
                    cells[cell.index] = Some(Skin {
                        part,
                        shade,
                        tone,
                        key,
                        depth: sh.depth,
                    });
                },
            );
        }
    }

    pub fn paint(&self, frame: &mut Frame, t: f32) {
        let bg = self.theme.palette().bg;
        for (i, skin) in self.cells.iter().enumerate() {
            let Some(skin) = *skin else { continue };
            // Distant koi sink into the water: they fade toward the paper, or into the dark.
            let haze = (1.0 - skin.depth) * self.theme.haze();
            paint_cell(frame, i, skin, t, |c| mix(c, bg, haze), bg);
        }
    }
}

fn pick(set: &[u8], key: u32) -> char {
    char::from(set[(key >> 8) as usize % set.len()])
}

fn paint_cell(frame: &mut Frame, i: usize, skin: Skin, t: f32, sink: impl Fn(Rgb) -> Rgb, bg: Rgb) {
    let Skin {
        part,
        shade,
        tone,
        key,
        ..
    } = skin;
    // Glints re-roll on a staggered clock so the scales sparkle.
    let tick = t.mul_add(2.4, (key % 997) as f32 / 97.0).floor() as i32;
    let roll = hash3(key.cast_signed(), tick, 7);

    if part == Part::Edge {
        // Soft, anti-aliased silhouette: no block, just a glyph in the skin's shadow colour.
        let set = if shade >= 0.5 { EDGE_HEAVY } else { EDGE_LIGHT };
        let c = sink(if tone == Tone::White {
            [214.0, 178.0, 160.0]
        } else {
            tone.at(0.15)
        });
        frame.glyph(i, pick(set, key), mix(c, bg, (1.0 - shade) * 0.35), 1.0);
        return;
    }

    let mut fill = tone.at(shade);
    let glyph = if part == Part::Fin {
        // Fins are thin: a lighter tint of their colour, the water showing through.
        fill = mix(fill, bg, 0.5);
        pick(FIN_GLYPHS, key)
    } else {
        let jitter = ((key >> 4) % 3) as f32 - 1.0;
        let ramp = clamp(
            ((1.0 - shade) * 8.99).floor() + jitter,
            0.0,
            (SKIN_RAMP.len() - 1) as f32,
        );
        char::from(SKIN_RAMP[ramp as usize])
    };
    // Texture in the skin's own shadow tone, kept soft so the colour carries the form.
    let mut ink = if tone == Tone::White {
        mix(Tone::White.at(0.0), [190.0, 124.0, 98.0], 0.5)
    } else {
        mix(mix(fill, tone.at(0.0), 0.6), [90.0, 20.0, 10.0], 0.25)
    };
    if part == Part::Fin {
        ink = mix(ink, fill, 0.35);
    }
    frame.block(i, sink(fill));
    let ink = if roll < 0.045 {
        GLINTS[key as usize % GLINTS.len()]
    } else {
        sink(ink)
    };
    frame.glyph(i, glyph, ink, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::frame::Color;
    use crate::domain::koi::{KoiSpec, seed};
    use crate::domain::pond::Pond;

    fn painted(kind: KoiKind) -> (Vec<Skin>, Frame) {
        let grid = Grid {
            cols: 140,
            rows: 60,
            cw: 11.0,
            ch: 16.0,
        };
        let mut pond = Pond::new(
            grid.width(),
            grid.height(),
            grid.cw,
            grid.ch,
            fastrand::Rng::with_seed(4),
        );
        let koi = seed(&[KoiSpec { kind, size: 1.0 }], &mut pond);
        let mut skins = KoiSkins::new(grid, Theme::Dark);
        skins.rasterize(&koi, 0.0);
        let mut frame = Frame::new(grid.cols, grid.rows, Color::default());
        skins.paint(&mut frame, 0.0);
        (skins.cells.iter().flatten().copied().collect(), frame)
    }

    #[test]
    fn a_koi_has_a_body_fins_and_soft_edges() {
        let (cells, _) = painted(KoiKind::Kohaku);
        for part in [Part::Body, Part::Fin, Part::Edge] {
            assert!(cells.iter().any(|c| c.part == part), "no {part:?}");
        }
    }

    #[test]
    fn varieties_wear_their_colours() {
        let share = |kind, tone| {
            let (cells, _) = painted(kind);
            let body: Vec<&Skin> = cells.iter().filter(|c| c.part == Part::Body).collect();
            body.iter().filter(|c| c.tone == tone).count() as f32 / body.len() as f32
        };
        assert!(share(KoiKind::Benigoi, Tone::Red) > 0.9);
        assert!(share(KoiKind::Tancho, Tone::White) > 0.8);
        assert!(share(KoiKind::Tancho, Tone::Red) > 0.0, "the crown");
        assert!((0.1..0.8).contains(&share(KoiKind::Kohaku, Tone::Red)));
        assert!(share(KoiKind::Orange, Tone::Orange) > 0.3);
    }
}
