//! A shark posed for one frame, and what each point near it hits: body,
//! fins, head, eyes and markings, in body coordinates along the spine.

#![expect(
    clippy::cast_possible_truncation,
    reason = "lattice coordinates of skin markings, far below 2^31"
)]
#![expect(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::Part;
use super::shape::{Shape, shape};
use crate::domain::math::{clamp, hash3, smoothstep};
use crate::domain::pond::nonzero;
use crate::domain::raster::{Nearest, Pose, Sample};
use crate::domain::shark::{Shark, SharkKind};

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
pub(super) struct Posed {
    pub(super) pose: Pose,
    kind: SharkKind,
    pub(super) shape: Shape,
    id: i32,
    length: f32,
    fin_tip_left: f32,
    fin_tip_right: f32,
}

impl Posed {
    pub(super) fn new(sh: &Shark) -> Self {
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
    pub(super) fn sample(&self, px: f32, py: f32) -> Sample<(Part, f32)> {
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
