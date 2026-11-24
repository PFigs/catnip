//! The open PR: header, conversation, and under the "Files changed" bar the diff. The
//! conversation and the diff scroll apart, so the wheel moves what is under the pointer.

use std::collections::HashMap;

use rune::{Cursor, Cx, Drag, El, Icon, Phase, ScrollSpec, col, el, icon, row, text};
use rune_kit::diff::{Annotation, DiffMode, DiffView};
use rune_kit::shell::{SidebarRow, SidebarSection};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, Collapsible, ControlSize, FileNode, IconButton,
    IconButtonVariant, Kit, Markdown, SegmentedControl, TextArea, TextInput, Tree, TreeItem,
    glyphs,
};

use crate::app::{self, App, Item};
use crate::github::{Detail, Draft, Thread};
use crate::model::{self, Entry, Pr, ReviewState, age};
use crate::store;

static TREE: Icon = Icon {
    name: "pr-tree",
    view: 16.0,
    body: r#"<path d="M2.5 3H8M5 6.5H13.5M5 10H13.5M2.5 3V10H5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M5 13H10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};
static FOLD: Icon = Icon {
    name: "pr-fold",
    view: 16.0,
    body: r#"<path d="M5 2.5L8 5.5L11 2.5M5 13.5L8 10.5L11 13.5M2.5 8H13.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

static THREADS: Icon = Icon {
    name: "pr-threads",
    view: 16.0,
    body: r#"<path d="M2.5 3.5H13.5V10.5H8L5 13V10.5H2.5Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none"/>"#,
};

/// The pane width from which the "Files changed" bar has room for the word "Threads".
const THREADS_WORD_ROOM: f32 = 600.0;
/// The share of the pane the conversation takes at most.
const CONVERSATION_SHARE: f32 = 0.4;
/// The width of the grip on the right edge of the tree.
const GRIP: f32 = 6.0;

fn notice(t: &str, k: &Kit) -> El {
    col()
        .w_full()
        .grow(1.0)
        .center()
        .child(text(t.to_owned()).font(k.ty.ui).color(k.c.text_muted))
}

/// A number in the accent colour that takes a click.
fn link(label: String, k: &Kit) -> Button {
    let font = k.ty.small_medium;
    let ink = k.c.text_accent;
    Button::new(label.clone())
        .variant(ButtonVariant::GhostMuted)
        .control_size(ControlSize::Mini)
        .label_with(move |_, _| text(label).font(font).color(ink))
}

fn github_link(label: String, pr: &Pr, k: &Kit) -> El {
    let url = pr.url.clone();
    link(label, k)
        .help("Open on GitHub")
        .shortcut("o")
        .on_press_tx(move |_: &mut App, tx| {
            let _ = tx.open_url(&url);
        })
        .build(k)
}

/// `#2920 → #2918`: the PR, which a click opens on GitHub, and the open PR of its base branch,
/// which a click opens here.
fn base_link(pr: &Pr, base: &Pr, k: &Kit) -> El {
    let id = base.id.clone();
    let open = link(format!("#{}", base.number), k)
        .help(format!("{} → {}", pr.head_branch, pr.base_branch))
        .on_press_tx(move |s: &mut App, tx| {
            s.select(&id);
            tx.scroll_into_view(id.clone());
        })
        .build(k)
        .key("base-pr");
    row()
        .gap(2.0)
        .items_center()
        .child(github_link(format!("#{}", pr.number), pr, k))
        .child(text("→").font(k.ty.small).color(k.c.text_muted))
        .child(open)
}

fn header(pr: &Pr, base: Option<&Pr>, detail: Option<&Detail>, k: &Kit) -> El {
    let muted = |t: String| text(t).font(k.ty.small).color(k.c.text_muted);
    let mut facts = row()
        .gap(12.0)
        .items_center()
        .child(github_link(format!("{}#{}", pr.repo, pr.number), pr, k).key("pr-link"))
        .child(muted(pr.author.clone()))
        .child(
            text(format!("+{}", pr.additions))
                .font(k.ty.small_medium)
                .color(k.c.success.icon),
        )
        .child(
            text(format!("−{}", pr.deletions))
                .font(k.ty.small_medium)
                .color(k.c.danger.icon),
        );
    facts = match (detail, base) {
        (Some(_), Some(base)) => facts.child(base_link(pr, base, k)),
        (Some(d), None) => facts.child(muted(format!("{} → {}", d.head_branch, d.base_branch))),
        (None, _) => facts,
    };
    let refresh = IconButton::new(&super::REFRESH)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Mini)
        .help("Refresh this PR")
        .shortcut("shift+r")
        .on_press(|s: &mut App| s.reload())
        .build(k)
        .key("refresh-pr");
    facts = facts.child(refresh);
    col()
        .key("pr-header")
        .w_full()
        .px(20.0)
        .pt(14.0)
        .pb(10.0)
        .gap(4.0)
        .child(
            text(pr.title.clone())
                .font(k.ty.body_medium)
                .color(k.c.text_base)
                .wrap(),
        )
        .child(facts)
}

fn verdict_word(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "approved",
        ReviewState::ChangesRequested => "requested changes",
        ReviewState::Commented => "reviewed",
    }
}

fn verdict(state: ReviewState, k: &Kit) -> El {
    let color = match state {
        ReviewState::Approved => k.c.success.icon,
        ReviewState::ChangesRequested => k.c.danger.icon,
        ReviewState::Commented => k.c.text_muted,
    };
    text(verdict_word(state))
        .font(k.ty.small_medium)
        .color(color)
}

/// One line for an entry: its author and its first 40 characters.
fn gist(e: &Entry) -> String {
    let line = e.body.split_whitespace().collect::<Vec<_>>().join(" ");
    let start: String = line.chars().take(40).collect();
    let more = if line.chars().count() > 40 { "…" } else { "" };
    format!("{}: {start}{more}", e.author)
}

/// Edit and Delete for an entry GitHub lets the viewer change, or the question a Delete asks.
fn entry_actions(s: &App, e: &Entry, k: &Kit) -> El {
    let mut actions = row().gap(4.0).items_center();
    let small = |label: &str| {
        Button::new(label)
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Mini)
    };
    if s.deleting.as_deref() == Some(e.id.as_str()) {
        let question = text("Delete?").font(k.ty.small).color(k.c.text_muted);
        let yes = small("Yes")
            .on_press(|s: &mut App| s.confirm_delete())
            .build(k)
            .key("delete-yes");
        let no = small("No")
            .on_press(|s: &mut App| s.cancel_delete())
            .build(k);
        return actions.child(question).child(yes).child(no);
    }
    let editing = s.editing.as_ref().is_some_and(|x| x.entry == e.id);
    if e.can_edit && !editing {
        let id = e.id.clone();
        let edit = small("Edit").on_press(move |s: &mut App| s.start_edit(&id));
        actions = actions.child(edit.build(k).key("edit"));
    }
    if e.can_delete {
        let id = e.id.clone();
        let delete = small("Delete").on_press(move |s: &mut App| s.ask_delete(&id));
        actions = actions.child(delete.build(k).key("delete"));
    }
    actions
}

/// The text of an entry, or its edit box while the viewer edits it.
fn entry_body(s: &App, e: &Entry, k: &Kit) -> Option<El> {
    match s.editing.as_ref().filter(|x| x.entry == e.id) {
        Some(_) => Some(edit_box(s, k)),
        None if e.body.is_empty() => None,
        None => Some(prose(&e.body, k)),
    }
}

/// The edit box: ⌘↵ saves, Escape closes.
fn edit_box(s: &App, k: &Kit) -> El {
    let Some(editing) = s.editing.as_ref() else {
        return col();
    };
    let field = TextArea::new(&editing.edit)
        .auto_grow(2, 16)
        .on_edit(|s: &mut App| match s.editing.as_mut() {
            Some(e) => &mut e.edit,
            None => &mut s.reply_edit,
        })
        .on_key(app::edit_key)
        .on_submit(|s: &mut App, _| s.save_edit())
        .build(k)
        .w_full()
        .key("edit-field");
    let save = match editing.sending {
        true => "Saving…",
        false => "Save",
    };
    let buttons = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.cancel_edit())
                .build(k),
        )
        .child(
            Button::new(save)
                .variant(ButtonVariant::Submit)
                .shortcut("cmd+enter")
                .disabled(editing.sending)
                .on_press(|s: &mut App| s.save_edit())
                .build(k)
                .key("save-edit"),
        );
    col().w_full().gap(8.0).child(field).child(buttons)
}

fn entry(s: &App, key: String, e: &Entry, now: i64, k: &Kit) -> El {
    let mut by = row().gap(8.0).items_center().child(
        text(e.author.clone())
            .font(k.ty.small_medium)
            .color(k.c.text_base),
    );
    if let Some(state) = e.review {
        by = by.child(verdict(state, k));
    }
    if e.bot {
        by = by.child(text("bot").font(k.ty.small).color(k.c.text_faint));
    }
    by = by
        .child(text(age(&e.at, now)).font(k.ty.small).color(k.c.text_faint))
        .child(el().grow(1.0))
        .child(entry_actions(s, e, k));
    let card = col().key(key).w_full().gap(4.0).child(by);
    match entry_body(s, e, k) {
        Some(body) => card.child(body),
        None => card,
    }
}

/// Markdown that a reader selects, and whose links open in the browser.
fn prose(body: &str, k: &Kit) -> El {
    Markdown::new(body.to_owned())
        .selectable(true)
        .on_link_tx(|_: &mut App, url, tx| {
            let _ = tx.open_url(url);
        })
        .build(k)
}

/// The description in a card, under its author and age.
fn description(s: &App, e: &Entry, now: i64, k: &Kit) -> El {
    let by = row()
        .gap(8.0)
        .items_center()
        .child(
            text(e.author.clone())
                .font(k.ty.small_medium)
                .color(k.c.text_base),
        )
        .child(
            text(format!("opened {}", age(&e.at, now)))
                .font(k.ty.small)
                .color(k.c.text_faint),
        )
        .child(el().grow(1.0))
        .child(entry_actions(s, e, k));
    let body = entry_body(s, e, k)
        .unwrap_or_else(|| text("No description.").font(k.ty.ui).color(k.c.text_faint));
    panel(k, col().w_full().gap(8.0).child(by).child(body)).key("description")
}

/// "Comments", their count, and the switch between the newest and the oldest first.
fn comments_header(count: usize, oldest_first: bool, k: &Kit) -> El {
    let order = SegmentedControl::new(&["Newest", "Oldest"], Some(usize::from(oldest_first)))
        .fit()
        .on_pick(|s: &mut App, i, _| s.set_oldest_first(i == 1))
        .build(k)
        .key("comment-order");
    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(text("Comments").font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            text(count.to_string())
                .font(k.ty.small)
                .color(k.c.text_faint),
        )
        .child(el().grow(1.0))
        .child(order)
}

fn conversation(s: &App, d: &Detail, max_h: f32, k: &Kit) -> El {
    let now = store::now();
    let oldest_first = s.settings.oldest_first;
    let mut comments = col().w_full().gap(12.0);
    let shown = model::conversation(&d.entries, s.show_all, oldest_first);
    for (i, &n) in shown.iter().enumerate() {
        if i > 0 {
            comments = comments.child(el().w_full().h(1.0).fill(k.c.border_muted));
        }
        comments = comments.child(entry(s, format!("entry-{n}"), &d.entries[n], now, k));
    }
    let more = d.entries.len() - shown.len();
    if more > 0 {
        let show = Button::new(format!("Show {more} more"))
            .variant(ButtonVariant::GhostMuted)
            .on_press(|s: &mut App| s.show_all = true)
            .build(k);
        comments = comments.child(row().child(show));
    }
    // The comment box sits next to the newest comment.
    let (first, last) = match oldest_first {
        true => (comments, comment_box(s, k)),
        false => (comment_box(s, k), comments),
    };
    let list = col()
        .w_full()
        .px(20.0)
        .pb(12.0)
        .gap(16.0)
        .child(description(s, &d.description, now, k))
        .child(comments_header(d.entries.len(), oldest_first, k))
        .child(first)
        .child(last);
    col()
        .key("conversation")
        .w_full()
        .max_h(max_h)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(16.0)
        .child(list)
}

/// The box at the foot of the conversation that sends a comment on the PR.
fn comment_box(s: &App, k: &Kit) -> El {
    let busy = s.sending_comment();
    // One line until used, so that the comments under it stay in view.
    let open = busy || s.pr_comment.focused || !s.pr_comment.text.is_empty();
    let field = TextArea::new(&s.pr_comment)
        .placeholder("Comment on this PR (⌘↵ to send)")
        .label("Comment on the PR")
        .auto_grow(if open { 3 } else { 1 }, 10)
        .width(None)
        .disabled(busy)
        .on_edit(|s: &mut App| &mut s.pr_comment)
        .on_submit(|s: &mut App, _| s.send_comment())
        .build(k);
    let send = Button::new(if busy { "Sending…" } else { "Comment" })
        .variant(ButtonVariant::Submit)
        .disabled(busy || s.pr_comment.text.trim().is_empty())
        .on_press(|s: &mut App| s.send_comment())
        .build(k)
        .key("pr-comment-send");
    let mut comment = col().key("pr-comment").w_full().gap(8.0).child(field);
    if open {
        comment = comment.child(row().w_full().justify_end().child(send));
    }
    comment
}

fn bar_button(glyph: &'static Icon, help: &str, shortcut: &str) -> IconButton {
    IconButton::new(glyph)
        .variant(IconButtonVariant::GhostMuted)
        .help(help)
        .shortcut(shortcut)
}

/// The find field with its count, previous, next and close, in place of the bar's title.
fn find_bar(s: &App, k: &Kit) -> El {
    let field = TextInput::new(&s.find_edit)
        .placeholder("Find in the diff")
        .label("Find in the diff")
        .leading_icon(&glyphs::SEARCH)
        .width(None)
        .on_edit(|s: &mut App| &mut s.find_edit)
        .on_change(|s: &mut App, _| {
            let text = s.find_edit.text.clone();
            s.set_find(&text);
        })
        .on_key(app::find_key)
        .build(k)
        .key("find-field");
    let count = match (s.find_at, s.find_matches.len()) {
        (_, 0) if s.find_edit.text.trim().is_empty() => String::new(),
        (_, 0) => "No match".to_owned(),
        (Some(at), n) => format!("{} of {n}", at + 1),
        (None, n) => format!("{n} found"),
    };
    let none = s.find_matches.is_empty();
    row()
        .key("find-bar")
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .items_center()
        .child(field.grow(1.0).min_w(0.0))
        .child(text(count).font(k.ty.small).color(k.c.text_muted).px(6.0))
        .child(
            bar_button(&glyphs::CHEVRON_UP, "Previous match", "shift+enter")
                .disabled(none)
                .on_press_tx(|s: &mut App, tx| s.step_find(false, tx))
                .build(k),
        )
        .child(
            bar_button(&glyphs::CHEVRON_DOWN, "Next match", "enter")
                .disabled(none)
                .on_press_tx(|s: &mut App, tx| s.step_find(true, tx))
                .build(k),
        )
        .child(
            bar_button(&glyphs::CLOSE, "Close find", "escape")
                .on_press(|s: &mut App| s.close_find())
                .build(k)
                .key("find-close"),
        )
}

fn files_bar(s: &App, k: &Kit) -> El {
    let files = s.diff.files();
    let (adds, dels) = files
        .iter()
        .fold((0, 0), |(a, d), f| (a + f.additions(), d + f.deletions()));
    let count = match files.len() {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    };
    let chevron = if s.conversation_open {
        &glyphs::CHEVRON_UP
    } else {
        &glyphs::CHEVRON_DOWN
    };
    let mut toggle = row()
        .key("files-toggle")
        .h_full()
        .gap(8.0)
        .items_center()
        .role("button")
        .label("Files changed (f)")
        .on_press(|s: &mut App| s.conversation_open = !s.conversation_open)
        .child(icon(chevron, 14.0).color(k.c.icon_muted));
    if !s.find_open {
        toggle = toggle
            .grow(1.0)
            .child(
                text("Files changed")
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
            )
            .child(text(count).font(k.ty.small).color(k.c.text_muted))
            .child(
                text(format!("+{adds}"))
                    .font(k.ty.small_medium)
                    .color(k.c.success.icon),
            )
            .child(
                text(format!("−{dels}"))
                    .font(k.ty.small_medium)
                    .color(k.c.danger.icon),
            );
    }
    let find = bar_button(&glyphs::SEARCH, "Find in the diff", "cmd+f")
        .expanded(s.find_open)
        .on_press(|s: &mut App| match s.find_open {
            true => s.close_find(),
            false => s.open_find(),
        })
        .build(k)
        .key("find-button");
    let tree_help = match (s.tree_fits(), s.settings.tree_closed) {
        (false, _) if s.sidebar.is_open() => "Close the drawer (b) to show the file tree",
        (false, _) => "The window is too narrow for the file tree",
        (true, true) => "Show the file tree",
        (true, false) => "Hide the file tree",
    };
    let tree = bar_button(&TREE, tree_help, "t")
        .expanded(s.tree_shown())
        .disabled(!s.tree_fits())
        .on_press(|s: &mut App| s.toggle_tree())
        .build(k)
        .key("tree-button");
    let threads = threads_button(s, k);
    let open = (0..files.len()).any(|f| !s.diff.collapsed(f));
    let fold = bar_button(
        &FOLD,
        if open {
            "Fold every file"
        } else {
            "Open every file"
        },
        "z",
    )
    .on_press(|s: &mut App| s.fold_all())
    .build(k)
    .key("fold-button");
    let split = s.diff.mode() == DiffMode::Split;
    let modes = SegmentedControl::new(&["Unified", "Split"], Some(split as usize))
        .fit()
        .on_pick(|s: &mut App, i, _| {
            s.diff.set_mode(if i == 1 {
                DiffMode::Split
            } else {
                DiffMode::Unified
            })
        })
        .build(k);
    let mut bar = row()
        .key("files-bar")
        .w_full()
        .h(40.0)
        .px(20.0)
        .gap(12.0)
        .items_center()
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(toggle);
    if s.find_open {
        bar = bar.child(find_bar(s, k));
    }
    bar.child(threads)
        .child(
            row()
                .gap(2.0)
                .items_center()
                .child(find)
                .child(tree)
                .child(fold),
        )
        .child(modes)
}

/// "Threads" with the count of open threads: a click goes to the next one, as `u` does. A
/// narrow pane leaves out the word.
fn threads_button(s: &App, k: &Kit) -> El {
    let count = s.open_threads().len();
    let badge = Badge::new(count.to_string())
        .variant(BadgeVariant::Subtle)
        .build(k);
    let word = s.pane_width >= THREADS_WORD_ROOM;
    Button::new("Threads")
        .variant(ButtonVariant::GhostMuted)
        .icon(&THREADS)
        .help(if count == 0 {
            "No open thread"
        } else {
            "Next open thread"
        })
        .shortcut("u")
        .disabled(count == 0)
        .on_press_tx(|s: &mut App, tx| {
            s.step_thread(true, tx);
        })
        .label_with(move |font, ink| {
            let label = row().gap(6.0).items_center();
            let label = match word {
                true => label.child(text("Threads").font(font).color(ink)),
                false => label,
            };
            label.child(badge)
        })
        .build(k)
        .key("threads-button")
}

/// The file tree with its search field, or the list of threads: a click on a file or an item
/// shows it.
fn tree_column(s: &App, k: &Kit) -> El {
    let switch = SegmentedControl::new(&["Files", "Threads"], Some(s.threads_pane as usize))
        .label("Show")
        .width(None)
        .on_pick(|s: &mut App, i, _| {
            s.threads_pane = i == 1;
            s.tree_search.focused &= !s.threads_pane;
        })
        .build(k);
    let column = col()
        .key("tree-column")
        .w(s.tree_drawn_width())
        .h_full()
        .min_h(0.0)
        .pt(8.0)
        .gap(6.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], k.c.border_muted)
        .child(col().key("pane-switch").w_full().px(8.0).child(switch));
    let column = match s.threads_pane {
        true => column.child(thread_list(s, k)),
        false => column.child(file_tree(s, k)),
    };
    column.child(tree_grip(s, k))
}

/// The file filter over the file tree.
fn file_tree(s: &App, k: &Kit) -> El {
    let search = TextInput::new(&s.tree_search)
        .placeholder("Filter files  ⇧T")
        .label("Filter files")
        .leading_icon(&glyphs::SEARCH)
        .clear_button()
        .width(None)
        .on_edit(|s: &mut App| &mut s.tree_search)
        .on_change(|s: &mut App, _| {
            let text = s.tree_search.text.clone();
            s.set_tree_search(&text);
        })
        .on_key(app::tree_search_key)
        .on_action(|s: &mut App, _| s.set_tree_search(""))
        .build(k)
        .key("tree-search");
    let query = s.tree_search.text.trim().to_owned();
    let mut open: HashMap<String, usize> = HashMap::new();
    for t in s.detail.iter().flatten().flat_map(|d| &d.threads) {
        if !t.resolved {
            *open.entry(t.path.clone()).or_default() += 1;
        }
    }
    let tree = Tree::new(
        |s: &App| &s.tree,
        |s: &mut App| &mut s.tree,
        FileNode::<(u32, u32)>::label,
    )
    .row(move |node, r| {
        let mut item = TreeItem::file(node, r);
        if let Some((added, removed)) = node.data {
            item = item.changes(added, removed);
        }
        if let Some(n) = open.get(&node.path).filter(|_| !node.dir) {
            item = item.badge(n.to_string());
        }
        if !query.is_empty() {
            item = item.highlight(query.clone());
        }
        item
    })
    // An arrow keeps the keyboard in the tree; a click or Enter hands it to the diff.
    .on_select(|s: &mut App, id, tx| match tx.by_keys() {
        true => s.show_file(id, tx),
        false => s.jump_to_file(id, tx),
    })
    .on_open(|s: &mut App, id, tx| s.jump_to_file(id, tx))
    .name("Files")
    .build(s, k)
    .grow(1.0)
    .min_h(0.0)
    .px(4.0);
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(6.0)
        .child(col().w_full().px(8.0).child(search))
        .child(tree)
}

/// The threads and the conversation entries of the open PR, in groups with their counts.
fn thread_list(s: &App, k: &Kit) -> El {
    let list = col()
        .key("thread-list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(4.0)
        .gap(6.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let groups = s.thread_groups();
    let Some(d) = s.detail.as_ref().and_then(|d| d.as_ref().ok()) else {
        return list;
    };
    if groups.is_empty() {
        let none = text("No threads or comments")
            .font(k.ty.ui)
            .color(k.c.text_faint);
        return list.child(none.px(8.0));
    }
    let now = store::now();
    groups.into_iter().fold(list, |list, (name, items)| {
        let rows = items.iter().map(|&item| thread_row(d, item, now, k));
        list.child(
            SidebarSection::new(name)
                .count(items.len())
                .rows(rows)
                .build(k),
        )
    })
}

/// A row of the list of threads: the first comment, then the place or the kind of entry.
/// Resolved threads are dimmed.
fn thread_row(d: &Detail, item: Item, now: i64, k: &Kit) -> El {
    let (key, first, below, dim) = match item {
        Item::Thread(n) => {
            let t = &d.threads[n];
            let place = match (t.line, t.original_line) {
                (Some(line), _) => format!("{}:{line}", t.path),
                (None, Some(line)) => format!("{} · was line {line}", t.path),
                (None, None) => t.path.clone(),
            };
            (format!("t{n}"), t.comments.first(), place, t.resolved)
        }
        Item::Entry(n) => {
            let e = &d.entries[n];
            let kind = e.review.map_or("comment", verdict_word);
            let below = format!("{kind} · {}", age(&e.at, now));
            (format!("e{n}"), Some(e), below, false)
        }
    };
    let row = SidebarRow::new(first.map_or_else(|| "No comment".to_owned(), gist))
        .subtitle(below)
        .select_only()
        .on_select(move |s: &mut App, tx| {
            tx.blur();
            s.go_to(item, tx);
        })
        .build(k)
        .key(format!("item-{key}"));
    if dim { row.opacity(0.6) } else { row }
}

/// The right edge of the tree: a drag sets its width, a double click puts the default back.
fn tree_grip(s: &App, k: &Kit) -> El {
    let w = &s.tree_width;
    el().key("tree-grip")
        .role("separator")
        .label("Resize the file tree")
        .range_value(
            f64::from(w.width()),
            f64::from(w.min_width()),
            f64::from(w.max_width()),
        )
        .w(GRIP)
        .pin(None, Some(0.0), Some(0.0), Some(0.0))
        .cursor(Cursor::ResizeColumn)
        .hover(k.c.border_strong)
        .focusable(false)
        .on_drag(|s: &mut App, d: &mut Drag, tx| match d.phase {
            Phase::Down => d.data = s.tree_drawn_width(),
            Phase::Move => {
                tx.instant();
                s.drag_tree(d.data, d.dx);
            }
            Phase::Up if d.cancelled() => s.restore_tree_width(),
            Phase::Up => s.save_tree_width(),
        })
        .on_double_press(|s: &mut App| s.reset_tree_width())
}

/// A shaded box with a border.
fn panel(k: &Kit, inner: El) -> El {
    col()
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(8.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(inner)
}

fn card(k: &Kit, inner: El) -> El {
    col().w_full().px(16.0).py(8.0).child(panel(k, inner))
}

fn thread(s: &App, n: usize, k: &Kit) -> El {
    let Some(t) = s
        .detail
        .as_ref()
        .and_then(|d| d.as_ref().ok())
        .and_then(|d| d.threads.get(n))
    else {
        return col();
    };
    let now = store::now();
    let mut body = col().w_full().gap(10.0);
    let spot = s.thread_spot(t);
    if let Some(label) = thread_label(t, spot.is_some_and(|(.., on_line)| on_line)) {
        body = body.child(text(label).font(k.ty.small_medium).color(k.c.warning.icon));
    }
    if t.line.is_none() && !spot.is_some_and(|(.., on_line)| on_line) && !t.hunk.is_empty() {
        body = body.child(code_made_on(&t.hunk, k));
    }
    for (i, c) in t.comments.iter().enumerate() {
        body = body.child(entry(s, format!("c{i}"), c, now, k));
    }
    let replying = s.reply_to.as_ref() == Some(&t.id);
    if replying {
        body = body.child(reply_box(s, k));
    }
    let mut actions = row().w_full().gap(8.0).justify_end();
    if t.can_reply && !replying {
        let id = t.id.clone();
        let reply = Button::new("Reply")
            .variant(ButtonVariant::GhostMuted)
            .on_press(move |s: &mut App| s.start_reply(&id))
            .build(k)
            .key("reply");
        actions = actions.child(reply);
    }
    let allowed = if t.resolved {
        t.can_unresolve
    } else {
        t.can_resolve
    };
    if allowed {
        let busy = s.resolving.contains(&t.id);
        let label = match (t.resolved, busy) {
            (false, false) => "Resolve",
            (false, true) => "Resolving…",
            (true, false) => "Unresolve",
            (true, true) => "Unresolving…",
        };
        let (id, resolve) = (t.id.clone(), !t.resolved);
        let button = Button::new(label)
            .variant(ButtonVariant::GhostMuted)
            .disabled(busy)
            .on_press(move |s: &mut App| s.resolve(&id, resolve))
            .build(k)
            .key("resolve");
        actions = actions.child(button);
    }
    body = body.child(actions);
    if !t.resolved {
        return card(k, body);
    }
    // Folded, the line names the first comment; opened, the comment shows below it.
    let open = s.opened_threads.contains(&t.id);
    let title = match t.comments.first().filter(|_| !open) {
        Some(c) => format!("Resolved · {}", gist(c)),
        None => "Resolved".to_owned(),
    };
    let id = t.id.clone();
    let fold = Collapsible::new(title, open)
        .ghost()
        .child(body)
        .on_toggle(move |s: &mut App| s.toggle_thread(&id))
        .build(k)
        .key("resolved");
    match open {
        true => card(k, fold),
        false => col().w_full().px(16.0).py(4.0).opacity(0.6).child(fold),
    }
}

/// What a thread off its own line says above its comments: outdated, moved, or on a whole
/// file. None for a thread on its line.
fn thread_label(t: &Thread, on_line: bool) -> Option<String> {
    match (t.line, t.outdated, t.original_line) {
        (Some(_), ..) => None,
        (None, true, Some(line)) if on_line => Some(format!("Outdated · was line {line}")),
        (None, true, Some(line)) => Some(format!("Outdated · line {line}, which changed since")),
        (None, _, None) => Some("On the whole file".into()),
        (None, ..) => Some("Outdated".into()),
    }
}

/// The last lines of the diff hunk a thread was made on, which end at its line.
fn code_made_on(hunk: &str, k: &Kit) -> El {
    let lines: Vec<&str> = hunk.lines().filter(|l| !l.starts_with("@@")).collect();
    let tail = lines[lines.len().saturating_sub(4)..].join("\n");
    Markdown::new(format!("```diff\n{tail}\n```"))
        .selectable(true)
        .build(k)
        .key("code-made-on")
}

/// The reply box of a thread: ⌘↵ sends, Escape closes.
fn reply_box(s: &App, k: &Kit) -> El {
    let field = TextArea::new(&s.reply_edit)
        .placeholder("Reply (⌘↵ to send)")
        .auto_grow(2, 10)
        .on_edit(|s: &mut App| &mut s.reply_edit)
        .on_key(app::reply_key)
        .on_submit(|s: &mut App, _| s.send_reply())
        .build(k)
        .w_full()
        .key("reply-field");
    let send = match s.reply_sending {
        true => "Replying…",
        false => "Reply",
    };
    let buttons = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.cancel_reply())
                .build(k),
        )
        .child(
            Button::new(send)
                .variant(ButtonVariant::Submit)
                .shortcut("cmd+enter")
                .disabled(s.reply_sending)
                .on_press(|s: &mut App| s.send_reply())
                .build(k)
                .key("send-reply"),
        );
    col().w_full().gap(8.0).child(field).child(buttons)
}

/// The threads on files the diff lacks, folded under one line above the diff.
fn unplaced_threads(s: &App, d: &Detail, k: &Kit) -> Option<El> {
    let unplaced: Vec<usize> = (0..d.threads.len())
        .filter(|&n| !d.threads[n].resolved && s.thread_spot(&d.threads[n]).is_none())
        .collect();
    if unplaced.is_empty() {
        return None;
    }
    let mut list = col().w_full().max_h(320.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    for &n in &unplaced {
        let path = text(d.threads[n].path.clone())
            .font(k.ty.small)
            .color(k.c.text_muted);
        list = list.child(col().w_full().px(16.0).pt(8.0).child(path));
        list = list.child(thread(s, n, k).key(format!("unplaced-{n}")));
    }
    let title = match unplaced.len() {
        1 => "1 open thread on a file this diff no longer has".to_owned(),
        n => format!("{n} open threads on files this diff no longer has"),
    };
    let fold = Collapsible::new(title, s.unplaced_open)
        .ghost()
        .child(list)
        .on_toggle(|s: &mut App| s.unplaced_open = !s.unplaced_open)
        .build(k);
    Some(col().key("unplaced").w_full().px(16.0).child(fold))
}

fn draft(s: &App, n: usize, k: &Kit) -> El {
    let Some(d) = s.current_drafts().get(n) else {
        return col();
    };
    let posting = s.is_posting(d);
    let state = match posting {
        true => "Sending…",
        false => "Unsent, goes with your review",
    };
    let remove = Button::new("Remove")
        .variant(ButtonVariant::GhostMuted)
        .disabled(posting)
        .on_press(move |s: &mut App| s.remove_draft(n))
        .build(k);
    let top = row()
        .w_full()
        .items_center()
        .justify_between()
        .child(text(state).font(k.ty.small_medium).color(k.c.warning.icon))
        .child(remove);
    let body = col()
        .w_full()
        .gap(4.0)
        .child(top)
        .child(Markdown::new(d.body.clone()).selectable(true).build(k));
    card(k, body)
}

/// The unsent line comments the diff does not show, with Remove: those written on an older
/// commit, and those on a file the diff no longer has.
fn drafts_off_diff(s: &App, k: &Kit) -> Option<El> {
    let off = s.drafts_off_diff();
    if off.is_empty() {
        return None;
    }
    let head = s
        .detail
        .as_ref()
        .and_then(|d| d.as_ref().ok())
        .map_or("", |d| d.head_commit.as_str());
    let mut list = col()
        .key("drafts-off-diff")
        .w_full()
        .px(20.0)
        .py(8.0)
        .gap(6.0);
    for (title, older) in [
        ("Written on an older commit", true),
        (
            "On a file this diff no longer has, sent with your review",
            false,
        ),
    ] {
        let group: Vec<_> = off.iter().filter(|o| (o.0 != head) == older).collect();
        if group.is_empty() {
            continue;
        }
        list = list.child(text(title).font(k.ty.small_medium).color(k.c.warning.icon));
        for &&(draft_head, n, d) in &group {
            list = list.child(off_diff_row(draft_head, n, d, k));
        }
    }
    Some(list)
}

fn off_diff_row(head: &str, n: usize, d: &Draft, k: &Kit) -> El {
    let lines = match d.start == d.end {
        true => format!("{}:{}", d.path, d.end),
        false => format!("{}:{}-{}", d.path, d.start, d.end),
    };
    let key = format!("off-{head}-{n}");
    let head = head.to_owned();
    let remove = Button::new("Remove")
        .variant(ButtonVariant::GhostMuted)
        .on_press(move |s: &mut App| s.remove_draft_of(&head, n))
        .build(k);
    row()
        .key(key)
        .w_full()
        .gap(8.0)
        .items_center()
        .child(text(lines).font(k.ty.small).color(k.c.text_muted))
        .child(
            text(d.body.clone())
                .font(k.ty.small)
                .color(k.c.text_base)
                .wrap()
                .grow(1.0),
        )
        .child(remove)
}

/// A line that says GitHub holds older comments than the pane shows, and opens the PR there.
fn truncated(url: String, k: &Kit) -> El {
    let open = Button::new("Older comments and threads are on GitHub")
        .variant(ButtonVariant::GhostMuted)
        .on_press_tx(move |_: &mut App, tx| {
            let _ = tx.open_url(&url);
        })
        .build(k);
    row().key("truncated").w_full().px(16.0).child(open)
}

fn composer(s: &App, k: &Kit) -> El {
    let field = TextArea::new(&s.comment_edit)
        .placeholder("Comment on this line (⌘↵ to send, ⇧⌘↵ to add to your review)")
        .rows(3)
        .on_edit(|s: &mut App| &mut s.comment_edit)
        .on_key(app::comment_key)
        .on_submit(|s: &mut App, _| s.send_line_comment())
        .build(k)
        .w_full();
    let buttons = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.cancel_comment())
                .build(k),
        )
        .child(
            Button::new("Add to review")
                .variant(ButtonVariant::Outline)
                .shortcut("cmd+shift+enter")
                .on_press(|s: &mut App| s.add_comment())
                .build(k),
        )
        .child(
            Button::new("Comment")
                .variant(ButtonVariant::Submit)
                .shortcut("cmd+enter")
                .on_press(|s: &mut App| s.send_line_comment())
                .build(k),
        );
    card(k, col().w_full().gap(8.0).child(field).child(buttons))
}

/// What sits under a line: `t<n>` a thread, `d<n>` an unsent comment, `compose` the box.
fn note(s: &App, a: &Annotation, k: &Kit) -> El {
    let index = |prefix: char| a.id.strip_prefix(prefix)?.parse::<usize>().ok();
    let e = match (index('t'), index('d')) {
        (Some(n), _) => thread(s, n, k),
        (_, Some(n)) => draft(s, n, k),
        _ => composer(s, k),
    };
    e.key(format!("note-{}", a.id))
}

/// The count of review threads and unsent comments of a file, at the end of its header, so a
/// folded file still shows them.
fn file_notes(s: &App, file: usize, k: &Kit) -> El {
    let Some(path) = s.diff.files().get(file).map(|f| f.path()) else {
        return el();
    };
    let threads = s.detail.as_ref().and_then(|d| d.as_ref().ok());
    let threads = threads.map_or(0, |d| {
        let open = s.open_threads();
        open.iter().filter(|&&n| d.threads[n].path == path).count()
    });
    let unsent = s.current_drafts().iter().filter(|d| d.path == path).count();
    let mut notes = row().gap(4.0).items_center();
    if threads > 0 {
        let label = match threads {
            1 => "1 open thread".to_owned(),
            n => format!("{n} open threads"),
        };
        notes = notes.child(Badge::new(label).build(k));
    }
    if unsent > 0 {
        let unsent = Badge::new(format!("{unsent} unsent")).variant(BadgeVariant::Accent);
        notes = notes.child(unsent.build(k));
    }
    notes
}

fn diff(s: &App, k: &Kit) -> El {
    if let Some(e) = &s.diff_error {
        return notice(&format!("The diff did not load: {e}"), k);
    }
    if !s.diff_ready {
        return notice("Loading the diff…", k);
    }
    let kit = k.clone();
    let header_kit = k.clone();
    DiffView::new(|s: &App| &s.diff, |s: &mut App| &mut s.diff)
        .annotation(move |s: &App, a: &Annotation| note(s, a, &kit))
        .header_actions(move |s: &App, file| file_notes(s, file, &header_kit))
        .on_comment(|s: &mut App, lines, _| s.start_comment(lines))
        .on_visible_file(|s: &mut App, file| s.follow_file(file))
        .build(s, k)
}

pub fn view(s: &App, cx: &mut Cx<App>, k: &Kit) -> El {
    let pane = col()
        .key("pr-pane")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(k.c.bg_base)
        .on_layout_tx(|s: &mut App, r: [f32; 4], tx| {
            let fits = s.tree_fits();
            s.set_pane_width(r[2]);
            s.pane_height = r[3];
            if fits == s.tree_fits() {
                tx.live_only();
            }
        });
    if s.head_shows() {
        return pane.child(super::cat::head(k));
    }
    let Some(pr) = s.current() else {
        return pane
            .child(notice("Pick a pull request", k))
            .child(super::cat::view(&s.cat, k));
    };
    let base = model::base_pr(&s.prs, pr);
    let detail = match &s.detail {
        None => {
            return pane
                .child(header(pr, base, None, k))
                .child(notice("Loading…", k));
        }
        Some(Err(e)) => {
            let message = format!("The PR did not load: {e}");
            return pane
                .child(header(pr, base, None, k))
                .child(notice(&message, k));
        }
        Some(Ok(d)) => d,
    };
    let height = cx.layout_with("pr-pane", |r| r[3]).unwrap_or(0.0);
    let mut pane = pane.child(header(pr, base, Some(detail), k));
    if detail.truncated {
        pane = pane.child(truncated(pr.url.clone(), k));
    }
    if s.conversation_open {
        pane = pane.child(conversation(s, detail, height * CONVERSATION_SHARE, k));
    }
    if let Some(off) = drafts_off_diff(s, k) {
        pane = pane.child(off);
    }
    if let Some(unplaced) = unplaced_threads(s, detail, k) {
        pane = pane.child(unplaced);
    }
    let mut body = row().w_full().grow(1.0).min_h(0.0);
    if s.tree_shown() && s.diff_ready {
        body = body.child(tree_column(s, k));
    }
    pane.child(files_bar(s, k)).child(
        body.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .min_h(0.0)
                .child(diff(s, k)),
        ),
    )
}
