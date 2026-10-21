//! Where the predators meet the fish: picking prey, eating it, and eaten
//! fish swimming back in as new schools.

#![expect(
    clippy::many_single_char_names,
    reason = "math notation (x, y, dx, d2, ...), kept from the TypeScript this ports so the two read side by side"
)]

use super::{Eaten, Fish, Pond};
use crate::domain::math::{rand, rand_int, rand_sign};
use crate::domain::species::{SPECIES, pick_species};

/// A predator's search for prey: see [`Pond::find_prey`].
#[derive(Debug, Clone, Copy)]
pub struct Hunt {
    pub head: (f32, f32),
    pub heading: f32,
    pub range: f32,
    pub current: Option<usize>,
    pub margin: Option<f32>,
}

impl Pond {
    /// Pick prey in a forward cone from `head` along `heading`, preferring
    /// close fish in sparse buckets (stragglers) over fish buried in a dense
    /// school, and sticking with `current` unless something is clearly
    /// better. With a `margin`, fish that close to the edge are passed over.
    #[must_use]
    pub fn find_prey(&self, hunt: Hunt) -> Option<usize> {
        let (hx, hy) = hunt.head;
        let (fy, fx) = hunt.heading.sin_cos();
        let reachable = |f: &Fish| {
            hunt.margin
                .is_none_or(|m| f.x > m && f.x < self.width - m && f.y > m && f.y < self.height - m)
        };
        let mut best = None;
        let mut best_score = f32::INFINITY;
        let mut current_score = f32::INFINITY;
        for (i, f) in self.fish.iter().enumerate() {
            if !f.alive || !reachable(f) {
                continue;
            }
            let dx = f.x - hx;
            let dy = f.y - hy;
            let d = dx.hypot(dy);
            if d > hunt.range || d < 1e-3 {
                continue;
            }
            let cos = dx.mul_add(fx, dy * fy) / d;
            if cos < 0.2 {
                continue;
            }
            let crowd = f32::from(self.bucket_count[self.bucket_of(f.x, f.y)]);
            let score = d * (1.0 - cos).mul_add(0.8, 1.0) * crowd.mul_add(0.07, 1.0);
            if score < best_score {
                best_score = score;
                best = Some(i);
            }
            if Some(i) == hunt.current {
                current_score = score;
            }
        }
        let keep = hunt
            .current
            .filter(|&t| reachable(&self.fish[t]) && current_score < best_score * 1.3);
        keep.or(best)
    }

    /// Eats every live fish within `radius` of `mouth`; returns how many.
    pub fn feed(&mut self, mouth: (f32, f32), radius: f32) -> u32 {
        let mut eaten = 0;
        for f in &mut self.fish {
            if f.alive
                && (f.y - mouth.1).mul_add(f.y - mouth.1, (f.x - mouth.0).powi(2)) < radius * radius
            {
                f.alive = false;
                f.respawn_at = self.time + rand(&mut self.rng, 3.0, 8.0);
                eaten += 1;
                self.eaten.push(Eaten { x: f.x, y: f.y });
            }
        }
        eaten
    }

    pub fn respawn(&mut self) {
        let (w, h, time) = (self.width, self.height, self.time);
        let due: Vec<usize> = (0..self.fish.len())
            .filter(|&i| !self.fish[i].alive && self.fish[i].respawn_at <= time)
            .collect();
        if due.is_empty() || (due.len() < 6 && time - self.last_spawn < 9.0) {
            return;
        }
        self.last_spawn = time;

        // A new school swims in from a random edge.
        let side = rand_int(&mut self.rng, 0, 3);
        let along = rand(&mut self.rng, 0.15, 0.85);
        let x = match side {
            0 => -20.0,
            1 => w + 20.0,
            _ => along * w,
        };
        let y = match side {
            2 => -20.0,
            3 => h + 20.0,
            _ => along * h,
        };
        let inward = (h / 2.0 - y).atan2(w / 2.0 - x) + rand(&mut self.rng, -0.5, 0.5);
        let species = pick_species(&mut self.rng);
        let sp = SPECIES[species];
        let mill_dir = rand_sign(&mut self.rng);
        let school = usize::try_from(sp.school_size.1).unwrap_or(0);
        for &i in due.iter().take(school) {
            let fx = rand(&mut self.rng, -1.0, 1.0).mul_add(self.perception, x);
            let fy = rand(&mut self.rng, -1.0, 1.0).mul_add(self.perception, y);
            let f = &mut self.fish[i];
            f.alive = true;
            f.species = species;
            f.alarm = 0.0;
            f.turn = 0.0;
            f.mill_dir = mill_dir;
            f.x = fx;
            f.y = fy;
            f.vx = inward.cos() * sp.cruise * 1.4;
            f.vy = inward.sin() * sp.cruise * 1.4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_pond;
    use super::*;

    #[test]
    fn feeding_eats_only_inside_the_mouth_and_schedules_a_return() {
        let mut pond = test_pond();
        for f in &mut pond.fish {
            f.x = 1000.0;
        }
        pond.fish[0].x = 0.0;
        pond.fish[0].y = 0.0;
        pond.fish[1].x = 0.0;
        pond.fish[1].y = 5.0;
        assert_eq!(pond.feed((0.0, 0.0), 6.0), 2);
        assert!(!pond.fish[0].alive && !pond.fish[1].alive);
        assert!((3.0..8.0).contains(&pond.fish[0].respawn_at));
        assert_eq!(pond.eaten.len(), 2);
        assert_eq!(pond.feed((0.0, 0.0), 6.0), 0, "the dead aren't eaten twice");
    }

    #[test]
    fn eaten_fish_return_as_a_school() {
        let mut pond = test_pond();
        for f in pond.fish.iter_mut().take(10) {
            f.alive = false;
            f.respawn_at = 1.0;
        }
        pond.time = 0.5;
        pond.respawn();
        assert_eq!(
            pond.fish.iter().filter(|f| !f.alive).count(),
            10,
            "not due yet"
        );
        pond.time = 10.0;
        pond.respawn();
        let returned = pond.fish.iter().take(10).filter(|f| f.alive).count();
        assert!(returned >= 9, "{returned} returned");
    }

    #[test]
    fn prey_must_be_ahead_and_in_range() {
        let mut pond = test_pond();
        for f in &mut pond.fish {
            f.alive = false;
        }
        pond.fish[3].alive = true;
        pond.fish[3].x = 500.0;
        pond.fish[3].y = 400.0;
        pond.rebuild_hash();
        let hunt = |heading: f32, range: f32| Hunt {
            head: (400.0, 400.0),
            heading,
            range,
            current: None,
            margin: None,
        };
        assert_eq!(pond.find_prey(hunt(0.0, 200.0)), Some(3));
        assert_eq!(
            pond.find_prey(hunt(std::f32::consts::PI, 200.0)),
            None,
            "behind"
        );
        assert_eq!(pond.find_prey(hunt(0.0, 50.0)), None, "out of range");
        let edge = Hunt {
            margin: Some(600.0),
            ..hunt(0.0, 200.0)
        };
        assert_eq!(pond.find_prey(edge), None, "too near the edge");
    }

    #[test]
    fn the_current_target_is_kept_unless_clearly_beaten() {
        let mut pond = test_pond();
        for f in &mut pond.fish {
            f.alive = false;
        }
        for (i, x) in [(0, 520.0), (1, 500.0)] {
            pond.fish[i].alive = true;
            pond.fish[i].x = x;
            pond.fish[i].y = 400.0;
        }
        pond.rebuild_hash();
        let hunt = |current| Hunt {
            head: (400.0, 400.0),
            heading: 0.0,
            range: 300.0,
            current,
            margin: None,
        };
        assert_eq!(pond.find_prey(hunt(None)), Some(1));
        assert_eq!(
            pond.find_prey(hunt(Some(0))),
            Some(0),
            "120 vs 100 is within 30%"
        );
        pond.fish[0].x = 700.0;
        pond.rebuild_hash();
        assert_eq!(pond.find_prey(hunt(Some(0))), Some(1));
    }
}
