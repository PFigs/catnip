//! The window: the PR drawer on the left, the filters and the review button on top, the PR on the right.

mod cat;
mod detail;
mod dialogs;
mod drawer;
mod sample;

use rune::{Color, Cx, El, Icon, el, icon, row, text};
use rune_kit::shell::{SidebarToggle, SplitView, Toolbar};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit,
    Menu, MenuItem, PaletteSeed, SegmentedControl, Toast, ToastRegion, glyphs,
};

use crate::app::App;
use crate::github::Rerun;
use crate::keys::{self, Hint};
use crate::model::{Ci, Mark, Quick, ReviewState};

pub use sample::{booting, sample};

static BUBBLE: Icon = Icon {
    name: "pr-bubble",
    view: 16.0,
    body: r#"<path d="M2.5 3.5H13.5V10.5H7L4 13V10.5H2.5Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none"/>"#,
};
/// A page with a minus sign: changes asked for.
static CHANGES: Icon = Icon {
    name: "pr-changes",
    view: 16.0,
    body: r#"<path d="M4 1.75H9.5L12.5 4.75V14.25H4Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none"/><path d="M6 9H10.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#,
};
static REFRESH: Icon = Icon {
    name: "pr-refresh",
    view: 16.0,
    body: r#"<path d="M12.5 6A5 5 0 1 0 13 9.5M12.5 2.5V6H9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};
static GEAR: Icon = Icon {
    name: "pr-gear",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="2.2" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M8 1.5V3.5M8 12.5V14.5M1.5 8H3.5M12.5 8H14.5M3.4 3.4L4.8 4.8M11.2 11.2L12.6 12.6M3.4 12.6L4.8 11.2M11.2 4.8L12.6 3.4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};
static SORT: Icon = Icon {
    name: "pr-sort",
    view: 16.0,
    body: r#"<path d="M3 4.5H13M3 8H10M3 11.5H7" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none"/>"#,
};

/// The size of a CI mark.
const CI_MARK: f32 = 14.0;
/// The CI menu is wide enough for the second line of "Update branch".
const CI_MENU_WIDTH: f32 = 320.0;
/// How far past the switch point the toolbar changes between its tight and full forms.
const SWITCH_MARGIN: f32 = 4.0;
/// The room a button icon takes with its gap to the label.
const ICON_ROOM: f32 = 22.0;
/// The room kept for the "+N" and "Clear" buttons.
const MORE_WIDTH: f32 = 56.0;
const CLEAR_WIDTH: f32 = 56.0;
const SAVE_GROUP: &str = "Save group";
const GAP: f32 = 4.0;

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::TRANSPARENT)
}

/// GitHub's background, text and accent colours.
fn kit(dark: bool) -> Kit {
    let (bg, fg, accent) = if dark {
        ("#0d1117", "#e6edf3", "#4493f8")
    } else {
        ("#ffffff", "#1f2328", "#0969da")
    };
    let mut k = Kit::from_palette(PaletteSeed {
        bg: hex(bg),
        fg: hex(fg),
        accent: hex(accent),
    });
    k.radius.sm = 6.0;
    k
}

/// The kit with its submit button in the success colour, for Approve.
fn approve_kit(k: &Kit) -> Kit {
    let mut g = k.clone();
    g.c.bg_icon_button_contrast = k.c.success.icon;
    g.c.submit_fg = k.c.bg_base;
    g.c.submit_icon = k.c.bg_base;
    g
}

/// The viewer's verdict in words, or None when the viewer has not reviewed the PR.
pub fn verdict_words(m: Mark) -> Option<&'static str> {
    match m {
        Mark::Mine | Mark::Todo => None,
        Mark::Acted(v) | Mark::NewCommits(v) => Some(match v {
            ReviewState::Approved => "You approved",
            ReviewState::ChangesRequested => "You asked for changes",
            ReviewState::Commented => "You commented",
        }),
    }
}

/// The mark with its tooltip, or None for a PR of the viewer. New commits since the review put
/// a dot on the verdict's icon.
pub fn mark_icon(m: Mark, k: &Kit) -> Option<El> {
    let verdict = match m {
        Mark::Mine => return None,
        Mark::Todo => {
            let eye = icon(&glyphs::EYE, 14.0).color(k.c.bg_accent);
            return Some(eye.help("Waiting for your review"));
        }
        Mark::Acted(v) | Mark::NewCommits(v) => v,
    };
    let (glyph, color) = match verdict {
        ReviewState::Approved => (&glyphs::CHECK, k.c.success.icon),
        ReviewState::ChangesRequested => (&CHANGES, k.c.danger.icon),
        ReviewState::Commented => (&BUBBLE, k.c.icon_muted),
    };
    let words = verdict_words(m).unwrap_or_default();
    let mark = icon(glyph, 14.0).color(color);
    if !matches!(m, Mark::NewCommits(_)) {
        return Some(mark.help(words));
    }
    let dot = el()
        .key("new-commits")
        .size(7.0, 7.0)
        .radius(3.5)
        .fill(k.c.warning.icon)
        .border(1.5, k.c.bg_base)
        .absolute(9.0, -1.0);
    let both = row().size(14.0, 14.0).child(mark).child(dot);
    Some(both.help(format!("{words}, then new commits came")))
}

pub fn ci_help(ci: Option<Ci>) -> &'static str {
    match ci {
        Some(Ci::Passes) => "CI passes",
        Some(Ci::Fails) => "CI fails",
        Some(Ci::Runs) => "CI runs",
        None => "No CI on this commit",
    }
}

/// The CI mark: a ring of its colour with no glyph, so it does not read as the tick of the
/// viewer's review. A run pulses.
pub fn ci_icon(ci: Ci, k: &Kit) -> El {
    let color = match ci {
        Ci::Passes => k.c.success.icon,
        Ci::Fails => k.c.danger.icon,
        Ci::Runs => k.c.warning.icon,
    };
    let ring = el().size(10.0, 10.0).radius(5.0).border(2.0, color);
    let ring = match ci {
        Ci::Runs => ring.cycle(rune::Cycle {
            period: 1.4,
            phase: 0.0,
            stops: &[(0.0, 1.0), (0.5, 0.3), (1.0, 1.0)],
            still: 1.0,
            fps: 30.0,
        }),
        _ => ring,
    };
    row().size(CI_MARK, CI_MARK).center().child(ring)
}

fn author_button(label: String, on: bool) -> Button {
    let variant = if on {
        ButtonVariant::Contrast
    } else {
        ButtonVariant::Outline
    };
    Button::new(label).variant(variant)
}

/// The width of the quick filter control.
fn quick_width(k: &Kit) -> f32 {
    let labels = Quick::ALL.map(Quick::label);
    let widest = labels
        .iter()
        .map(|l| rune::text::measure(l, &[], k.ty.ui, None).0)
        .fold(0.0, f32::max);
    labels.len() as f32 * (widest + 24.0)
}

/// The quick filter and the author buttons that fit in `room` points; the rest go in "+N".
fn filters(s: &App, k: &Kit, room: f32, layer: &mut Vec<El>) -> El {
    let labels = Quick::ALL.map(Quick::label);
    let at = Quick::ALL.iter().position(|q| *q == s.quick);
    let quick = SegmentedControl::new(&labels, at)
        .fit()
        .on_pick(|s: &mut App, i, _| s.set_quick(Quick::ALL[i]))
        .build(k)
        .help(labels[0])
        .help_keys(["1"])
        .help_row(labels[1])
        .help_keys(["2"])
        .help_row(labels[2])
        .help_keys(["3"]);
    let quick_width = quick_width(k);
    let clear = if s.picked.is_empty() {
        0.0
    } else {
        CLEAR_WIDTH
    };
    let groups = &s.settings.author_groups;
    let savable = !s.picked.is_empty() && !groups.values().any(|g| *g == s.picked);
    let width = |label: &str| rune::text::measure(label, &[], k.ty.ui_medium, None).0 + 24.0 + GAP;
    let pinned: f32 = groups.keys().map(|g| width(g)).sum::<f32>()
        + if savable { width(SAVE_GROUP) } else { 0.0 };
    let widths: Vec<f32> = s.authors.iter().map(|a| width(a)).collect();
    let mut budget = room - quick_width - GAP - clear - pinned;
    if widths.iter().sum::<f32>() > budget {
        budget -= MORE_WIDTH;
    }
    let mut used = 0.0;
    let fit = widths
        .iter()
        .take_while(|w| {
            used += *w;
            used <= budget
        })
        .count();

    let mut bar = row().gap(GAP).items_center().child(quick);
    for (name, group) in groups {
        let x = name.clone();
        let button = author_button(name.clone(), s.picked == *group)
            .on_press(move |s: &mut App| s.toggle_author_group(&x))
            .build(k)
            .key(format!("group-{name}"));
        bar = bar.child(button);
    }
    for a in &s.authors[..fit] {
        let name = a.clone();
        let button = author_button(a.clone(), s.picked.contains(a))
            .on_press(move |s: &mut App| s.toggle_author(&name))
            .build(k)
            .key(format!("author-{a}"));
        bar = bar.child(button);
    }
    let rest = &s.authors[fit..];
    if !rest.is_empty() {
        let picked = rest.iter().filter(|a| s.picked.contains(*a)).count();
        let label = match picked {
            0 => format!("+{}", rest.len()),
            n => format!("+{} ({n})", rest.len()),
        };
        let trigger = author_button(label, picked > 0)
            .expanded(s.more_authors)
            .on_press(|s: &mut App| s.more_authors = !s.more_authors)
            .build(k)
            .key("more-authors");
        let menu = rest.iter().fold(
            Menu::new("more-authors-menu")
                .trigger(trigger)
                .open(s.more_authors),
            |m, a| {
                let name = a.clone();
                m.item(
                    MenuItem::new(a.clone())
                        .checked(s.picked.contains(a))
                        .on_select(move |s: &mut App| s.toggle_author(&name)),
                )
            },
        );
        bar = bar.child(
            menu.on_close(|s: &mut App| s.more_authors = false)
                .build(k, layer),
        );
    }
    if savable {
        let save = Button::new(SAVE_GROUP)
            .variant(ButtonVariant::GhostMuted)
            .on_press(|s: &mut App| s.naming_group = true)
            .build(k)
            .key("save-group");
        bar = bar.child(save);
    }
    if !s.picked.is_empty() {
        let clear = Button::new("Clear")
            .variant(ButtonVariant::GhostMuted)
            .on_press(|s: &mut App| s.clear_authors())
            .build(k);
        bar = bar.child(clear);
    }
    bar.key("filters")
}

/// The CI button and its menu: the mark of the open PR, with the label "CI" when `labeled`.
fn ci_controls(s: &App, k: &Kit, labeled: bool, layer: &mut Vec<El>) -> El {
    let detail = s.detail.as_ref().and_then(|d| d.as_ref().ok());
    let ci = detail.and_then(|d| d.ci);
    let mark = move |k: &Kit| match ci {
        Some(ci) => ci_icon(ci, k),
        None => icon(&glyphs::DASH, 16.0).color(k.c.icon_faint),
    };
    let help = ci_help(ci);
    let kit = k.clone();
    let trigger = Button::new("CI")
        .variant(ButtonVariant::Outline)
        .help(help)
        .expanded(s.ci_menu)
        .on_press(|s: &mut App| s.ci_menu = !s.ci_menu)
        .label_with(move |font, ink| {
            let mut e = row().gap(6.0).items_center().child(mark(&kit));
            if labeled {
                e = e.child(text("CI").key("ci-label").font(font).color(ink));
            }
            e
        })
        .build(k)
        .key("ci-menu");
    let update = MenuItem::new("Update branch")
        .disabled(!s.can_update_branch())
        .on_select(|s: &mut App| {
            s.ci_menu = false;
            s.update_branch();
        });
    let update = match detail {
        Some(d) if !d.can_update_branch && d.conflicts => {
            update.description("Has conflicts, resolve them on GitHub")
        }
        Some(d) if !d.can_update_branch => {
            update.description("Up to date, or you cannot push to this branch")
        }
        _ => update,
    };
    Menu::new("ci")
        .trigger(trigger)
        .open(s.ci_menu)
        .min_width(CI_MENU_WIDTH)
        .item(
            MenuItem::new("Re-run failed jobs")
                .disabled(!s.can_run_ci())
                .on_select(|s: &mut App| {
                    s.ci_menu = false;
                    s.rerun(Rerun::Failed);
                }),
        )
        .item(
            MenuItem::new("Re-run all jobs")
                .disabled(!s.can_run_ci())
                .on_select(|s: &mut App| {
                    s.ci_menu = false;
                    s.rerun(Rerun::All);
                }),
        )
        .separator()
        .item(update)
        .on_close(|s: &mut App| s.ci_menu = false)
        .build(k, layer)
}

/// The toolbar's buttons on the right. In a `tight` toolbar the CI button shows its mark only,
/// and Review its word only.
fn review_controls(s: &App, k: &Kit, tight: bool, layer: &mut Vec<El>) -> El {
    let Some(pr) = s.current() else {
        return row().key("review-controls");
    };
    let drafts = s.current_drafts().len();
    let help = match (s.sending, drafts) {
        (true, _) => "Sending…".to_owned(),
        (false, 0) if s.can_review() && !s.can_approve() => "Approved".to_owned(),
        (false, 0) => "Approve".to_owned(),
        (false, 1) => "Approve and send your unsent line comment".to_owned(),
        (false, n) => format!("Approve and send your {n} unsent line comments"),
    };
    let approve = IconButton::new(&glyphs::CHECK)
        .variant(IconButtonVariant::Submit)
        .control_size(ControlSize::Large)
        .label("Approve")
        .help(help)
        .shortcut("shift+a")
        .disabled(!s.can_approve())
        .on_press(|s: &mut App| s.submit(ReviewState::Approved))
        .build(&approve_kit(k));
    // The count of unsent line comments sits over the corner of the tick, so it takes no width.
    let mut approve = el().key("approve").child(approve);
    if drafts > 0 && !s.sending {
        let count = Badge::new(drafts.to_string())
            .variant(BadgeVariant::Subtle)
            .build(k)
            .key("approve-drafts")
            .pin(Some(-6.0), Some(-6.0), None, None);
        approve = approve.child(count);
    }
    let request_changes = IconButton::new(&glyphs::CLOSE)
        .control_size(ControlSize::Large)
        .label("Request changes")
        .help("Request changes…")
        .disabled(!s.can_review())
        .on_press(|s: &mut App| s.open_composer(ReviewState::ChangesRequested))
        .build(k)
        .key("request-changes");
    let mut trigger = Button::new("Review").variant(ButtonVariant::Outline);
    if !tight {
        trigger = trigger.icon(&glyphs::CHEVRON_DOWN);
    }
    let trigger = trigger
        .expanded(s.review_menu)
        .on_press(|s: &mut App| s.review_menu = !s.review_menu)
        .build(k)
        .key("review-menu");
    let url = pr.url.clone();
    let copy = |label: &str, shortcut: &str, link: bool| {
        MenuItem::new(label)
            .shortcut(shortcut)
            .on_select_tx(move |s: &mut App, tx| {
                s.review_menu = false;
                s.copy_pr(link, tx);
            })
    };
    let menu =
        Menu::new("review")
            .trigger(trigger)
            .open(s.review_menu)
            .item(
                MenuItem::new("Comment…")
                    .shortcut("c")
                    .disabled(!s.can_review())
                    .on_select(|s: &mut App| s.open_composer(ReviewState::Commented)),
            )
            .item(
                MenuItem::new("Request changes…")
                    .disabled(!s.can_review())
                    .on_select(|s: &mut App| s.open_composer(ReviewState::ChangesRequested)),
            )
            .separator()
            .item(MenuItem::new("Open on GitHub").shortcut("o").on_select_tx(
                move |s: &mut App, tx| {
                    s.review_menu = false;
                    let _ = tx.open_url(&url);
                },
            ))
            .item(copy("Copy link", "cmd+shift+c", true))
            .item(copy("Copy number", "cmd+c", false))
            .on_close(|s: &mut App| s.review_menu = false)
            .build(k, layer);
    row()
        .key("review-controls")
        .gap(8.0)
        .items_center()
        .child(ci_controls(s, k, !tight, layer))
        .child(menu)
        .child(request_changes)
        .child(approve)
}

fn toast(s: &App, k: &Kit) -> Option<El> {
    let notice = s.toast.as_ref()?;
    let (glyph, color) = if notice.failed {
        (&glyphs::CIRCLE_X, k.c.danger.icon)
    } else {
        (&glyphs::CHECK, k.c.success.icon)
    };
    let t = Toast::new("review-toast", notice.text.clone())
        .lead(icon(glyph, 16.0).color(color))
        .on_close(|s: &mut App| s.toast = None)
        .build(k);
    Some(ToastRegion::new().toast(t).build(k))
}

/// Hint mode: a letter on each target on screen, in screen order. The keys of those targets
/// go to the app, so that a typed letter finds its target. A click leaves the mode.
fn hints(s: &App, cx: &mut Cx<App>, k: &Kit) -> Option<El> {
    if !s.hinting {
        return None;
    }
    let list = cx.layout("sidebar-scroll");
    let tree = cx.layout("files");
    let mut shown: Vec<(String, [f32; 4])> = keys::targets(s)
        .into_iter()
        .filter_map(|(key, hint)| {
            let r = cx.layout(&key)?;
            // A PR row or a file row shows its letter only when its list shows all of it.
            let within = match hint {
                Hint::Pr(_) if key != "base-pr" => Some(list),
                Hint::File(_) => Some(tree),
                _ => None,
            };
            let seen = match within {
                Some(Some(l)) => r[1] >= l[1] && r[1] + r[3] <= l[1] + l[3],
                Some(None) => false,
                None => r[2] > 0.0 && r[3] > 0.0,
            };
            seen.then_some((key, r))
        })
        .collect();
    shown.sort_by_key(|(_, r)| (r[1].round() as i32, r[0].round() as i32));
    let labels = keys::labels(shown.len());
    let mut layer = rune::el()
        .key("hints")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .on_press(|s: &mut App| s.stop_hints());
    for ((_, r), label) in shown.iter().zip(labels) {
        if !label.starts_with(&s.hint_typed) {
            continue;
        }
        let chip = row()
            .absolute(r[0], r[1])
            .h(16.0)
            .px(4.0)
            .radius(4.0)
            .center()
            .fill(k.c.warning.icon)
            .child(
                text(label.to_uppercase())
                    .font(k.ty.small_medium)
                    .color(k.c.bg_base),
            );
        layer = layer.child(chip);
    }
    let keys: Vec<String> = shown.into_iter().map(|(key, _)| key).collect();
    Some(layer.on_layout_tx(move |s: &mut App, _, tx| {
        if s.hint_keys == keys {
            tx.live_only();
        } else {
            s.hint_keys = keys.clone();
        }
    }))
}

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = kit(cx.system_dark());
    let mut layer = Vec::new();
    let width = |r: [f32; 4]| r[2];
    // The room of the filters with a tight toolbar. The full one, with the CI label and the
    // icon of Review, takes its part only when the quick filter and "+N" still fit.
    let extra = rune::text::measure("CI", &[], k.ty.ui_medium, None).0 + 6.0 + ICON_ROOM;
    let full = cx.layout_with("ci-label", |_| ()).is_some();
    let room = cx.layout_with("toolbar/leading", width).unwrap_or(0.0)
        + cx.layout_with("toolbar/spacer", width).unwrap_or(0.0)
        + if full { extra } else { 0.0 };
    // The form changes only past a margin, so a small error in `extra` cannot flip it each frame.
    let margin = if full { -SWITCH_MARGIN } else { SWITCH_MARGIN };
    let tight = room - extra < quick_width(&k) + GAP + MORE_WIDTH + margin;
    let room = room - if tight { 0.0 } else { extra };
    let toolbar = Toolbar::new()
        .leading(filters(s, &k, room, &mut layer))
        .trailing(review_controls(s, &k, tight, &mut layer));
    let main = cat::over(s, detail::view(s, cx, &k), &k);
    dialogs::build(s, &k, &mut layer);
    layer.extend(toast(s, &k));
    layer.extend(hints(s, cx, &k));
    let split = SplitView::new(main)
        .sidebar(
            &s.sidebar,
            |s: &mut App| &mut s.sidebar,
            drawer::view(s, &k, &mut layer),
        )
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .sidebar_toggle(SidebarToggle::new().in_header())
        .reduced_motion(cx.is_reduced_motion())
        .build(&k);
    rune_kit::root(split, layer)
}

#[cfg(test)]
mod tests {
    use rune::runtime::ElementInfo;
    use rune::{Env, Key, KeyEvent, Mods, Runtime, Tx};
    use rune_kit::diff::{LineRange, LineRef, Side};

    use super::*;
    use crate::store::Store;

    fn runtime(size: (f32, f32)) -> Runtime<App> {
        let mut app = App::new(Store::memory(), None);
        sample(&mut app);
        let mut rt = Runtime::new(app, view, size, Env::new(false, 2.0));
        rt.on_key = Some(crate::app::on_key);
        rt.frame(0.0);
        settle(&mut rt);
        rt
    }

    /// Frames until springs and layout reads have settled.
    fn settle(rt: &mut Runtime<App>) {
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn find(rt: &Runtime<App>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    /// The element keyed `key` inside the element keyed `scope`.
    fn find_in(rt: &Runtime<App>, scope: &str, key: &str) -> Option<ElementInfo> {
        rt.elements().into_iter().find(|e| {
            e.key.as_deref() == Some(key) && !e.exiting && e.path.split('/').any(|p| p == scope)
        })
    }

    fn click(rt: &mut Runtime<App>, e: &ElementInfo) {
        let x = e.rect[0] + e.rect[2] / 2.0;
        let y = e.rect[1] + e.rect[3] / 2.0;
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }

    fn typed(t: &str, shift: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Text(t.into()),
            mods: Mods {
                shift,
                ..Default::default()
            },
        }
    }

    fn ctrl_f() -> KeyEvent {
        KeyEvent {
            key: Key::Command('f'),
            mods: Mods {
                ctrl: true,
                ..Default::default()
            },
        }
    }

    fn press(rt: &mut Runtime<App>, ev: KeyEvent) {
        rt.key(ev);
        rt.frame(1.0 / 60.0);
    }

    fn type_text(rt: &mut Runtime<App>, text: &str) {
        for c in text.chars() {
            press(rt, typed(&c.to_string(), false));
        }
    }

    fn approval_sent(rt: &Runtime<App>) -> bool {
        let sent = rt.state.net.sent.lock().unwrap();
        sent.iter().any(|r| {
            r.body
                .as_ref()
                .is_some_and(|b| b.contains("addPullRequestReview"))
        })
    }

    /// The drawer rows on screen, by PR id, with their top edge.
    fn drawer_rows(rt: &Runtime<App>) -> Vec<(String, f32)> {
        let ids: Vec<String> = rt.state.prs.iter().map(|p| p.id.clone()).collect();
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting && e.path.contains("sidebar-list"))
            .filter_map(|e| e.key.filter(|k| ids.contains(k)).map(|k| (k, e.rect[1])))
            .filter(|(_, y)| (0.0..800.0).contains(y))
            .collect()
    }

    #[test]
    fn the_drawer_stacks_cards_until_the_first_pr_arrives() {
        let mut rt = Runtime::new(
            App::new(Store::memory(), None),
            view,
            (1400.0, 800.0),
            Env::new(false, 2.0),
        );
        rt.poll = Some(crate::app::poll);
        rt.motion_needs_cause();
        rt.frame(0.0);
        let frames = |rt: &mut Runtime<App>, seconds: f32| {
            for _ in 0..(seconds * 60.0) as usize {
                rt.frame(1.0 / 60.0);
            }
        };
        frames(&mut rt, 0.4);
        assert!(find(&rt, "boot-card-1-0").is_some(), "the first stack");
        let last = find(&rt, "boot-card-1-5").unwrap();
        assert!(last.opacity < 0.1, "the last card waits for its turn");
        frames(&mut rt, 2.0);
        assert!(
            find(&rt, "boot-card-2-0").is_some(),
            "the stack starts again"
        );

        sample(&mut rt.state);
        rt.state.loading = true;
        rt.invalidate();
        settle(&mut rt);
        let cards = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with("boot-card")));
        assert_eq!(cards.count(), 0, "the cards go");
        assert!(
            find(&rt, &rt.state.prs[0].id.clone()).is_some(),
            "the PR row shows"
        );
    }

    #[test]
    fn picking_a_pr_keeps_the_drawer_list_and_its_scroll() {
        let mut rt = runtime((1280.0, 800.0));
        rt.wheel(150.0, 500.0, -400.0);
        settle(&mut rt);
        let before = drawer_rows(&rt);
        assert!(before.len() > 5, "the drawer lists PRs: {before:?}");
        assert_ne!(before[0].0, "PR_4210", "the drawer scrolled");

        let (id, _) = before[before.len() / 2].clone();
        let row = find(&rt, &id).unwrap();
        let (x, y) = (
            row.rect[0] + row.rect[2] / 2.0,
            row.rect[1] + row.rect[3] / 2.0,
        );
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(&mut rt);

        assert_eq!(rt.state.selected.as_deref(), Some(id.as_str()));
        assert_eq!(drawer_rows(&rt), before);
    }

    #[test]
    fn typed_shortcuts_work_after_a_click_on_a_drawer_row() {
        let mut rt = runtime((1280.0, 800.0));
        let row = find(&rt, "PR_4210").unwrap();
        let x = row.rect[0] + 20.0;
        let y = row.rect[1] + row.rect[3] / 2.0;
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        press(&mut rt, typed("f", false));
        press(&mut rt, typed("A", true));

        assert_eq!(rt.state.selected.as_deref(), Some("PR_4210"));
        assert!(!rt.state.conversation_open, "f folds the conversation");
        assert!(approval_sent(&rt), "Shift+A sends the approval");

        let mut rt = runtime((1280.0, 800.0));
        press(&mut rt, typed("/", false));
        type_text(&mut rt, "jitter");
        settle(&mut rt);
        let (id, _) = drawer_rows(&rt)[1].clone();
        let row = find(&rt, &id).unwrap();
        click(&mut rt, &row);
        assert_eq!(rt.state.selected.as_deref(), Some(id.as_str()));
        press(&mut rt, typed("j", false));
        assert_ne!(
            rt.state.selected.as_deref(),
            Some(id.as_str()),
            "j moves the selection after a click on a row the search kept"
        );
    }

    #[test]
    fn the_file_and_hunk_keys_open_the_file_they_go_to() {
        let mut rt = runtime((1400.0, 800.0));
        press(&mut rt, typed("z", false));
        settle(&mut rt);
        let folded = |rt: &Runtime<App>| -> Vec<bool> {
            (0..5).map(|f| rt.state.diff.collapsed(f)).collect()
        };
        assert_eq!(
            folded(&rt),
            [true; 5],
            "every file folded, as in a new diff"
        );

        press(&mut rt, typed("n", false));
        settle(&mut rt);
        assert!(!rt.state.diff.collapsed(0), "n opens the file at the top");
        for _ in 0..5 {
            if rt.state.diff.visible_file() == 1 {
                break;
            }
            press(&mut rt, typed("n", false));
            settle(&mut rt);
        }
        assert_eq!(
            rt.state.diff.visible_file(),
            1,
            "n goes on to the next file"
        );
        assert!(!rt.state.diff.collapsed(1), "n opens the next file");

        press(&mut rt, typed("]", false));
        settle(&mut rt);
        assert!(!rt.state.diff.collapsed(2), "] opens the file it goes to");
        assert_eq!(folded(&rt)[3..], [true, true], "no other file opens");

        // The back keys on the state: the sample diff is too short to scroll its last files to
        // the top. The first file is open, the others are folded.
        let mut app = App::new(Store::memory(), None);
        sample(&mut app);
        let mut tx = Tx::default();
        let mut key_at = |app: &mut App, file: usize, key: &str| {
            app.diff.scroll_to_file(file, &mut tx);
            crate::app::on_key(app, &typed(key, false), &mut tx);
        };
        key_at(&mut app, 2, "p");
        assert_eq!(app.diff.visible_file(), 0, "p goes back to the open hunk");
        assert!(app.diff.collapsed(1), "p opens no folded file it passes");
        app.diff.set_collapsed(0, true);
        key_at(&mut app, 2, "p");
        assert_eq!(app.diff.visible_file(), 1);
        assert!(
            !app.diff.collapsed(1),
            "p with no hunk left opens the file before"
        );
        key_at(&mut app, 4, "[");
        assert_eq!(app.diff.visible_file(), 3);
        assert!(!app.diff.collapsed(3), "[ opens the folded file it goes to");
        app.follow_file(4);
        assert!(
            app.diff.collapsed(4),
            "the tree follow of a wheel scroll opens no file"
        );
    }

    #[test]
    fn a_folded_file_shows_the_count_of_its_threads_and_unsent_comments() {
        let mut rt = runtime((1400.0, 800.0));
        press(&mut rt, typed("z", false));
        settle(&mut rt);
        assert!((0..5).all(|f| rt.state.diff.collapsed(f)));
        let notes = |rt: &Runtime<App>, f: usize| -> Vec<String> {
            // The diff keys the row of file `f` `df<f>`, and its header actions `actions`.
            let scope = format!("/df{f}/inner/actions/");
            rt.elements()
                .into_iter()
                .filter(|e| !e.exiting && e.path.contains(&scope))
                .filter_map(|e| e.text)
                .collect()
        };
        assert_eq!(
            notes(&rt, 0),
            ["2 open threads"],
            "src/net/retry.rs, one of them outdated"
        );
        assert_eq!(
            notes(&rt, 1),
            ["1 open thread"],
            "src/net/pool.rs, an outdated one beside a resolved one"
        );
        assert_eq!(notes(&rt, 2), ["1 open thread"], "src/lib.rs");
        assert_eq!(notes(&rt, 3), ["1 unsent"], "config/retry.toml");
    }

    #[test]
    fn a_row_names_your_verdict_and_marks_new_commits() {
        let mut rt = runtime((1400.0, 1200.0));
        rt.state.set_quick(Quick::All);
        rt.invalidate();
        settle(&mut rt);
        let row = |rt: &Runtime<App>, i: usize| {
            let id = rt.state.prs[i].id.clone();
            let words = texts_in(rt, &id).join(" ");
            (words, find_in(rt, &id, "new-commits").is_some())
        };
        let (words, dot) = row(&rt, 4);
        assert!(
            words.contains("You approved"),
            "approved before new commits: {words}"
        );
        assert!(dot, "new commits since the approval");
        let (words, dot) = row(&rt, 7);
        assert!(
            words.contains("You approved"),
            "approved at the head: {words}"
        );
        assert!(!dot, "no new commits");
    }

    /// The texts inside the element keyed `scope`.
    fn texts_in(rt: &Runtime<App>, scope: &str) -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting && e.path.split('/').any(|p| p == scope))
            .filter_map(|e| e.text)
            .collect()
    }

    #[test]
    fn the_threads_button_shows_the_open_count_and_fits_the_bar_at_900_points() {
        let mut rt = runtime((900.0, 700.0));
        assert!(rt.state.sidebar.is_open());
        assert_eq!(
            texts_in(&rt, "threads-button"),
            ["4"],
            "a narrow pane leaves out the word"
        );
        let fits = |rt: &Runtime<App>| {
            let bar = find(rt, "files-bar").unwrap();
            let right = bar.rect[0] + bar.rect[2];
            for e in rt
                .elements()
                .into_iter()
                .filter(|e| !e.exiting && e.path.contains("/files-bar/"))
            {
                assert!(
                    e.rect[0] >= bar.rect[0] && e.rect[0] + e.rect[2] <= right + 0.5,
                    "{:?} {:?} leaves the bar {:?}",
                    e.key,
                    e.rect,
                    bar.rect
                );
            }
        };
        fits(&rt);

        let button = find(&rt, "threads-button").unwrap();
        click(&mut rt, &button);
        assert_eq!(
            rt.state.thread_at.as_deref(),
            Some("RT_4"),
            "a click does what u does: the outdated thread that moved to line 7 comes first"
        );

        if let Some(Ok(d)) = rt.state.detail.as_mut() {
            d.threads.iter_mut().for_each(|t| t.resolved = true);
        }
        rt.invalidate();
        settle(&mut rt);
        assert_eq!(texts_in(&rt, "threads-button"), ["0"]);
        rt.state.sidebar.toggle();
        rt.invalidate();
        settle(&mut rt);
        assert_eq!(
            texts_in(&rt, "threads-button"),
            ["Threads", "0"],
            "with the drawer closed"
        );
        fits(&rt);
        let button = find(&rt, "threads-button").unwrap();
        assert!(!button.focusable, "off with no open thread");
    }

    #[test]
    fn a_resolved_thread_is_one_line_and_resolve_shows_only_when_github_allows() {
        let mut rt = runtime((1400.0, 900.0));
        rt.state.conversation_open = false;
        for f in 0..5 {
            rt.state.diff.set_collapsed(f, f != 1 && f != 2);
        }
        rt.invalidate();
        settle(&mut rt);
        assert!(
            find_in(&rt, "note-t2", "resolve").is_none(),
            "the viewer may not resolve it"
        );
        assert!(find_in(&rt, "note-t1", "resolve").is_none(), "folded");
        let folded = find(&rt, "note-t1").unwrap();
        assert!(folded.rect[3] < 80.0, "one line: {:?}", folded.rect);
        assert!(folded.opacity < 1.0, "faint while folded");
        let below = |rt: &Runtime<App>| find(rt, "note-t2").unwrap().rect[1];
        let before = below(&rt);

        let e = find_in(&rt, "note-t1", "trigger").unwrap();
        click(&mut rt, &e);
        assert!(rt.state.opened_threads.contains("RT_2"));
        assert_eq!(
            find(&rt, "note-t1").unwrap().opacity,
            1.0,
            "plain once opened"
        );
        let unresolve = find_in(&rt, "note-t1", "resolve").expect("Unresolve on the opened thread");
        assert!(below(&rt) > before + 20.0, "the rows below move down");
        sent(&rt);
        click(&mut rt, &unresolve);
        let reqs = sent(&rt);
        assert_eq!(reqs.len(), 1);
        assert!(
            reqs[0]
                .body
                .as_ref()
                .unwrap()
                .contains("unresolveReviewThread")
        );

        let e = find_in(&rt, "note-t1", "trigger").unwrap();
        click(&mut rt, &e);
        assert!(below(&rt) <= before + 0.5, "folds again");

        rt.state.diff.set_collapsed(0, false);
        rt.invalidate();
        settle(&mut rt);
        assert!(
            find_in(&rt, "note-t0", "resolve").is_some(),
            "Resolve on an open thread"
        );
    }

    #[test]
    fn a_link_in_a_comment_opens_in_the_browser() {
        let url = "https://example.com/doc";
        let mut rt = runtime((1400.0, 900.0));
        rt.state.conversation_open = false;
        if let Some(Ok(d)) = rt.state.detail.as_mut() {
            d.threads[0].comments[0].body = url.into();
        }
        for f in 0..5 {
            rt.state.diff.set_collapsed(f, f != 0);
        }
        rt.invalidate();
        settle(&mut rt);
        let c = find_in(&rt, "note-t0", "c0").unwrap();
        let x = c.rect[0] + 20.0;
        let y = c.rect[1] + c.rect[3] - 8.0;
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(&mut rt);
        assert_eq!(rt.take_url_requests(), [url]);
    }

    fn sent(rt: &Runtime<App>) -> Vec<crate::github::Request> {
        std::mem::take(&mut *rt.state.net.sent.lock().unwrap())
    }

    #[test]
    fn the_x_left_of_approve_opens_the_request_changes_composer() {
        let mut rt = runtime((900.0, 700.0));
        let x = find(&rt, "request-changes").unwrap();
        assert!(x.rect[0] < find(&rt, "approve").unwrap().rect[0]);
        click(&mut rt, &x);
        assert_eq!(rt.state.review, Some(ReviewState::ChangesRequested));
    }

    #[test]
    fn a_click_on_the_base_pr_opens_it_when_a_filter_hides_its_row() {
        let mut rt = runtime((1400.0, 800.0));
        let listed = |rt: &Runtime<App>| drawer_rows(rt).iter().any(|(id, _)| id == "PR_4207");
        press(&mut rt, typed("2", false));
        settle(&mut rt);
        assert!(!listed(&rt), "Acted on hides the base PR");

        let link = find_in(&rt, "pr-header", "base-pr").expect("a link to the base PR");
        let label = rt.elements().into_iter().find_map(|e| {
            (e.path.split('/').any(|p| p == "base-pr") && !e.exiting)
                .then_some(e.text)
                .flatten()
        });
        assert_eq!(label.as_deref(), Some("#4207"));
        click(&mut rt, &link);

        assert_eq!(rt.state.selected.as_deref(), Some("PR_4207"));
        assert_eq!(
            rt.state.quick,
            crate::model::Quick::ActedOn,
            "no filter changes"
        );
        assert!(listed(&rt), "the drawer lists the open PR");
    }

    #[test]
    fn the_key_sheet_names_only_keys_that_work() {
        let mut rt = runtime((1280.0, 800.0));
        // Away from the top, so that the keys that go back have somewhere to go.
        press(&mut rt, typed("]", false));
        press(&mut rt, typed("n", false));
        settle(&mut rt);
        // The diff keys go first: the PR keys open another PR.
        for (_, lines) in crate::keys::SHEET.iter().rev() {
            for (caps, words) in lines.iter() {
                let (last, mods) = caps.split_last().unwrap();
                let shift = mods == ["Shift"];
                if !(mods.is_empty() || shift) || last.chars().count() != 1 {
                    continue;
                }
                // Closes what the key before opened, so that this key reaches the handler.
                let s = &mut rt.state;
                s.keys_open = false;
                s.review = None;
                s.pr_search.focused = false;
                s.tree_search.focused = false;
                s.find_edit.focused = false;
                s.pr_comment.focused = false;
                let ev = typed(&last.to_lowercase(), shift);
                let done = crate::app::on_key(s, &ev, &mut Tx::default());
                assert!(done, "{caps:?} ({words}) does nothing");
                rt.frame(1.0 / 60.0);
            }
        }
    }

    #[test]
    fn a_hint_letter_opens_its_pr_and_ends_hint_mode() {
        let mut rt = runtime((1400.0, 800.0));
        press(&mut rt, typed(" ", false));
        settle(&mut rt);
        let keys = rt.state.hint_keys.clone();
        let s = &rt.state;
        let off_screen = s.shown.iter().any(|&i| !keys.contains(&s.prs[i].id));
        assert!(off_screen, "rows off screen get no letter");
        let target = drawer_rows(&rt)[1].0.clone();
        let at = keys.iter().position(|k| *k == target).expect("a letter");
        for c in crate::keys::labels(keys.len())[at].chars() {
            press(&mut rt, typed(&c.to_string(), false));
        }
        settle(&mut rt);
        assert_eq!(rt.state.selected.as_deref(), Some(target.as_str()));
        assert!(!rt.state.hinting);
    }

    #[test]
    fn up_jumps_while_no_pr_is_open_and_steps_the_list_otherwise() {
        let mut rt = runtime((1400.0, 900.0));
        let open = rt.state.selected.take();
        rt.invalidate();
        settle(&mut rt);
        assert!(
            find(&rt, "cat-game").is_some(),
            "the cat plays in the empty pane"
        );
        press(&mut rt, named(Key::Up));
        assert_eq!(
            rt.state.cat.mode,
            crate::cat::Mode::Play,
            "Up took over the cat"
        );
        assert!(rt.state.selected.is_none(), "Up opened no PR");

        rt.state.selected = open.clone();
        rt.invalidate();
        settle(&mut rt);
        assert!(find(&rt, "cat-game").is_none(), "a PR replaces the game");
        press(&mut rt, named(Key::Down));
        assert_ne!(rt.state.selected, open, "Down stepped to the next PR");
    }

    /// A runtime with the clock running and the peek due now.
    fn peek_runtime() -> Runtime<App> {
        let mut rt = runtime((1400.0, 900.0));
        rt.poll = Some(crate::app::poll);
        rt.state.peek_due = Some(0.0);
        rt
    }

    fn frames(rt: &mut Runtime<App>, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            rt.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn the_cat_peeks_out_and_hides_after_six_seconds() {
        let mut rt = peek_runtime();
        frames(&mut rt, 0.5);
        assert!(find(&rt, "peek-cat").is_some(), "the cat peeks out");
        frames(&mut rt, 6.0);
        assert!(find(&rt, "peek-cat").is_none(), "the cat hides again");
    }

    #[test]
    fn a_click_on_the_peeking_cat_opens_the_game_over_the_pr() {
        let mut rt = peek_runtime();
        frames(&mut rt, 0.5);
        let open = rt.state.selected.clone();
        assert!(open.is_some());
        let cat = find(&rt, "peek-cat").expect("the cat peeks out");
        click(&mut rt, &cat);
        assert!(find(&rt, "cat-game").is_some(), "the game opens");
        assert_eq!(rt.state.selected, open, "the PR stays open under it");

        press(&mut rt, named(Key::Escape));
        settle(&mut rt);
        assert!(find(&rt, "cat-game").is_none(), "Escape hides the game");
        assert_eq!(rt.state.selected, open);
    }

    #[test]
    fn a_click_on_the_winking_head_starts_the_game_while_the_list_loads() {
        let app = App::new(Store::memory(), None);
        let mut rt = Runtime::new(app, view, (1400.0, 900.0), Env::new(false, 2.0));
        rt.frame(0.0);
        settle(&mut rt);
        assert!(find(&rt, "cat-game").is_none());
        let head = find(&rt, "cat-head").expect("the head shows while the list loads");
        click(&mut rt, &head);
        assert!(find(&rt, "cat-game").is_some(), "the click starts the game");
    }

    fn named(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Mods::default(),
        }
    }

    /// The file under the tree's keyboard cursor, and whether the tree has focus.
    fn tree_cursor(rt: &Runtime<App>) -> (Option<String>, bool) {
        let focused = find(rt, "files").is_some_and(|e| e.focused);
        (rt.state.tree.cursor().map(str::to_owned), focused)
    }

    #[test]
    fn down_from_the_file_filter_walks_the_tree_and_enter_opens_the_file() {
        let mut rt = runtime((1400.0, 800.0));
        rt.state.conversation_open = false;
        rt.invalidate();
        settle(&mut rt);
        press(&mut rt, typed("T", true));
        type_text(&mut rt, "retry");
        settle(&mut rt);
        press(&mut rt, named(Key::Down));
        settle(&mut rt);
        let (first, focused) = tree_cursor(&rt);
        assert!(focused, "Down moves into the tree");
        assert_eq!(
            first.as_deref(),
            Some("config/retry.toml"),
            "the first match"
        );

        // The arrows step through the folder rows too.
        press(&mut rt, named(Key::Down));
        press(&mut rt, named(Key::Down));
        settle(&mut rt);
        let (second, focused) = tree_cursor(&rt);
        assert!(focused, "an arrow keeps the focus in the tree");
        assert_eq!(second.as_deref(), Some("docs/retry.md"));

        press(&mut rt, named(Key::Enter));
        settle(&mut rt);
        let file = rt.state.diff.file_index("docs/retry.md").unwrap();
        assert!(!rt.state.diff.collapsed(file), "the file is open");
        assert!(!tree_cursor(&rt).1, "Enter leaves the tree");
    }

    #[test]
    fn a_hint_letter_on_a_file_row_opens_the_file() {
        let mut rt = runtime((1400.0, 800.0));
        rt.state.conversation_open = false;
        rt.invalidate();
        settle(&mut rt);
        press(&mut rt, typed(" ", false));
        settle(&mut rt);
        let keys = rt.state.hint_keys.clone();
        let at = keys
            .iter()
            .position(|k| k == "src/lib.rs")
            .expect("a letter on the file row");
        for c in crate::keys::labels(keys.len())[at].chars() {
            press(&mut rt, typed(&c.to_string(), false));
        }
        settle(&mut rt);
        let file = rt.state.diff.file_index("src/lib.rs").unwrap();
        assert!(!rt.state.diff.collapsed(file), "the file opens");
        assert!(!rt.state.hinting);
    }

    #[test]
    fn approve_is_on_screen_with_the_drawer_closed() {
        let mut rt = runtime((900.0, 700.0));
        let check = |rt: &Runtime<App>| {
            let approve = find(rt, "approve").expect("an Approve button");
            let right = approve.rect[0] + approve.rect[2];
            assert!(
                approve.rect[0] >= 0.0 && right <= 900.0,
                "on screen: {:?}",
                approve.rect
            );
            assert!(right > 900.0 - 40.0, "at the right end: {:?}", approve.rect);
            let filters = find(rt, "filters").expect("the filters");
            let review = find(rt, "review-controls").expect("the review controls");
            assert!(
                filters.rect[0] + filters.rect[2] <= review.rect[0],
                "the filters end before the review controls: {:?} {:?}",
                filters.rect,
                review.rect
            );
            let ci = find(rt, "ci-menu").expect("a CI button");
            let menu = find(rt, "review-menu").expect("the Review button");
            assert!(
                filters.rect[0] + filters.rect[2] <= ci.rect[0]
                    && ci.rect[0] + ci.rect[2] <= menu.rect[0],
                "the CI button sits between the filters and Review: {:?} {:?} {:?}",
                filters.rect,
                ci.rect,
                menu.rect
            );
        };
        check(&rt);
        rt.state.sidebar.toggle();
        rt.invalidate();
        settle(&mut rt);
        assert!(!rt.state.sidebar.is_open());
        check(&rt);
    }

    #[test]
    fn a_click_on_a_tree_file_shows_that_file_and_keeps_the_shortcuts() {
        let mut rt = runtime((1400.0, 800.0));
        let file = rt.state.diff.file_index("src/net/pool.rs").unwrap();
        rt.state.diff.set_collapsed(file, true);
        rt.invalidate();
        settle(&mut rt);

        let row = find_in(&rt, "files", "src/net/pool.rs").expect("a tree row for the file");
        click(&mut rt, &row);

        assert!(
            !rt.state.diff.collapsed(file),
            "the click opens the folded file"
        );
        assert_eq!(
            rt.state.diff.visible_file(),
            file,
            "the diff shows the file"
        );
        press(&mut rt, typed("j", false));
        assert_ne!(
            rt.state.selected.as_deref(),
            Some("PR_4210"),
            "j still moves to the next PR"
        );

        let loading = rt.state.selected.clone();
        press(&mut rt, typed("T", true));
        press(&mut rt, ctrl_f());
        press(&mut rt, typed("j", false));
        assert_ne!(
            rt.state.selected, loading,
            "Shift+T and Ctrl+F with no diff on screen keep j"
        );

        let mut rt = runtime((1400.0, 800.0));
        let folder = find_in(&rt, "files", "docs").expect("a tree row for the folder");
        click(&mut rt, &folder);
        press(&mut rt, typed("j", false));
        assert_ne!(
            rt.state.selected.as_deref(),
            Some("PR_4210"),
            "j still moves to the next PR after a click on a folder"
        );
    }

    #[test]
    fn a_file_row_of_the_tree_shows_its_count_of_open_threads() {
        let rt = runtime((1400.0, 800.0));
        let badge = |path: &str| -> Option<String> {
            let row = format!("/rows/{path}/badge");
            rt.elements()
                .into_iter()
                .find(|e| !e.exiting && e.path.ends_with(&row))
                .and_then(|e| e.text)
        };
        assert_eq!(
            badge("src/net/retry.rs").as_deref(),
            Some("2"),
            "one of them outdated"
        );
        assert_eq!(
            badge("src/net/pool.rs").as_deref(),
            Some("1"),
            "its resolved thread left out"
        );
        assert_eq!(
            badge("config/retry.toml"),
            None,
            "an unsent comment is no thread"
        );
    }

    #[test]
    fn the_tree_shows_only_when_the_diff_keeps_its_room() {
        let mut rt = runtime((900.0, 700.0));
        assert!(
            find(&rt, "tree-column").is_none(),
            "no tree in a 560 point pane"
        );
        let diff = find(&rt, "diff").expect("the diff");
        assert!(
            diff.rect[2] > 500.0,
            "the diff takes the pane: {:?}",
            diff.rect
        );

        rt.state.sidebar.toggle();
        rt.invalidate();
        settle(&mut rt);
        assert!(
            find(&rt, "tree-column").is_some(),
            "the tree shows with the drawer closed"
        );

        let mut rt = runtime((1100.0, 800.0));
        assert!(
            find(&rt, "tree-column").is_some(),
            "a tree of 240 points in a 760 point pane"
        );
        rt.state.tree_width.set_width(400.0);
        rt.invalidate();
        settle(&mut rt);
        let tree = find(&rt, "tree-column").expect("a saved tree of 400 points still shows");
        let diff = find(&rt, "diff").expect("the diff");
        assert!(tree.rect[2] < 400.0, "narrowed: {:?}", tree.rect);
        assert!(
            diff.rect[2] >= crate::app::DIFF_ROOM - 1.0,
            "the diff keeps its room: {:?}",
            diff.rect
        );
        assert_eq!(rt.state.tree_width.width(), 400.0, "the saved width stays");

        let rt = runtime((1400.0, 800.0));
        let tree = find(&rt, "tree-column").expect("the tree in a wide pane");
        let diff = find(&rt, "diff").expect("the diff");
        assert!(
            (tree.rect[2] - rt.state.tree_width.default_width()).abs() < 1.0,
            "{:?}",
            tree.rect
        );
        assert!(tree.rect[0] + tree.rect[2] <= diff.rect[0] + 0.5);
        assert!(
            diff.rect[2] >= 600.0,
            "the diff keeps its room: {:?}",
            diff.rect
        );
    }

    /// Drags the grip of the tree `dx` points.
    fn drag_grip(rt: &mut Runtime<App>, dx: f32) {
        let grip = find(rt, "tree-grip").expect("a grip on the tree edge");
        let x = grip.rect[0] + grip.rect[2] / 2.0;
        let y = grip.rect[1] + grip.rect[3] / 2.0;
        rt.pointer_move(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(x + dx / 2.0, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(x + dx, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_up(x + dx, y);
        settle(rt);
    }

    #[test]
    fn a_drag_on_the_tree_edge_sets_its_width_and_leaves_the_diff_its_room() {
        let width = |rt: &Runtime<App>| find(rt, "tree-column").expect("the tree").rect[2];
        let mut rt = runtime((1400.0, 800.0));
        drag_grip(&mut rt, 100.0);
        assert!((width(&rt) - 340.0).abs() < 1.0, "{}", width(&rt));
        assert_eq!(
            rt.state.settings.tree_width,
            Some(340.0),
            "saved when the drag ends"
        );
        drag_grip(&mut rt, -400.0);
        assert!(
            (width(&rt) - 180.0).abs() < 1.0,
            "the minimum: {}",
            width(&rt)
        );

        let grip = find(&rt, "tree-grip").unwrap();
        let (x, y) = (grip.rect[0] + grip.rect[2] / 2.0, grip.rect[1] + 20.0);
        for _ in 0..2 {
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(1.0 / 60.0);
        }
        settle(&mut rt);
        assert!(
            (width(&rt) - 240.0).abs() < 1.0,
            "a double click puts the default back: {}",
            width(&rt)
        );

        let mut rt = runtime((900.0, 700.0));
        rt.state.sidebar.toggle();
        rt.invalidate();
        settle(&mut rt);
        drag_grip(&mut rt, 300.0);
        let diff = find(&rt, "diff").expect("the diff");
        assert!(width(&rt) > 300.0, "the tree widens: {}", width(&rt));
        assert!(
            diff.rect[2] >= crate::app::DIFF_ROOM - 1.0,
            "the diff keeps its room: {:?}",
            diff.rect
        );
    }

    #[test]
    fn text_typed_in_the_pr_search_narrows_the_list_and_runs_no_app_key() {
        let mut rt = runtime((1280.0, 800.0));
        let before = drawer_rows(&rt).len();
        press(&mut rt, typed("/", false));
        settle(&mut rt);
        assert!(rt.state.pr_search.focused, "/ puts the cursor in the field");

        type_text(&mut rt, "jitter");
        settle(&mut rt);
        assert_eq!(rt.state.pr_search.text, "jitter");
        assert_eq!(
            rt.state.selected.as_deref(),
            Some("PR_4210"),
            "j moves no row"
        );
        let rows = drawer_rows(&rt);
        assert!(rows.len() < before, "the list narrows: {rows:?}");
        for (id, _) in &rows {
            let pr = rt.state.pr(id).unwrap();
            assert!(pr.title.contains("jitter"), "{} is listed", pr.title);
        }

        press(&mut rt, typed("A", true));
        assert!(
            !approval_sent(&rt),
            "Shift+A in the field sends no approval"
        );
        assert_eq!(rt.state.pr_search.text, "jitterA");
        press(&mut rt, ctrl_f());
        assert!(!rt.state.find_open, "Ctrl+F in the field runs no app key");
    }

    #[test]
    fn enter_in_find_shows_the_next_match_and_opens_a_folded_file() {
        let mut rt = runtime((1400.0, 800.0));
        let toml = rt.state.diff.file_index("config/retry.toml").unwrap();
        rt.state.diff.set_collapsed(toml, true);
        rt.invalidate();
        settle(&mut rt);

        press(&mut rt, ctrl_f());
        settle(&mut rt);
        type_text(&mut rt, "max_ms");
        press(
            &mut rt,
            KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            },
        );
        settle(&mut rt);

        assert!(!rt.state.diff.collapsed(toml), "the match opens its file");
        assert_eq!(
            rt.state.diff.selection(),
            Some(LineRange::line(toml, LineRef::new(Side::New, 3)))
        );
        let diff = find(&rt, "diff").expect("the diff");
        let shown = rt.elements().into_iter().any(|e| {
            !e.exiting
                && e.text.as_deref().is_some_and(|t| t.contains("max_ms"))
                && e.visible[3] > 0.0
                && e.visible[1] >= diff.rect[1]
                && e.visible[1] < diff.rect[1] + diff.rect[3]
        });
        assert!(shown, "the matched line is on screen");
    }

    #[test]
    fn a_click_on_a_repo_header_folds_its_group() {
        let mut rt = runtime((1280.0, 800.0));
        let repo_of = |rt: &Runtime<App>, id: &str| rt.state.pr(id).unwrap().repo.clone();
        let before = drawer_rows(&rt);
        assert!(before.iter().any(|(id, _)| repo_of(&rt, id) == "acme/web"));

        let header = find(&rt, "repo-acme/web").expect("a header for acme/web");
        click(&mut rt, &header);

        let after = drawer_rows(&rt);
        assert!(
            after.iter().all(|(id, _)| repo_of(&rt, id) != "acme/web"),
            "{after:?}"
        );
        assert!(
            after.iter().any(|(id, _)| repo_of(&rt, id) == "acme/api"),
            "{after:?}"
        );
        assert!(rt.state.settings.folded.contains("acme/web"));
    }

    fn escape() -> KeyEvent {
        KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        }
    }

    /// Whether the element keyed `key` shows inside the conversation's scroll.
    fn in_conversation(rt: &Runtime<App>, key: &str) -> bool {
        let (Some(e), Some(c)) = (find(rt, key), find(rt, "conversation")) else {
            return false;
        };
        let (top, bottom) = (e.visible[1], e.visible[1] + e.visible[3]);
        e.visible[3] > 0.0 && top >= c.rect[1] - 0.5 && bottom <= c.rect[1] + c.rect[3] + 0.5
    }

    #[test]
    fn m_opens_the_comment_box_and_cmd_enter_sends_on_my_own_pr() {
        let mut rt = runtime((1400.0, 600.0));
        rt.state.me = "ann".into();
        rt.state.conversation_open = false;
        rt.invalidate();
        settle(&mut rt);
        press(&mut rt, typed("/", false));
        type_text(&mut rt, "m");
        assert!(!rt.state.conversation_open, "m in the search types an m");
        press(&mut rt, escape());
        settle(&mut rt);

        press(&mut rt, typed("m", false));
        settle(&mut rt);
        assert!(rt.state.conversation_open && rt.state.pr_comment.focused);
        assert!(
            in_conversation(&rt, "pr-comment"),
            "the box on my own PR, scrolled into view"
        );
        type_text(&mut rt, "jk");
        assert_eq!(
            rt.state.selected.as_deref(),
            Some("PR_4210"),
            "j moves no row"
        );
        assert_eq!(rt.state.pr_comment.text, "jk");
        press(&mut rt, escape());
        settle(&mut rt);
        assert!(!rt.state.pr_comment.focused, "Escape leaves the box");
        assert_eq!(rt.state.pr_comment.text, "jk", "and keeps the text");

        press(&mut rt, typed("m", false));
        settle(&mut rt);
        sent(&rt);
        press(
            &mut rt,
            KeyEvent {
                key: Key::Enter,
                mods: Mods {
                    cmd: true,
                    ..Default::default()
                },
            },
        );
        settle(&mut rt);
        let reqs = sent(&rt);
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_deref().unwrap();
        assert!(
            body.contains("addComment(") && body.contains("\"subjectId\":\"PR_4210\""),
            "{body}"
        );
        let send = find(&rt, "pr-comment-send").unwrap();
        assert!(!send.focusable, "off while the answer is on the way");
    }

    #[test]
    fn the_threads_switch_shows_the_list_and_a_comment_row_scrolls_the_conversation() {
        let mut rt = runtime((1400.0, 600.0));
        let min = rt.state.tree_width.min_width();
        rt.state.tree_width.set_width(min);
        rt.state.conversation_open = false;
        rt.invalidate();
        settle(&mut rt);
        let column = find(&rt, "tree-column").unwrap().rect;
        assert!(find_in(&rt, "tree-column", "tree-search").is_some());
        let threads = rt
            .elements()
            .into_iter()
            .find(|e| {
                !e.exiting
                    && e.path.split('/').any(|p| p == "pane-switch")
                    && e.text.as_deref() == Some("Threads")
            })
            .expect("a Files | Threads switch");
        click(&mut rt, &threads);

        assert!(rt.state.threads_pane);
        assert!(
            find_in(&rt, "tree-column", "tree-search").is_none(),
            "no file filter on Threads"
        );
        assert_eq!(find(&rt, "tree-column").unwrap().rect, column, "same width");
        let right = column[0] + column[2];
        for e in rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting && e.path.contains("/thread-list/"))
        {
            assert!(
                e.rect[0] + e.rect[2] <= right + 0.5,
                "{:?} {:?} runs past the column {column:?}",
                e.key,
                e.rect
            );
        }
        let groups: Vec<String> = ["Open", "Outdated", "Resolved", "Comments"]
            .into_iter()
            .filter(|g| find_in(&rt, "thread-list", g).is_some())
            .map(String::from)
            .collect();
        assert_eq!(groups, ["Open", "Outdated", "Resolved", "Comments"]);

        let last = rt
            .state
            .detail
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .entries
            .len()
            - 1;
        let key = format!("item-e{last}");
        let reveal = key.clone();
        rt.update(move |_, tx| tx.scroll_into_view_in("thread-list", reveal));
        settle(&mut rt);
        let row = find_in(&rt, "thread-list", &key).unwrap();
        click(&mut rt, &row);
        assert!(rt.state.conversation_open && rt.state.show_all);
        assert!(
            in_conversation(&rt, &format!("entry-{last}")),
            "the entry scrolls into view"
        );
        let d = rt.state.detail.as_ref().unwrap().as_ref().unwrap();
        let (author, scope) = (d.entries[last].author.clone(), format!("entry-{last}"));
        let shows = rt.elements().into_iter().any(|e| {
            e.path.split('/').any(|p| p == scope) && e.text.as_deref() == Some(author.as_str())
        });
        assert!(shows, "the key of an entry names its own entry");

        press(&mut rt, typed("T", true));
        settle(&mut rt);
        assert!(!rt.state.threads_pane, "Shift+T shows Files");
        assert!(rt.state.tree_search.focused);
    }
}
