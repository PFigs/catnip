//! The keyboard: the keys the key sheet lists, and hint mode, which puts a letter on each
//! control on screen so that a typed letter presses it.

use rune::Tx;

use crate::app::App;
use crate::model::{self, ReviewState, Row};

/// A key and what it does, as the key sheet shows them.
pub type Line = (&'static [&'static str], &'static str);

/// The keys the key sheet lists, in groups.
pub const SHEET: &[(&str, &[Line])] = &[
    (
        "Pull requests",
        &[
            (&["J"], "Next PR"),
            (&["K"], "Previous PR"),
            (&["/"], "Search PRs"),
            (&["1"], "To review"),
            (&["2"], "Acted on"),
            (&["3"], "All PRs"),
            (&["B"], "Show or hide the PR list"),
            (&["R"], "Refresh all"),
            (&["Shift", "R"], "Refresh this PR"),
            (&["O"], "Open on GitHub"),
            (&["Ctrl", "C"], "Copy the PR number"),
            (&["Ctrl", "Shift", "C"], "Copy the PR link"),
        ],
    ),
    (
        "Review",
        &[
            (&["Shift", "A"], "Approve"),
            (&["C"], "Comment on the review"),
            (&["M"], "Comment on the PR"),
            (&["Ctrl", "Enter"], "Send a line comment"),
            (
                &["Ctrl", "Shift", "Enter"],
                "Add a line comment to the review",
            ),
        ],
    ),
    (
        "Diff",
        &[
            (&["N"], "Next change"),
            (&["P"], "Previous change"),
            (&["]"], "Next file"),
            (&["["], "Previous file"),
            (&["U"], "Next open thread"),
            (&["Shift", "U"], "Previous open thread"),
            (&["Z"], "Fold or open every file"),
            (&["T"], "Show or hide the file tree"),
            (&["Shift", "T"], "Search files"),
            (&["Down"], "Move from the file search into the tree"),
            (&["Enter"], "Open the file under the cursor in the tree"),
            (&["F"], "Show or hide the conversation"),
            (&["Ctrl", "F"], "Find in the diff"),
        ],
    ),
    (
        "Keyboard",
        &[
            (&["Space"], "Put a letter on each control"),
            (&["?"], "Show this list"),
            (&["Up"], "Make the cat jump, while the game shows"),
            (&["Down"], "Make the cat roll, while the game shows"),
            (
                &["Right"],
                "Make the cat fire its laser, while the game shows",
            ),
        ],
    ),
];

/// What a hint letter presses.
#[derive(Clone, Debug, PartialEq)]
pub enum Hint {
    Pr(String),
    File(String),
    RefreshAll,
    Settings,
    Ci,
    Review,
    Approve,
    RequestChanges,
    GitHub,
    RefreshPr,
    Threads,
    Find,
    Tree,
    Fold,
}

/// The letters, home row first.
const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// The targets hint mode can label, by the key of their element: each PR row, each file row
/// of the tree, the base PR of the open PR, and the controls that have a key of their own.
pub fn targets(s: &App) -> Vec<(String, Hint)> {
    let mut all: Vec<(String, Hint)> = (s.rows.iter())
        .filter_map(|r| match r {
            Row::Pr(i) => Some(s.prs[*i].id.clone()),
            Row::Header { .. } => None,
        })
        .map(|id| (id.clone(), Hint::Pr(id)))
        .collect();
    let controls = [
        ("refresh-all", Hint::RefreshAll),
        ("open-settings", Hint::Settings),
        ("ci-menu", Hint::Ci),
        ("review-menu", Hint::Review),
        ("approve", Hint::Approve),
        ("request-changes", Hint::RequestChanges),
        ("pr-link", Hint::GitHub),
        ("refresh-pr", Hint::RefreshPr),
        ("threads-button", Hint::Threads),
        ("find-button", Hint::Find),
        ("tree-button", Hint::Tree),
        ("fold-button", Hint::Fold),
    ];
    let files = (0..s.tree.visible_len())
        .filter_map(|row| s.tree.row(row))
        .filter(|(node, _)| !node.dir)
        .map(|(node, _)| (node.path.clone(), Hint::File(node.path.clone())));
    all.extend(files);
    all.extend(controls.map(|(key, hint)| (key.to_owned(), hint)));
    if let Some(base) = s.current().and_then(|pr| model::base_pr(&s.prs, pr)) {
        all.push(("base-pr".into(), Hint::Pr(base.id.clone())));
    }
    all
}

/// The letters of `n` targets: one letter each up to 26 targets, else two letters each, so
/// that no label starts another.
pub fn labels(n: usize) -> Vec<String> {
    let one: Vec<char> = LETTERS.chars().collect();
    if n <= one.len() {
        return one[..n].iter().map(char::to_string).collect();
    }
    let two = one
        .iter()
        .flat_map(|a| one.iter().map(move |b| format!("{a}{b}")));
    two.take(n).collect()
}

/// Does what a click on the target does.
pub fn run(hint: Hint, s: &mut App, tx: &mut Tx) {
    match hint {
        Hint::Pr(id) => {
            s.select(&id);
            tx.scroll_into_view(id);
        }
        Hint::File(path) => s.jump_to_file(&path, tx),
        Hint::RefreshAll => s.refresh(),
        Hint::Settings => s.settings_open = true,
        Hint::Ci => s.ci_menu = !s.ci_menu,
        Hint::Review => s.review_menu = !s.review_menu,
        Hint::Approve => s.submit(ReviewState::Approved),
        Hint::RequestChanges => s.open_composer(ReviewState::ChangesRequested),
        Hint::GitHub => {
            if let Some(url) = s.current().map(|p| p.url.clone()) {
                let _ = tx.open_url(&url);
            }
        }
        Hint::RefreshPr => s.reload(),
        Hint::Threads => {
            s.step_thread(true, tx);
        }
        Hint::Find => s.open_find(),
        Hint::Tree => s.toggle_tree(),
        Hint::Fold => s.fold_all(),
    }
}
