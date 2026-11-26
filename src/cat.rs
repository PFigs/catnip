//! The pixel cat game of the empty PR pane: its rules, apart from the view. Points grow up
//! from the ground and right from the pane's left edge.

/// The size of one art pixel, in points.
pub const PIXEL: f32 = 4.0;
/// The left edge of the cat's sprite.
pub const CAT_X: f32 = 80.0;
/// How high the bottom of a block hangs above the ground: a running cat is taller, a ball is
/// lower.
pub const BLOCK_GAP: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bug,
    Block,
}

impl Kind {
    /// The sprite's width and height in points.
    pub fn size(self) -> (f32, f32) {
        match self {
            Kind::Bug => (8.0 * PIXEL, 6.0 * PIXEL),
            Kind::Block => (12.0 * PIXEL, 8.0 * PIXEL),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub kind: Kind,
    /// The left edge.
    pub x: f32,
    /// The seconds since a laser hit this bug. It flashes, then goes.
    pub zapped: Option<f32>,
    /// In self-play, the cat zaps this bug instead of jumping it.
    auto_zap: bool,
}

impl Obstacle {
    pub fn new(kind: Kind, x: f32) -> Self {
        Self {
            kind,
            x,
            zapped: None,
            auto_zap: false,
        }
    }

    /// The bottom and the top.
    fn span(&self) -> (f32, f32) {
        let bottom = match self.kind {
            Kind::Bug => 0.0,
            Kind::Block => BLOCK_GAP,
        };
        (bottom, bottom + self.kind.size().1)
    }
}

/// A laser beam: level, from `from` to `to`, at `height` above the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    pub from: f32,
    pub to: f32,
    pub height: f32,
    /// The seconds it still shows.
    left: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// The cat plays itself.
    Auto,
    Play,
    /// The cat hit something. Up plays again.
    Over,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Jump,
    Roll,
    Fire,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pose {
    Run(u8),
    Jump,
    Roll(u8),
}

pub struct Game {
    pub mode: Mode,
    /// The cat's height above the ground.
    pub y: f32,
    vy: f32,
    /// The seconds of roll left.
    rolling: f32,
    pub obstacles: Vec<Obstacle>,
    /// The points the world moved, which picks the leg frames.
    distance: f32,
    /// The points the world moved in this play.
    played: f32,
    pub score: u32,
    pub best: u32,
    /// The points zaps scored in this play.
    bonus: u32,
    pub beam: Option<Beam>,
    /// The seconds until the next shot.
    cooldown: f32,
    /// The pane's width, where obstacles come in.
    pub width: f32,
    seed: u32,
    /// The seconds since the game ended.
    over_for: f32,
    /// The time of the last tick.
    last: Option<f64>,
}

const SPEED: f32 = 300.0;
const GRAVITY: f32 = 1800.0;
const JUMP: f32 = 600.0;
const ROLL_SECS: f32 = 0.6;
const COOLDOWN: f32 = 0.4;
const BEAM_SECS: f32 = 0.12;
const ZAP_SECS: f32 = 0.25;
const ZAP_SCORE: u32 = 5;
/// After a hit with no key for this long, the cat plays itself again.
const OVER_SECS: f32 = 5.0;

impl Default for Game {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            y: 0.0,
            vy: 0.0,
            rolling: 0.0,
            obstacles: Vec::new(),
            distance: 0.0,
            played: 0.0,
            score: 0,
            best: 0,
            bonus: 0,
            beam: None,
            cooldown: 0.0,
            width: 0.0,
            seed: 0x9E37_79B9,
            over_for: 0.0,
            last: None,
        }
    }
}

impl Game {
    /// Steps the game to `now`, in seconds.
    pub fn tick(&mut self, now: f64) {
        let dt = self.last.map_or(0.0, |l| (now - l).clamp(0.0, 0.05));
        self.last = Some(now);
        self.step(dt as f32);
    }

    /// Stops the clock, so the next tick moves nothing.
    pub fn pause(&mut self) {
        self.last = None;
    }

    pub fn step(&mut self, dt: f32) {
        if self.mode == Mode::Over {
            self.over_for += dt;
            if self.over_for >= OVER_SECS {
                self.restart(Mode::Auto);
            }
            return;
        }
        let dx = SPEED * dt;
        self.distance += dx;
        self.played += dx;
        for o in &mut self.obstacles {
            o.x -= dx;
        }
        for o in &mut self.obstacles {
            o.zapped = o.zapped.map(|z| z + dt);
        }
        self.obstacles
            .retain(|o| o.x + o.kind.size().0 > 0.0 && o.zapped.is_none_or(|z| z < ZAP_SECS));
        self.cooldown = (self.cooldown - dt).max(0.0);
        if let Some(b) = &mut self.beam {
            b.left -= dt;
        }
        self.beam = self.beam.filter(|b| b.left > 0.0);
        self.spawn();
        if self.mode == Mode::Auto {
            self.autoplay();
        }
        self.vy -= GRAVITY * dt;
        self.y = (self.y + self.vy * dt).max(0.0);
        if self.y == 0.0 {
            self.vy = 0.0;
        }
        self.rolling = (self.rolling - dt).max(0.0);
        if self.mode == Mode::Play {
            self.score = (self.played / 20.0) as u32 + self.bonus;
            if self.hit() {
                self.mode = Mode::Over;
                self.best = self.best.max(self.score);
                self.over_for = 0.0;
            }
        }
    }

    /// A key in the game. The first key takes over from the cat, and Up plays again after a
    /// hit.
    pub fn press(&mut self, m: Move) {
        match (self.mode, m) {
            (Mode::Over, Move::Jump) => return self.restart(Mode::Play),
            (Mode::Over, _) => return,
            (Mode::Auto, _) => self.restart(Mode::Play),
            (Mode::Play, _) => {}
        }
        match m {
            Move::Jump if self.y == 0.0 => {
                self.vy = JUMP;
                self.rolling = 0.0;
            }
            Move::Jump => {}
            Move::Roll => {
                self.rolling = ROLL_SECS;
                self.vy = self.vy.min(-JUMP);
            }
            Move::Fire => self.fire(),
        }
    }

    pub fn pose(&self) -> Pose {
        if self.rolling > 0.0 {
            Pose::Roll((self.distance / 16.0) as u8 % 2)
        } else if self.y > 0.0 {
            Pose::Jump
        } else {
            Pose::Run((self.distance / 24.0) as u8 % 2)
        }
    }

    /// The cat's box: left, bottom, right, top. The ball is half as wide and lower than the cat.
    pub fn cat_box(&self) -> [f32; 4] {
        match self.rolling > 0.0 {
            true => [CAT_X + 16.0, self.y, CAT_X + 48.0, self.y + 8.0 * PIXEL],
            false => [CAT_X + 8.0, self.y, CAT_X + 60.0, self.y + 10.0 * PIXEL],
        }
    }

    /// Whether the cat touches an obstacle.
    pub fn hit(&self) -> bool {
        let [l, b, r, t] = self.cat_box();
        self.obstacles
            .iter()
            .filter(|o| o.zapped.is_none())
            .any(|o| {
                let (bottom, top) = o.span();
                o.x < r && o.x + o.kind.size().0 > l && bottom < t && top > b
            })
    }

    /// Fires a level beam from the cat's chest. It zaps the first bug in its way, and a block
    /// stops it.
    fn fire(&mut self) {
        if self.cooldown > 0.0 {
            return;
        }
        self.cooldown = COOLDOWN;
        let [_, _, from, _] = self.cat_box();
        let height = self.y + 14.0;
        let edge = self.width.max(400.0);
        let target = self
            .obstacles
            .iter_mut()
            .filter(|o| o.zapped.is_none() && o.x + o.kind.size().0 > from && o.x < edge)
            .filter(|o| (o.span().0..=o.span().1).contains(&height))
            .min_by(|a, b| a.x.total_cmp(&b.x));
        let to = match target {
            Some(o) => {
                if o.kind == Kind::Bug {
                    o.zapped = Some(0.0);
                    self.bonus += ZAP_SCORE;
                }
                o.x
            }
            None => edge,
        };
        self.beam = Some(Beam {
            from,
            to,
            height,
            left: BEAM_SECS,
        });
    }

    fn restart(&mut self, mode: Mode) {
        *self = Game {
            mode,
            best: self.best,
            width: self.width,
            seed: self.seed,
            last: self.last,
            ..Game::default()
        };
    }

    /// Keeps one obstacle waiting past the right edge. A run starts with a bug.
    fn spawn(&mut self) {
        let edge = self.width.max(400.0);
        let last = self.obstacles.last().map_or(0.0, |o| o.x);
        if last > edge {
            return;
        }
        let kind = match self.obstacles.is_empty() || self.random() % 10 < 7 {
            true => Kind::Bug,
            false => Kind::Block,
        };
        let x = edge.max(last) + 360.0 + (self.random() % 400) as f32;
        let mut o = Obstacle::new(kind, x);
        o.auto_zap = kind == Kind::Bug && self.random().is_multiple_of(3);
        self.obstacles.push(o);
    }

    fn random(&mut self) -> u32 {
        random(&mut self.seed)
    }

    /// Zaps about one bug in three, jumps the others, and rolls under a block, just before the
    /// cat reaches it.
    fn autoplay(&mut self) {
        let front = CAT_X + 60.0;
        let next = self
            .obstacles
            .iter()
            .find(|o| o.zapped.is_none() && o.x + o.kind.size().0 > CAT_X + 8.0);
        let Some(o) = next else { return };
        let gap = o.x - front;
        let ready = self.y == 0.0 && self.rolling == 0.0;
        match o.kind {
            Kind::Bug if o.auto_zap && ready && gap < 200.0 => self.fire(),
            Kind::Bug if ready && gap < 60.0 => self.vy = JUMP,
            Kind::Block if ready && gap < 40.0 => self.rolling = ROLL_SECS,
            _ => {}
        }
    }
}

/// The next number of a fixed sequence from `seed`.
pub fn random(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 30.0;

    #[test]
    fn the_cat_plays_itself_without_a_hit() {
        let mut g = Game {
            width: 1200.0,
            ..Game::default()
        };
        let mut seen = Vec::new();
        for _ in 0..(60.0 / DT) as usize {
            g.step(DT);
            assert!(!g.hit(), "a hit at {:?}", g.obstacles);
            seen.extend(g.obstacles.iter().map(|o| o.kind));
        }
        assert_eq!(g.mode, Mode::Auto);
        assert!(seen.contains(&Kind::Bug) && seen.contains(&Kind::Block));
        assert!(g.bonus > 0, "the cat zaps some bugs");
    }

    #[test]
    fn a_player_who_does_not_jump_hits_the_first_bug() {
        let mut g = Game {
            width: 1200.0,
            ..Game::default()
        };
        g.press(Move::Jump);
        assert_eq!(g.mode, Mode::Play);
        for _ in 0..(10.0 / DT) as usize {
            g.step(DT);
            if g.mode == Mode::Over {
                break;
            }
        }
        assert_eq!(g.mode, Mode::Over);
        assert!(g.score > 0);
        assert_eq!(g.best, g.score);
        assert_eq!(g.obstacles.first().map(|o| o.kind), Some(Kind::Bug));
    }

    #[test]
    fn a_laser_zaps_the_bug_ahead_and_a_block_stops_it() {
        let at = Obstacle::new;
        let mut g = Game {
            width: 1200.0,
            mode: Mode::Play,
            obstacles: vec![at(Kind::Bug, 400.0)],
            ..Game::default()
        };
        g.press(Move::Fire);
        g.step(DT);
        assert!(g.obstacles[0].zapped.is_some(), "the bug ahead");
        assert!(g.score >= 5, "a zap scores 5: {}", g.score);

        let mut g = Game {
            width: 1200.0,
            mode: Mode::Play,
            y: 30.0,
            obstacles: vec![at(Kind::Block, 300.0), at(Kind::Bug, 600.0)],
            ..Game::default()
        };
        g.press(Move::Fire);
        g.step(DT);
        assert!(
            g.obstacles.iter().all(|o| o.zapped.is_none()),
            "the block stops the beam"
        );
    }
}
