//! The pixel cat game in the empty PR pane: the sprites and the strip they run on.

use rune::{El, Icon, Transition, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Kit};

use crate::app::App;

use crate::cat::{BLOCK_GAP, CAT_X, Game, Kind, Mode, PIXEL, Pose};

/// The strip's height, and the ground line's place in it.
const STRIP: f32 = 220.0;
const GROUND: f32 = 180.0;

static RUN_A: Icon = Icon {
    name: "cat-run-a",
    view: 16.0,
    body: r#"<path d="M10 5h1v1h-1zM14 5h1v1h-1zM10 6h2v1h-2zM13 6h2v1h-2zM0 7h1v1h-1zM10 7h5v1h-5zM1 8h1v1h-1zM10 8h1v1h-1zM12 8h1v1h-1zM14 8h1v1h-1zM1 9h1v1h-1zM10 9h6v1h-6zM2 10h9v1h-9zM12 10h3v1h-3zM3 11h10v1h-10zM3 12h9v1h-9zM3 13h9v1h-9zM3 14h1v1h-1zM5 14h1v1h-1zM10 14h1v1h-1zM12 14h1v1h-1zM2 15h1v1h-1zM6 15h1v1h-1zM9 15h1v1h-1zM13 15h1v1h-1z" fill="currentColor"/>"#,
};
static RUN_B: Icon = Icon {
    name: "cat-run-b",
    view: 16.0,
    body: r#"<path d="M10 5h1v1h-1zM14 5h1v1h-1zM10 6h2v1h-2zM13 6h2v1h-2zM0 7h1v1h-1zM10 7h5v1h-5zM1 8h1v1h-1zM10 8h1v1h-1zM12 8h1v1h-1zM14 8h1v1h-1zM1 9h1v1h-1zM10 9h6v1h-6zM2 10h9v1h-9zM12 10h3v1h-3zM3 11h10v1h-10zM3 12h9v1h-9zM3 13h9v1h-9zM4 14h1v1h-1zM6 14h1v1h-1zM9 14h1v1h-1zM11 14h1v1h-1zM4 15h1v1h-1zM6 15h1v1h-1zM9 15h1v1h-1zM11 15h1v1h-1z" fill="currentColor"/>"#,
};
static JUMP: Icon = Icon {
    name: "cat-jump",
    view: 16.0,
    body: r#"<path d="M10 5h1v1h-1zM14 5h1v1h-1zM10 6h2v1h-2zM13 6h2v1h-2zM0 7h1v1h-1zM10 7h5v1h-5zM1 8h1v1h-1zM10 8h1v1h-1zM12 8h1v1h-1zM14 8h1v1h-1zM1 9h1v1h-1zM10 9h6v1h-6zM2 10h9v1h-9zM12 10h3v1h-3zM3 11h10v1h-10zM3 12h9v1h-9zM3 13h9v1h-9zM2 14h2v1h-2zM11 14h2v1h-2z" fill="currentColor"/>"#,
};
static BALL_A: Icon = Icon {
    name: "cat-ball-a",
    view: 8.0,
    body: r#"<path d="M2 0h4v1h-4zM1 1h6v1h-6zM0 2h3v1h-3zM4 2h4v1h-4zM0 3h8v1h-8zM0 4h8v1h-8zM0 5h4v1h-4zM5 5h3v1h-3zM1 6h6v1h-6zM2 7h4v1h-4z" fill="currentColor"/>"#,
};
static BALL_B: Icon = Icon {
    name: "cat-ball-b",
    view: 8.0,
    body: r#"<path d="M2 0h4v1h-4zM1 1h6v1h-6zM0 2h4v1h-4zM5 2h3v1h-3zM0 3h8v1h-8zM0 4h8v1h-8zM0 5h3v1h-3zM4 5h4v1h-4zM1 6h6v1h-6zM2 7h4v1h-4z" fill="currentColor"/>"#,
};
static BUG: Icon = Icon {
    name: "cat-bug",
    view: 8.0,
    body: r#"<path d="M1 2h1v1h-1zM6 2h1v1h-1zM2 3h1v1h-1zM5 3h1v1h-1zM1 4h6v1h-6zM0 5h8v1h-8zM1 6h6v1h-6zM0 7h1v1h-1zM2 7h1v1h-1zM5 7h1v1h-1zM7 7h1v1h-1z" fill="currentColor"/>"#,
};
static BLOCK: Icon = Icon {
    name: "cat-block",
    view: 12.0,
    body: r#"<path d="M0 4h12v1h-12zM0 5h1v1h-1zM11 5h1v1h-1zM0 6h1v1h-1zM2 6h1v1h-1zM4 6h1v1h-1zM6 6h1v1h-1zM8 6h1v1h-1zM11 6h1v1h-1zM0 7h1v1h-1zM3 7h1v1h-1zM5 7h1v1h-1zM7 7h1v1h-1zM9 7h1v1h-1zM11 7h1v1h-1zM0 8h1v1h-1zM2 8h1v1h-1zM4 8h1v1h-1zM6 8h1v1h-1zM8 8h1v1h-1zM11 8h1v1h-1zM0 9h1v1h-1zM3 9h1v1h-1zM5 9h1v1h-1zM7 9h1v1h-1zM9 9h1v1h-1zM11 9h1v1h-1zM0 10h1v1h-1zM11 10h1v1h-1zM0 11h12v1h-12z" fill="currentColor"/>"#,
};

/// The game strip: the score line, the cat, the obstacles and the ground.
pub fn view(g: &Game, k: &Kit) -> El {
    let ink = k.c.text_muted;
    let line = match g.mode {
        Mode::Auto => "Up to jump · Down to roll · Right to fire".to_owned(),
        Mode::Play => format!("Score {}", g.score),
        Mode::Over => format!(
            "Ouch! Score {} · Best {} · Up to play again",
            g.score, g.best
        ),
    };
    let (sprite, size, dx) = match g.pose() {
        Pose::Run(0) => (&RUN_A, 16.0, 0.0),
        Pose::Run(_) => (&RUN_B, 16.0, 0.0),
        Pose::Jump => (&JUMP, 16.0, 0.0),
        Pose::Roll(0) => (&BALL_A, 8.0, 16.0),
        Pose::Roll(_) => (&BALL_B, 8.0, 16.0),
    };
    let side = size * PIXEL;
    let cat = icon(sprite, side)
        .color(k.c.text_base)
        .absolute(CAT_X + dx, GROUND - g.y - side)
        .instant_changes();
    let obstacles = g.obstacles.iter().map(|o| {
        let (sprite, color, view, lift) = match o.kind {
            Kind::Bug if o.zapped.is_some() => (&BUG, k.c.text_base, 8.0, 0.0),
            Kind::Bug => (&BUG, k.c.danger.icon, 8.0, 0.0),
            Kind::Block => (&BLOCK, k.c.warning.icon, 12.0, BLOCK_GAP),
        };
        let side = view * PIXEL;
        icon(sprite, side)
            .color(color)
            .absolute(o.x, GROUND - lift - side)
            .instant_changes()
    });
    let beam = g.beam.map(|b| {
        el().h(3.0)
            .w(b.to - b.from)
            .radius(1.5)
            .fill(k.c.danger.icon)
            .absolute(b.from, GROUND - b.height - 1.5)
            .instant_changes()
    });
    let ground = el()
        .h(2.0)
        .w(g.width.max(0.0))
        .fill(k.c.border_muted)
        .absolute(0.0, GROUND);
    col()
        .key("cat-game")
        .w_full()
        .h(STRIP)
        .clip_rounded(0.0)
        .child(text(line).font(k.ty.small).color(ink).absolute(CAT_X, 16.0))
        .child(ground)
        .children(obstacles)
        .children(beam)
        .child(cat)
}
static PEEK: Icon = Icon {
    name: "cat-peek",
    view: 8.0,
    body: r#"<path d="M1 2h1v1h-1zM5 2h1v1h-1zM1 3h2v1h-2zM4 3h2v1h-2zM1 4h5v1h-5zM1 5h1v1h-1zM3 5h1v1h-1zM5 5h1v1h-1zM1 6h5v1h-5zM0 7h2v1h-2zM3 7h1v1h-1zM5 7h2v1h-2z" fill="currentColor"/>"#,
};
static HEAD: Icon = Icon {
    name: "cat-head",
    view: 16.0,
    body: r#"<path d="M1 2h2v1h-2zM13 2h2v1h-2zM1 3h3v1h-3zM12 3h3v1h-3zM1 4h4v1h-4zM11 4h4v1h-4zM1 5h14v1h-14zM0 6h16v1h-16zM0 7h16v1h-16zM0 8h3v1h-3zM5 8h6v1h-6zM13 8h3v1h-3zM0 9h3v1h-3zM5 9h6v1h-6zM13 9h3v1h-3zM0 10h16v1h-16zM0 11h7v1h-7zM9 11h7v1h-7zM0 12h5v1h-5zM6 12h4v1h-4zM11 12h5v1h-5zM0 13h6v1h-6zM10 13h6v1h-6zM1 14h14v1h-14zM2 15h12v1h-12z" fill="currentColor"/>"#,
};
static LID: Icon = Icon {
    name: "cat-lid",
    view: 16.0,
    body: r#"<path d="M11 8h2v1h-2zM11 9h2v1h-2z" fill="currentColor"/>"#,
};
static LASH: Icon = Icon {
    name: "cat-lash",
    view: 16.0,
    body: r#"<path d="M10 9h4v1h-4z" fill="currentColor"/>"#,
};

/// The right eye closes for a moment every 3 seconds.
const WINK: rune::Cycle = rune::Cycle {
    period: 3.0,
    phase: 0.0,
    stops: &[
        (0.0, 0.0),
        (0.86, 0.0),
        (0.88, 1.0),
        (0.95, 1.0),
        (0.97, 0.0),
        (1.0, 0.0),
    ],
    still: 0.0,
    fps: 30.0,
};

/// The big winking cat head of the empty pane while the list loads. A click starts the game.
pub fn head(k: &Kit) -> El {
    let side = 160.0;
    let layer = |sprite, color| icon(sprite, side).color(color).absolute(0.0, 0.0);
    let face = col()
        .key("cat-head")
        .size(side, side)
        .child(layer(&HEAD, k.c.text_base))
        .child(layer(&LID, k.c.text_base).cycle(WINK))
        .child(layer(&LASH, k.c.bg_base).cycle(WINK))
        .help("Play while your PRs load")
        .on_press(|s: &mut App| s.open_game());
    col()
        .w_full()
        .grow(1.0)
        .center()
        .gap(20.0)
        .child(face)
        .child(
            text("Loading your PRs…")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        )
}

/// Puts the peeking cat, or the game, over the open PR's pane.
pub fn over(s: &App, pane: El, k: &Kit) -> El {
    if s.current().is_none() {
        return pane;
    }
    if s.game_open {
        let back = Button::new("Back to the PR")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut App| s.game_open = false)
            .build(k);
        let bar = row()
            .w_full()
            .px(16.0)
            .py(16.0)
            .gap(12.0)
            .items_center()
            .child(back)
            .child(
                text("or press Escape")
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            );
        let cover = col()
            .key("cat-overlay")
            .absolute(0.0, 0.0)
            .size(s.pane_width, s.pane_height)
            .fill(k.c.bg_base)
            .on_press(|_: &mut App| {})
            .child(bar)
            .child(col().grow(1.0))
            .child(view(&s.cat, k));
        return pane.child(cover);
    }
    let Some(at) = s.peek else { return pane };
    let side = 8.0 * PIXEL;
    let x = 40.0 + at * (s.pane_width - 80.0 - side).max(0.0);
    let rise = Transition::offset(0.0, side).combined(Transition::opacity());
    let cat = icon(&PEEK, side)
        .key("peek-cat")
        .color(k.c.text_base)
        .absolute(x, s.pane_height - side + 4.0)
        .transition(rise)
        .help("Psst")
        .on_press(|s: &mut App| s.open_game());
    pane.child(cat)
}
