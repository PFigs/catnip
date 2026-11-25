//! The drawer: every PR the filters keep, with what the viewer did on it.

use std::borrow::Cow;

use rune::{El, Spring, SpringPair, Transition, col, el, row, text};
use rune_kit::shell::{RowStatus, Sidebar, SidebarHeader, SidebarRow};
use rune_kit::{
    Button, ButtonVariant, IconButton, IconButtonVariant, Kit, Menu, MenuItem, TextInput, glyphs,
};

use super::{GEAR, REFRESH, SORT, ci_help, ci_icon, mark_icon, verdict_words};
use crate::app::{self, App};
use crate::github::MAX_PAGES;
use crate::model::{self, Row, Sort, age, mark};
use crate::store;

const ROW: f32 = 48.0;

/// A PR row. Under a repository header its second line leaves the repository out.
fn pr_row(s: &App, i: usize, grouped: bool, now: i64, k: &Kit) -> El {
    let pr = &s.prs[i];
    let repo = match grouped {
        true => "",
        false => pr.repo.rsplit('/').next().unwrap_or(&pr.repo),
    };
    let mut subtitle = format!(
        "{repo}#{} · {} · {}",
        pr.number,
        pr.author,
        age(&pr.last_commit, now)
    );
    if pr.draft {
        subtitle.push_str(" · Draft");
    }
    let m = mark(pr, &s.me);
    if let Some(words) = verdict_words(m) {
        subtitle = format!("{subtitle} · {words}");
    }
    let id = pr.id.clone();
    let mut r = SidebarRow::new(pr.title.clone())
        .height(ROW)
        .subtitle(subtitle)
        .select_only()
        .avatar(
            pr.ci
                .map_or_else(el, |ci| ci_icon(ci, k).help(ci_help(Some(ci)))),
        )
        .selected(s.selected.as_ref() == Some(&pr.id))
        .on_select(move |s: &mut App, tx| {
            s.select(&id);
            tx.blur();
        });
    if let Some(e) = mark_icon(m, k) {
        r = r.status(RowStatus::Other(e));
    }
    let drafts = s.draft_count(pr);
    if drafts > 0 {
        r = r.badge(drafts.to_string());
    }
    r.build(k).key(pr.id.clone())
}

/// One stack of placeholder PR cards: each drops in after the one above it. Stack 0 is the
/// empty list, which must mount before the first cards for their drop to play.
fn boot_stack(stack: u32, k: &Kit) -> El {
    const BARS: [(f32, f32); 6] = [
        (180.0, 110.0),
        (140.0, 90.0),
        (200.0, 120.0),
        (160.0, 100.0),
        (190.0, 80.0),
        (130.0, 115.0),
    ];
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(h / 2.0).fill(k.c.border_muted);
    let drop = Transition::offset(0.0, -32.0)
        .combined(Transition::opacity())
        .springs(SpringPair::new(Spring::BOUNCY, Spring::BOUNCY));
    let count = if stack == 0 { 0 } else { BARS.len() };
    let cards = BARS[..count]
        .iter()
        .enumerate()
        .map(|(i, &(title, below))| {
            row()
                .key(format!("boot-card-{stack}-{i}"))
                .w_full()
                .h(ROW)
                .px(12.0)
                .gap(12.0)
                .items_center()
                .child(bar(16.0, 16.0))
                .child(
                    col()
                        .gap(6.0)
                        .child(bar(title, 10.0))
                        .child(bar(below, 8.0)),
                )
                .transition(Transition::asymmetric(drop, Transition::opacity()))
                .enter_delay(i as f32 * 0.12)
        });
    col().key("boot-stack").w_full().children(cards)
}

fn header_row(repo: &str, count: usize, folded: bool, k: &Kit) -> El {
    let name = repo.to_owned();
    SidebarHeader::new(repo)
        .count(count)
        .collapsed(folded)
        .on_toggle(move |s: &mut App, _| s.toggle_group(&name))
        .build(k)
}

fn row_key(s: &App, n: usize) -> Cow<'_, str> {
    match &s.rows[n] {
        Row::Pr(i) => Cow::Borrowed(s.prs[*i].id.as_str()),
        Row::Header { repo, .. } => Cow::Owned(format!("repo-{repo}")),
    }
}

fn search(s: &App, k: &Kit) -> El {
    TextInput::new(&s.pr_search)
        .placeholder("Search PRs  /")
        .label("Search PRs")
        .leading_icon(&glyphs::SEARCH)
        .clear_button()
        .width(None)
        .on_edit(|s: &mut App| &mut s.pr_search)
        .on_change(|s: &mut App, _| {
            let text = s.pr_search.text.clone();
            s.set_pr_search(&text);
        })
        .on_key(app::pr_search_key)
        .on_action(|s: &mut App, _| s.set_pr_search(""))
        .build(k)
        .key("pr-search")
}

/// The sort button right of the PR search, with its menu.
fn sort_menu(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let trigger = IconButton::new(&SORT)
        .variant(IconButtonVariant::GhostMuted)
        .label("Sort")
        .help(format!("Sort: {}", s.settings.sort.label()))
        .expanded(s.sort_menu)
        .on_press(|s: &mut App| s.sort_menu = !s.sort_menu)
        .build(k)
        .key("sort-menu");
    let mut menu = Menu::new("sort").trigger(trigger).open(s.sort_menu);
    for sort in Sort::ALL {
        menu = menu.item(
            MenuItem::new(sort.label())
                .checked(s.settings.sort == sort)
                .on_select(move |s: &mut App| s.set_sort(sort)),
        );
    }
    menu.on_close(|s: &mut App| s.sort_menu = false)
        .build(k, layer)
}

/// The refresh button left of the PR search, and the sort button right of it.
fn header(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let refresh = IconButton::new(&REFRESH)
        .variant(IconButtonVariant::GhostMuted)
        .help("Refresh all")
        .shortcut("r")
        .on_press(|s: &mut App| s.refresh())
        .build(k)
        .key("refresh-all");
    row()
        .w_full()
        .items_center()
        .gap(4.0)
        .child(refresh)
        .child(search(s, k).grow(1.0))
        .child(sort_menu(s, k, layer))
}

fn footer(s: &App, k: &Kit) -> El {
    let line = |t: String| text(t).font(k.ty.small).color(k.c.text_muted).wrap();
    let mut footer = col().gap(6.0).px(4.0).py(4.0);
    let query = s.pr_search.text.trim();
    let found = || model::visible(&s.prs, &s.me, s.quick, &s.picked, query, s.settings.sort).len();
    let status = match (&s.error, s.loading, s.has_scope()) {
        (Some(e), _, _) => e.clone(),
        (None, true, _) => format!("Loading… {}", s.prs.len()),
        (None, false, false) => "Add a scope in Settings".into(),
        (None, false, true) if !query.is_empty() && found() == 0 => {
            format!("No PR in \"{}\" matches \"{query}\"", s.quick.label())
        }
        (None, false, true) => format!("{} of {} open", s.shown.len(), s.prs.len()),
    };
    let settings = IconButton::new(&GEAR)
        .variant(IconButtonVariant::GhostMuted)
        .label("Settings")
        .on_press(|s: &mut App| s.settings_open = true)
        .build(k)
        .key("open-settings");
    footer = footer.child(
        row()
            .w_full()
            .items_center()
            .child(line(status).grow(1.0))
            .child(settings),
    );
    if !s.has_scope() && !s.loading {
        let open = Button::new("Open Settings")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut App| s.settings_open = true)
            .build(k);
        footer = footer.child(open);
    }
    if s.capped {
        footer = footer.child(line(format!(
            "Only the first {} PRs are listed. Narrow the scopes to see the rest.",
            MAX_PAGES * 50
        )));
    }
    footer
}

pub fn view(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let now = store::now();
    let kit = k.clone();
    let bar = Sidebar::new()
        .label("Pull requests")
        .row_height(ROW)
        .header(header(s, k, layer))
        .footer(footer(s, k));
    if s.loading && s.prs.is_empty() {
        return bar.child(boot_stack(s.boot_stack, k)).build(k);
    }
    bar.virtual_rows(s.rows.len(), row_key, move |s: &App, n| {
        let grouped = matches!(s.rows.first(), Some(Row::Header { .. }));
        match &s.rows[n] {
            Row::Pr(i) => pr_row(s, *i, grouped, now, &kit),
            Row::Header {
                repo,
                count,
                folded,
            } => header_row(repo, *count, *folded, &kit),
        }
    })
    .build(k)
}
