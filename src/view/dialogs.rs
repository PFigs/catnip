//! The review composer and the settings.

use std::collections::BTreeSet;

use rune::{El, col, row, text};
use rune_kit::{
    Button, ButtonVariant, Dialog, IconButton, IconButtonVariant, Keybind, Kit, TextArea,
    TextInput, glyphs,
};

use crate::app::App;
use crate::keys;
use crate::model::ReviewState;

fn composer(s: &App, state: ReviewState, k: &Kit, layer: &mut Vec<El>) {
    let title = match state {
        ReviewState::ChangesRequested => "Request changes",
        _ => "Comment",
    };
    let field = TextArea::new(&s.review_edit)
        .placeholder("Leave a comment (⌘↵ to send)")
        .rows(6)
        .on_edit(|s: &mut App| &mut s.review_edit)
        .on_submit(move |s: &mut App, _| s.submit(state))
        .build(k)
        .w_full();
    let submit = match s.current_drafts().len() {
        0 => "Submit".to_owned(),
        1 => "Submit with 1 line comment".to_owned(),
        n => format!("Submit with {n} line comments"),
    };
    let buttons = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.review = None)
                .build(k),
        )
        .child(
            Button::new(submit)
                .variant(ButtonVariant::Submit)
                .disabled(!s.can_review())
                .on_press(move |s: &mut App| s.submit(state))
                .build(k),
        );
    Dialog::new("composer", title)
        .open(true)
        .width(560.0)
        .fit()
        .child(
            col()
                .gap(12.0)
                .px(16.0)
                .pb(16.0)
                .child(field)
                .child(buttons),
        )
        .on_close(|s: &mut App| s.review = None)
        .build(k, layer);
}

fn name_group(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let field = TextInput::new(&s.group_edit)
        .placeholder("bots")
        .on_edit(|s: &mut App| &mut s.group_edit)
        .on_submit(|s: &mut App, _| s.save_group())
        .build(k)
        .w_full();
    let authors = s.picked.iter().cloned().collect::<Vec<_>>().join(", ");
    let buttons = row()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut App| s.close_group_dialog())
                .build(k),
        )
        .child(
            Button::new("Save")
                .variant(ButtonVariant::Submit)
                .disabled(s.group_edit.text.trim().is_empty())
                .on_press(|s: &mut App| s.save_group())
                .build(k),
        );
    Dialog::new("name-group", "Save author group")
        .open(true)
        .width(420.0)
        .fit()
        .child(
            col()
                .gap(12.0)
                .px(16.0)
                .pb(16.0)
                .child(text(authors).font(k.ty.small).color(k.c.text_muted).wrap())
                .child(field)
                .child(buttons),
        )
        .on_close(|s: &mut App| s.close_group_dialog())
        .build(k, layer);
}

fn group_row(name: &str, authors: &BTreeSet<String>, k: &Kit) -> El {
    let gone = name.to_owned();
    let remove = IconButton::new(&glyphs::CLOSE)
        .variant(IconButtonVariant::GhostMuted)
        .label(format!("Remove {name}"))
        .on_press(move |s: &mut App| {
            s.settings.author_groups.remove(&gone);
        })
        .build(k);
    let members = authors.iter().cloned().collect::<Vec<_>>().join(", ");
    row()
        .key(format!("group-row-{name}"))
        .w_full()
        .gap(8.0)
        .items_center()
        .child(text(name.to_owned()).font(k.ty.ui).color(k.c.text_base))
        .child(
            text(members)
                .font(k.ty.small)
                .color(k.c.text_muted)
                .grow(1.0),
        )
        .child(remove)
}

fn scope_row(scope: &str, k: &Kit) -> El {
    let gone = scope.to_owned();
    let remove = IconButton::new(&glyphs::CLOSE)
        .variant(IconButtonVariant::GhostMuted)
        .label(format!("Remove {scope}"))
        .on_press(move |s: &mut App| s.settings.scopes.retain(|x| *x != gone))
        .build(k);
    row()
        .key(format!("scope-{scope}"))
        .w_full()
        .items_center()
        .justify_between()
        .child(text(scope.to_owned()).font(k.ty.ui).color(k.c.text_base))
        .child(remove)
}

fn settings(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let label = |t: &str| text(t.to_owned()).font(k.ty.ui_medium).color(k.c.text_base);
    let note = |t: String| text(t).font(k.ty.small).color(k.c.text_muted).wrap();
    let add = TextInput::new(&s.scope_edit)
        .placeholder("org:acme, user:ann or repo:acme/api")
        .invalid(s.scope_error.is_some())
        .on_edit(|s: &mut App| &mut s.scope_edit)
        .on_submit(|s: &mut App, _| s.add_scope())
        .build(k)
        .w_full();
    let mut body = col()
        .gap(10.0)
        .px(16.0)
        .pb(16.0)
        .child(label("PRs come from"));
    body = s
        .settings
        .scopes
        .iter()
        .fold(body, |b, scope| b.child(scope_row(scope, k)));
    body = body.child(add);
    if let Some(e) = &s.scope_error {
        body = body.child(text(e.clone()).font(k.ty.small).color(k.c.danger.fg).wrap());
    }
    let suggestions = s
        .suggestions
        .iter()
        .filter(|x| !s.settings.scopes.contains(x))
        .take(24)
        .fold(row().flex_wrap().gap(6.0), |r, scope| {
            let x = scope.clone();
            r.child(
                Button::new(format!("+ {scope}"))
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(move |s: &mut App| s.push_scope(x.clone()))
                    .build(k),
            )
        });
    body = body
        .child(note("Suggested from your activity".into()))
        .child(suggestions);
    if !s.settings.author_groups.is_empty() {
        body = s
            .settings
            .author_groups
            .iter()
            .fold(body.child(label("Author groups")), |b, (name, authors)| {
                b.child(group_row(name, authors, k))
            });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let saved = s.net.token.is_some();
        let placeholder = if saved {
            "Token saved; paste to replace"
        } else {
            "ghp_…"
        };
        let token = TextInput::new(&s.token_edit)
            .placeholder(placeholder)
            .secure()
            .on_edit(|s: &mut App| &mut s.token_edit)
            .build(k)
            .grow(1.0);
        let forget = Button::new("Forget token")
            .variant(ButtonVariant::Ghost)
            .disabled(!saved)
            .on_press(|s: &mut App| s.forget_token())
            .build(k);
        body = body
            .child(label("GitHub token, kept in this browser"))
            .child(note(
                "A classic token with `repo` and `read:org` covers every scope.".into(),
            ))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(token)
                    .child(forget),
            );
    }
    let save = Button::new("Save")
        .variant(ButtonVariant::Submit)
        .on_press(|s: &mut App| s.save_settings())
        .build(k);
    body = body.child(row().justify_end().child(save));
    Dialog::new("settings", "Settings")
        .open(true)
        .width(600.0)
        .fit()
        .child(body)
        .on_close(|s: &mut App| s.close_settings())
        .build(k, layer);
}

pub fn build(s: &App, k: &Kit, layer: &mut Vec<El>) {
    if s.settings_open {
        settings(s, k, layer);
    } else if let Some(state) = s.review {
        composer(s, state, k, layer);
    } else if s.naming_group {
        name_group(s, k, layer);
    } else if s.keys_open {
        key_sheet(k, layer);
    }
}

/// Every key, in groups, in two columns.
fn key_sheet(k: &Kit, layer: &mut Vec<El>) {
    let group = |(title, lines): &(&str, &[keys::Line])| {
        let rows = lines.iter().map(|(caps, words)| {
            row()
                .w_full()
                .gap(12.0)
                .items_center()
                .child(text(*words).font(k.ty.ui).color(k.c.text_base).grow(1.0))
                .child(Keybind::new(caps).build(k))
        });
        col()
            .w_full()
            .gap(6.0)
            .child(text(*title).font(k.ty.ui_medium).color(k.c.text_muted))
            .children(rows)
    };
    let (left, right) = keys::SHEET.split_at(2);
    let column = |groups: &[(&str, &[keys::Line])]| {
        col().grow(1.0).gap(16.0).children(groups.iter().map(group))
    };
    Dialog::new("key-sheet", "Keys")
        .open(true)
        .width(720.0)
        .fit()
        .child(
            row()
                .gap(32.0)
                .px(16.0)
                .pb(16.0)
                .child(column(left))
                .child(column(right)),
        )
        .on_close(|s: &mut App| s.keys_open = false)
        .build(k, layer);
}
