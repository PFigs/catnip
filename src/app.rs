//! The app's state, the answers that change it, and the actions the view and keys run.

use std::collections::{BTreeSet, HashMap};

use rune::{Key, KeyEvent, TextEdit, Tx, Waker};
use rune_kit::diff::{Annotation, DiffMode, DiffState, FileDiff, LineRange, LineRef, Side, parse};
use rune_kit::shell::{self, PaneState};
use rune_kit::{FileNode, FileTree, TreeModel};

use crate::github::{self, Detail, Draft, Next, Rerun, Run, Thread, ThreadState, Viewer};
use crate::keys;
use crate::model::{self, Entry, EntryKind, MyReview, Pr, Quick, ReviewState, Row, Sort};
use crate::net::Net;
use crate::store::{Settings, Store};

/// Loaded PRs the cache keeps at most.
const CACHE_SIZE: usize = 20;
/// The PRs below the open one that load before the reviewer opens them.
const AHEAD: usize = 3;
/// How long a toast shows before it closes on its own.
const TOAST_SECS: f64 = 4.0;
/// How long one stack of placeholder cards plays in the drawer while the list loads.
const BOOT_STACK_SECS: f64 = 1.8;
/// The cat peeks out after this many seconds, plus up to this many more.
const PEEK_WAIT: (f64, u32) = (600.0, 600);
const PEEK_SECS: f64 = 6.0;
/// The points the diff keeps beside the file tree.
pub const DIFF_ROOM: f32 = 480.0;

pub enum Msg {
    /// `asked` numbers the viewer request; only the latest one's answer counts.
    Viewer {
        asked: u64,
        answer: Result<Viewer, String>,
    },
    Prs {
        generation: u64,
        page: usize,
        answer: Result<(Vec<Pr>, Next), String>,
    },
    /// `load` names the opening of a PR the request was sent for.
    Detail {
        load: u64,
        answer: Result<Detail, String>,
    },
    Diff {
        load: u64,
        answer: Result<String, String>,
    },
    /// The detail of PR `id`, loaded ahead of its opening for the list `generation`.
    Ahead {
        generation: u64,
        id: String,
        answer: Result<Detail, String>,
    },
    /// The diff of PR `id` at `detail`, loaded ahead of its opening.
    AheadDiff {
        generation: u64,
        id: String,
        detail: Detail,
        answer: Result<String, String>,
    },
    /// `head` and `sent` name the line comments the review carried.
    Submitted {
        id: String,
        head: String,
        sent: Vec<Draft>,
        answer: Result<MyReview, String>,
    },
    /// The GitHub Actions runs of the commit the CI action on the way started on.
    Runs(Result<Vec<Run>, String>),
    /// The answer to one re-run of that action.
    Reran(Result<(), String>),
    BranchUpdated(Result<(), String>),
    /// The answer to a resolve of `thread` on PR `id`, or to an unresolve.
    Resolved {
        id: String,
        thread: String,
        resolve: bool,
        answer: Result<ThreadState, String>,
    },
    /// The answer to a line comment of PR `id` at `head`, sent at once as a review.
    LineSent {
        id: String,
        head: String,
        draft: Draft,
        answer: Result<MyReview, String>,
    },
    /// The answer to a comment on the conversation of PR `id`.
    Commented {
        id: String,
        answer: Result<Entry, String>,
    },
    /// The answer to a reply to `thread` of PR `id`.
    Replied {
        id: String,
        thread: String,
        answer: Result<Entry, String>,
    },
    /// The entry `entry` of the PR `id` now reads `body`.
    Edited {
        id: String,
        entry: String,
        body: String,
        answer: Result<(), String>,
    },
    Deleted {
        id: String,
        entry: String,
        answer: Result<(), String>,
    },
}

/// The text box that edits an entry of the open PR.
pub struct Editing {
    pub entry: String,
    pub kind: EntryKind,
    pub edit: TextEdit,
    pub sending: bool,
}

/// A row of the list of threads, by its place in the detail: a thread or an entry of the
/// conversation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Thread(usize),
    Entry(usize),
}

/// A PR loaded once: its detail and its diff at the detail's head commit.
struct Loaded {
    id: String,
    head: String,
    detail: Detail,
    patch: String,
}

/// A CI action on the way, on the PR and the head commit the diff showed at the click.
struct CiAction {
    id: String,
    repo: String,
    head: String,
    /// The re-run asked; None for a branch update.
    rerun: Option<Rerun>,
    /// Re-runs sent, the answers still due, and the errors so far.
    sent: usize,
    due: usize,
    errors: Vec<String>,
}

/// The result of a review, shown in a toast.
pub struct Notice {
    pub text: String,
    pub failed: bool,
}

pub struct App {
    pub net: Net<Msg>,
    store: Store,
    pub settings: Settings,
    pub me: String,
    pub suggestions: Vec<String>,
    pub prs: Vec<Pr>,
    /// The pages of a refresh until its last page arrives.
    incoming: Vec<Pr>,
    /// Indexes into `prs` the drawer lists, in order.
    pub shown: Vec<usize>,
    /// The rows of the drawer: the `shown` PRs under their repository headers.
    pub rows: Vec<Row>,
    pub authors: Vec<String>,
    pub quick: Quick,
    pub picked: BTreeSet<String>,
    /// The drawer's search field; its text is not saved.
    pub pr_search: TextEdit,
    generation: u64,
    viewer_asked: u64,
    pub loading: bool,
    /// More PRs exist than the list fetches.
    pub capped: bool,
    pub error: Option<String>,
    pub toast: Option<Notice>,
    /// When the shown toast closes, on the clock `poll` gets.
    toast_due: Option<f64>,
    /// The count of placeholder stacks the drawer has played while the list loads empty.
    pub boot_stack: u32,
    /// When the next placeholder stack starts, on the clock `poll` gets.
    boot_due: Option<f64>,
    /// The pixel cat game the empty PR pane shows.
    pub cat: crate::cat::Game,
    /// The game shows over the open PR.
    pub game_open: bool,
    /// Where along the bottom of the PR pane the cat peeks out, from 0 to 1, while it does.
    pub peek: Option<f32>,
    /// When the cat next peeks out, or hides while it peeks, on the clock `poll` gets.
    pub(crate) peek_due: Option<f64>,
    peek_seed: u32,
    pub pane_height: f32,
    pub selected: Option<String>,
    load: u64,
    cache: Vec<Loaded>,
    /// The PRs whose load ahead is on the way.
    ahead: BTreeSet<String>,
    pub detail: Option<Result<Detail, String>>,
    pub diff: DiffState,
    /// The patch text of the shown diff.
    patch: String,
    pub diff_ready: bool,
    pub diff_error: Option<String>,
    /// The files of the shown diff by path, each with its added and removed line counts.
    pub tree: TreeModel<FileNode<(u32, u32)>>,
    pub tree_search: TextEdit,
    /// The width of the file tree, its range and its default.
    pub tree_width: PaneState,
    /// The width of the PR pane, from its last layout.
    pub pane_width: f32,
    pub find_open: bool,
    pub find_edit: TextEdit,
    /// The lines of the shown diff that hold the find text, in diff order.
    pub find_matches: Vec<(usize, LineRef)>,
    /// The match find last showed.
    pub find_at: Option<usize>,
    pub conversation_open: bool,
    pub show_all: bool,
    /// Unsent line comments by PR id and the head commit they were written against.
    pub drafts: HashMap<(String, String), Vec<Draft>>,
    pub composing: Option<LineRange>,
    pub comment_edit: TextEdit,
    pub review_menu: bool,
    pub ci_menu: bool,
    pub sort_menu: bool,
    pub more_authors: bool,
    /// The dialog that names a new author group is open.
    pub naming_group: bool,
    pub group_edit: TextEdit,
    /// The review composer, open for a comment or a change request.
    pub review: Option<ReviewState>,
    pub review_edit: TextEdit,
    pub sending: bool,
    /// The line comments on the way, by PR and head: they show as unsent until the answer.
    posting: Vec<(String, String, Draft)>,
    ci_action: Option<CiAction>,
    pub settings_open: bool,
    /// The key sheet is open.
    pub keys_open: bool,
    /// Hint mode is on: each target on screen shows a letter.
    pub hinting: bool,
    /// The element keys of the targets on screen, in the order of their letters. The view
    /// sets them when it lays out the letters.
    pub hint_keys: Vec<String>,
    /// The letters typed so far in hint mode.
    pub hint_typed: String,
    pub scope_edit: TextEdit,
    pub scope_error: Option<String>,
    pub token_edit: TextEdit,
    pub sidebar: PaneState,
    /// The id of the thread `u` or `Shift+U` last went to.
    pub thread_at: Option<String>,
    /// The threads with a resolve or an unresolve on the way.
    pub resolving: BTreeSet<String>,
    /// The resolved threads the reviewer opened.
    pub opened_threads: BTreeSet<String>,
    /// The comment box at the foot of the conversation.
    pub pr_comment: TextEdit,
    /// The PRs with a comment on the way.
    pub commenting: BTreeSet<String>,
    /// The thread whose reply box is open.
    pub reply_to: Option<String>,
    pub reply_edit: TextEdit,
    /// A reply is on the way.
    pub reply_sending: bool,
    pub editing: Option<Editing>,
    /// The entry whose Delete asks "Delete?", or is on the way.
    pub deleting: Option<String>,
    delete_sending: bool,
    /// The panel of threads on files the diff lacks is open.
    pub unplaced_open: bool,
    /// The file-tree column shows the list of threads in place of the files.
    pub threads_pane: bool,
    /// A detail request of the open PR is on the way.
    detail_due: bool,
    /// The drawer row to scroll into view at the next frame.
    reveal_row: Option<String>,
}

/// Files a reviewer rarely reads, which "open every file" leaves folded.
fn is_noise(path: &str) -> bool {
    const NAMES: [&str; 6] = [
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "Cargo.lock",
        "uv.lock",
        "poetry.lock",
    ];
    let name = path.rsplit('/').next().unwrap_or(path);
    NAMES.contains(&name)
        || path.contains("generated")
        || name.ends_with(".snap")
        || name.ends_with(".min.js")
}

/// The lines of `files` that hold `query`, in diff order: a removed line on its old side, any
/// other on its new side.
fn find_lines(files: &[FileDiff], query: &str) -> Vec<(usize, LineRef)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (f, file) in files.iter().enumerate() {
        for line in file.hunks.iter().flat_map(|h| &h.lines) {
            let at = match (line.new, line.old) {
                (Some(n), _) => LineRef::new(Side::New, n),
                (None, Some(n)) => LineRef::new(Side::Old, n),
                (None, None) => continue,
            };
            if model::matches(&line.text, query) {
                found.push((f, at));
            }
        }
    }
    found
}

/// A diff state in Split mode, the mode of the first diff.
fn split(mut diff: DiffState) -> DiffState {
    diff.set_mode(DiffMode::Split);
    diff
}

/// The scopes of a first run: the viewer's organizations, else their first ten repositories.
fn first_scopes(suggestions: &[String]) -> Vec<String> {
    let orgs: Vec<String> = suggestions
        .iter()
        .filter(|s| s.starts_with("org:"))
        .cloned()
        .collect();
    if !orgs.is_empty() {
        return orgs;
    }
    suggestions
        .iter()
        .filter(|s| s.starts_with("repo:"))
        .take(10)
        .cloned()
        .collect()
}

pub fn init() -> App {
    #[cfg(target_arch = "wasm32")]
    let token = crate::store::token();
    #[cfg(not(target_arch = "wasm32"))]
    let token = None;
    #[cfg(not(target_arch = "wasm32"))]
    if crate::scene_run() {
        return App::new(Store::memory(), token);
    }
    App::new(Store::open(), token)
}

pub fn setup(s: &mut App, waker: Waker) {
    s.net.waker = Some(waker);
}

/// Shows the peeking cat for `PEEK_SECS` at a random time and place while a PR is open, and
/// says whether it came or went.
fn peek(s: &mut App, now: f64, tx: &mut Tx) -> bool {
    if s.current().is_none() || s.game_open {
        return s.peek.take().is_some();
    }
    let wait = |seed: &mut u32| PEEK_WAIT.0 + f64::from(crate::cat::random(seed) % PEEK_WAIT.1);
    let due = match s.peek_due {
        Some(due) => due,
        None => *s.peek_due.insert(now + wait(&mut s.peek_seed)),
    };
    let flips = now >= due;
    if flips {
        let at = crate::cat::random(&mut s.peek_seed) % 1000;
        s.peek = match s.peek {
            Some(_) => None,
            None => Some(at as f32 / 1000.0),
        };
        let next = match s.peek {
            Some(_) => PEEK_SECS,
            None => wait(&mut s.peek_seed),
        };
        s.peek_due = Some(now + next);
        tx.animated();
    }
    let left = s.peek_due.unwrap_or(now) - now;
    tx.wake_after(std::time::Duration::from_secs_f64(left.max(0.0)));
    flips
}

pub fn poll(s: &mut App, now: f64, tx: &mut Tx) -> bool {
    let msgs = s.net.drain();
    let mut changed = !msgs.is_empty();
    if changed {
        tx.instant();
    }
    for msg in msgs {
        s.apply(msg);
    }
    if let Some(id) = s.reveal_row.take() {
        tx.scroll_into_view(id);
    }
    if s.loading && s.prs.is_empty() {
        // Rune lands every entrance of the first frame at once, so the first stack waits.
        let due = *s.boot_due.get_or_insert(now + 0.15);
        let due = match now >= due {
            true => {
                s.boot_stack += 1;
                changed = true;
                tx.animated();
                now + BOOT_STACK_SECS
            }
            false => due,
        };
        s.boot_due = Some(due);
        tx.wake_after(std::time::Duration::from_secs_f64(due - now));
    }
    if s.game_shows() {
        s.cat.width = s.pane_width;
        s.cat.tick(now);
        changed = true;
        tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 30.0));
    } else {
        s.cat.pause();
    }
    changed |= peek(s, now, tx);
    if s.toast.is_some() {
        let due = *s.toast_due.get_or_insert(now + TOAST_SECS);
        if now >= due {
            s.toast = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(due - now));
        }
    }
    changed
}

impl App {
    pub fn new(store: Store, token: Option<String>) -> App {
        let settings = store.settings().clone();
        let mut tree_width = PaneState::new(240.0).range(180.0, 480.0);
        if let Some(w) = settings.tree_width {
            tree_width = tree_width.with_width(w);
        }
        App {
            settings_open: cfg!(target_arch = "wasm32") && token.is_none(),
            keys_open: false,
            hinting: false,
            hint_keys: Vec::new(),
            hint_typed: String::new(),
            net: Net::new(token),
            store,
            picked: settings.authors.iter().cloned().collect(),
            settings,
            me: String::new(),
            suggestions: Vec::new(),
            prs: Vec::new(),
            incoming: Vec::new(),
            shown: Vec::new(),
            rows: Vec::new(),
            authors: Vec::new(),
            quick: Quick::default(),
            pr_search: TextEdit::new(""),
            generation: 0,
            viewer_asked: 0,
            loading: true,
            capped: false,
            error: None,
            toast: None,
            toast_due: None,
            boot_stack: 0,
            boot_due: None,
            selected: None,
            load: 0,
            cache: Vec::new(),
            ahead: BTreeSet::new(),
            detail: None,
            diff: split(DiffState::new(Vec::new())),
            patch: String::new(),
            diff_ready: false,
            diff_error: None,
            tree: TreeModel::new(),
            tree_search: TextEdit::new(""),
            tree_width,
            pane_width: 0.0,
            cat: Default::default(),
            game_open: false,
            peek: None,
            peek_due: None,
            peek_seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.subsec_nanos() | 1),
            pane_height: 0.0,
            find_open: false,
            find_edit: TextEdit::new(""),
            find_matches: Vec::new(),
            find_at: None,
            conversation_open: true,
            show_all: false,
            drafts: HashMap::new(),
            composing: None,
            comment_edit: TextEdit::new(""),
            review_menu: false,
            ci_menu: false,
            sort_menu: false,
            more_authors: false,
            naming_group: false,
            group_edit: TextEdit::new(""),
            review: None,
            review_edit: TextEdit::new(""),
            sending: false,
            posting: Vec::new(),
            ci_action: None,
            scope_edit: TextEdit::new(""),
            scope_error: None,
            token_edit: TextEdit::new(""),
            sidebar: PaneState::new(340.0).range(240.0, 560.0),
            thread_at: None,
            resolving: BTreeSet::new(),
            opened_threads: BTreeSet::new(),
            pr_comment: TextEdit::new(""),
            commenting: BTreeSet::new(),
            reply_to: None,
            reply_edit: TextEdit::new(""),
            reply_sending: false,
            editing: None,
            deleting: None,
            delete_sending: false,
            unplaced_open: false,
            threads_pane: false,
            detail_due: false,
            reveal_row: None,
        }
    }

    /// Sends the first request. `App::on_ready` calls it, after the first frame.
    pub fn start(&mut self) {
        self.load_viewer();
    }

    pub fn pr(&self, id: &str) -> Option<&Pr> {
        self.prs.iter().find(|p| p.id == id)
    }

    pub fn current(&self) -> Option<&Pr> {
        self.pr(self.selected.as_deref()?)
    }

    fn drafts_at(&self, id: &str, head: &str) -> &[Draft] {
        self.drafts
            .get(&(id.to_owned(), head.to_owned()))
            .map_or(&[], Vec::as_slice)
    }

    /// The unsent line comments for the head commit of the open diff.
    pub fn current_drafts(&self) -> &[Draft] {
        match (&self.selected, self.current_detail()) {
            (Some(id), Some(d)) => self.drafts_at(id, &d.head_commit),
            _ => &[],
        }
    }

    /// The count of unsent line comments of a PR, whatever head they were written on.
    pub fn draft_count(&self, pr: &Pr) -> usize {
        self.drafts
            .iter()
            .filter(|((id, _), _)| *id == pr.id)
            .map(|(_, list)| list.len())
            .sum()
    }

    /// The unsent line comments of the open PR the diff does not show, each with its head and
    /// its place in that head's list: those written on another head, which are never sent, and
    /// those on a file the shown diff does not have.
    pub fn drafts_off_diff(&self) -> Vec<(&str, usize, &Draft)> {
        let (Some(id), Some(d)) = (&self.selected, self.current_detail()) else {
            return Vec::new();
        };
        self.drafts
            .iter()
            .filter(|((pr, _), _)| pr == id)
            .flat_map(|((_, head), list)| {
                let older = *head != d.head_commit;
                list.iter()
                    .enumerate()
                    .filter(move |(_, draft)| {
                        older || (self.diff_ready && self.diff.file_index(&draft.path).is_none())
                    })
                    .map(move |(n, draft)| (head.as_str(), n, draft))
            })
            .collect()
    }

    fn current_detail(&self) -> Option<&Detail> {
        self.detail.as_ref()?.as_ref().ok()
    }

    /// Whether a scope picks the PRs. With none the list asks for one in Settings.
    pub fn has_scope(&self) -> bool {
        self.settings.scopes.iter().any(|s| model::is_scope(s))
    }

    /// The viewer may review the open PR: it is not theirs, and its diff is on screen.
    pub fn can_review(&self) -> bool {
        let theirs = self.current().is_some_and(|pr| pr.author != self.me);
        theirs && self.diff_ready && !self.sending
    }

    /// As `can_review`, unless the head commit of the diff is approved and no line comment waits.
    pub fn can_approve(&self) -> bool {
        let approved = match (self.current(), self.current_detail()) {
            (Some(pr), Some(detail)) => model::approved_at(&pr.my_reviews, &detail.head_commit),
            _ => false,
        };
        self.can_review() && (!approved || !self.current_drafts().is_empty())
    }

    /// A CI action may start on the open PR: its diff is on screen and no action is on the way.
    pub fn can_run_ci(&self) -> bool {
        self.current().is_some() && self.diff_ready && self.ci_action.is_none()
    }

    /// As `can_run_ci`, and GitHub lets the viewer update the branch.
    pub fn can_update_branch(&self) -> bool {
        self.can_run_ci() && self.current_detail().is_some_and(|d| d.can_update_branch)
    }

    fn load_viewer(&mut self) {
        self.loading = true;
        self.viewer_asked += 1;
        let asked = self.viewer_asked;
        self.net.send(github::viewer(), move |a| Msg::Viewer {
            asked,
            answer: a.and_then(|t| github::parse_viewer(&t)),
        });
    }

    /// Reloads the list and the open PR. With no scope it sends nothing: an empty search lists
    /// every open PR.
    pub fn refresh(&mut self) {
        if self.me.is_empty() {
            return self.load_viewer();
        }
        self.generation += 1;
        self.error = None;
        self.incoming.clear();
        if !self.has_scope() {
            self.loading = false;
            return self.set_prs(Vec::new());
        }
        self.cache.clear();
        self.ahead.clear();
        self.reload();
        self.loading = true;
        self.fetch_page(0, None);
    }

    fn fetch_page(&mut self, page: usize, after: Option<String>) {
        let generation = self.generation;
        let req = github::prs(&self.settings.scopes, &self.me, after.as_deref());
        self.net.send(req, move |a| Msg::Prs {
            generation,
            page,
            answer: a.and_then(|t| github::parse_prs(&t, page)),
        });
    }

    /// Shows the pages of the list that arrived. Until the list is complete, they update the
    /// rows they carry and add the new ones, so a refresh never shrinks the list.
    fn show_pages(&mut self, pages: Vec<Pr>, complete: bool) {
        if complete {
            return self.set_prs(pages);
        }
        let mut prs = std::mem::take(&mut self.prs);
        for pr in pages {
            match prs.iter_mut().find(|p| p.id == pr.id) {
                Some(old) => *old = pr,
                None => prs.push(pr),
            }
        }
        self.set_prs(prs);
    }

    /// Shows a new list, and closes the open PR when the list no longer holds it.
    fn set_prs(&mut self, prs: Vec<Pr>) {
        self.prs = prs;
        if self.selected.is_some() && self.current().is_none() {
            self.close_pr();
        }
        self.refilter();
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Viewer { asked, .. } if asked != self.viewer_asked => {}
            Msg::Viewer { answer: Ok(v), .. } => {
                self.me = v.login;
                if self.settings.scopes.is_empty() {
                    self.settings.scopes = first_scopes(&v.suggestions);
                }
                self.suggestions = v.suggestions;
                self.refresh();
            }
            Msg::Prs { generation, .. } if generation != self.generation => {}
            Msg::Prs {
                page,
                answer: Ok((prs, next)),
                ..
            } => {
                self.incoming.extend(prs);
                if let Next::Page(cursor) = next {
                    self.fetch_page(page + 1, Some(cursor));
                    return self.show_pages(self.incoming.clone(), false);
                }
                self.loading = false;
                self.capped = next == Next::PastCap;
                let prs = std::mem::take(&mut self.incoming);
                self.show_pages(prs, true);
            }
            Msg::Prs { answer: Err(e), .. } if !self.incoming.is_empty() => {
                self.loading = false;
                self.error = Some(format!("The list is not complete: {e}"));
                let prs = std::mem::take(&mut self.incoming);
                self.show_pages(prs, false);
            }
            Msg::Viewer { answer: Err(e), .. } | Msg::Prs { answer: Err(e), .. } => {
                self.loading = false;
                self.error = Some(e);
            }
            Msg::Ahead { generation, .. } | Msg::AheadDiff { generation, .. }
                if generation != self.generation => {}
            Msg::Ahead {
                id, answer: Ok(d), ..
            } => self.fetch_ahead_diff(id, d),
            Msg::AheadDiff {
                id,
                detail,
                answer: Ok(patch),
                ..
            } => {
                self.ahead.remove(&id);
                let head = detail.head_commit.clone();
                self.keep(Loaded {
                    id,
                    head,
                    detail,
                    patch,
                });
            }
            Msg::Ahead { id, .. } | Msg::AheadDiff { id, .. } => {
                self.ahead.remove(&id);
            }
            Msg::Detail { load, .. } | Msg::Diff { load, .. } if load != self.load => {}
            Msg::Detail { answer: Err(e), .. } if self.current_detail().is_some() => {
                self.detail_due = false;
                self.notify(format!("The PR did not reload: {e}"), true);
                if !self.diff_ready && self.diff_error.is_none() {
                    self.fetch_diff();
                }
            }
            Msg::Detail { answer, .. } => {
                self.detail_due = false;
                let pair = (self.current_detail(), answer.as_ref().ok());
                let same_head =
                    matches!(pair, (Some(old), Some(new)) if old.head_commit == new.head_commit);
                let same_base =
                    matches!(pair, (Some(old), Some(new)) if old.base_commit == new.base_commit);
                if !same_head {
                    self.add_comment();
                }
                if let (Ok(d), Some(id)) = (&answer, &self.selected)
                    && let Some(pr) = self.prs.iter_mut().find(|p| &p.id == id)
                {
                    pr.head = d.head_commit.clone();
                    pr.ci = d.ci;
                }
                self.detail = Some(answer);
                if !(same_head && self.diff_ready) {
                    self.clear_diff();
                    self.fetch_diff();
                } else if same_base {
                    self.remember();
                } else {
                    self.fetch_diff();
                }
                self.annotate();
                self.refilter();
            }
            Msg::Diff { answer, .. } => match answer {
                Ok(patch) => self.loaded(patch),
                Err(e) if self.diff_ready => {
                    self.notify(format!("The diff did not reload: {e}"), true);
                }
                Err(e) => self.diff_error = Some(e),
            },
            Msg::Submitted {
                id,
                head,
                sent,
                answer,
            } => self.submitted(&id, &head, &sent, answer),
            Msg::Runs(answer) => self.runs_arrived(answer),
            Msg::Reran(answer) => self.reran(answer),
            Msg::BranchUpdated(answer) => {
                let Some(action) = self.ci_action.take() else {
                    return;
                };
                let name = self.pr_name(&action.id);
                match answer {
                    Ok(()) => {
                        self.notify(format!("Updated the branch of {name}"), false);
                        self.ci_done(&action.id);
                    }
                    Err(e) => {
                        self.notify(format!("Update of the branch of {name} failed: {e}"), true)
                    }
                }
            }
            Msg::Resolved {
                id,
                thread,
                resolve,
                answer,
            } => self.resolved(&id, &thread, resolve, answer),
            Msg::LineSent {
                id,
                head,
                draft,
                answer,
            } => self.line_sent(&id, &head, draft, answer),
            Msg::Commented { id, answer } => self.commented(&id, answer),
            Msg::Replied { id, thread, answer } => self.replied(&id, &thread, answer),
            Msg::Edited {
                id,
                entry,
                body,
                answer,
            } => self.edited(&id, &entry, &body, answer),
            Msg::Deleted { id, entry, answer } => self.deleted(&id, &entry, answer),
        }
    }

    fn fetch_detail(&mut self, id: &str) {
        self.detail_due = true;
        let load = self.load;
        self.net.send(github::detail(id), move |a| Msg::Detail {
            load,
            answer: a.and_then(|t| github::parse_detail(&t)),
        });
    }

    fn fetch_diff(&mut self) {
        let (Some(pr), Some(Ok(d))) = (self.current(), &self.detail) else {
            return;
        };
        let load = self.load;
        self.net
            .send(github::diff(&pr.repo, d), move |a| Msg::Diff {
                load,
                answer: a,
            });
    }

    /// Loads the open PR again. Answers still on the way for the earlier load are dropped.
    pub fn reload(&mut self) {
        if let Some(id) = self.selected.clone() {
            self.load += 1;
            self.fetch_detail(&id);
        }
    }

    /// Shows a diff answer, unless the same patch is already on screen, and caches it.
    fn loaded(&mut self, patch: String) {
        if !(self.diff_ready && patch == self.patch) {
            self.show_files(parse(&patch));
        }
        self.patch = patch;
        self.remember();
    }

    /// Keeps the open PR and its shown patch in the cache under the detail's head.
    fn remember(&mut self) {
        let (Some(id), Some(detail)) = (self.selected.clone(), self.current_detail().cloned())
        else {
            return;
        };
        let head = detail.head_commit.clone();
        let patch = self.patch.clone();
        self.keep(Loaded {
            id,
            head,
            detail,
            patch,
        });
    }

    /// Puts a loaded PR in the cache, in place of its copy at the same head.
    fn keep(&mut self, loaded: Loaded) {
        self.cache
            .retain(|c| c.id != loaded.id || c.head != loaded.head);
        self.cache.push(loaded);
        if self.cache.len() > CACHE_SIZE {
            self.cache.remove(0);
        }
    }

    /// Loads the detail and the diff of the next PRs the drawer lists, unless the cache holds
    /// one at its listed head or its load is on the way.
    fn load_ahead(&mut self) {
        let listed = self.listed();
        let open = self.selected.as_ref();
        let Some(at) = listed
            .iter()
            .position(|&(_, i)| Some(&self.prs[i].id) == open)
        else {
            return;
        };
        for &(_, i) in listed.iter().skip(at + 1).take(AHEAD) {
            let pr = &self.prs[i];
            let cached = self
                .cache
                .iter()
                .any(|c| c.id == pr.id && c.head == pr.head);
            if cached || !self.ahead.insert(pr.id.clone()) {
                continue;
            }
            let generation = self.generation;
            let id = pr.id.clone();
            self.net.send(github::detail(&id), move |a| Msg::Ahead {
                generation,
                id,
                answer: a.and_then(|t| github::parse_detail(&t)),
            });
        }
    }

    fn fetch_ahead_diff(&mut self, id: String, detail: Detail) {
        let Some(repo) = self.pr(&id).map(|p| p.repo.clone()) else {
            self.ahead.remove(&id);
            return;
        };
        let generation = self.generation;
        self.net
            .send(github::diff(&repo, &detail), move |a| Msg::AheadDiff {
                generation,
                id,
                detail,
                answer: a,
            });
    }

    /// Replaces the shown diff in the same Unified or Split mode, with a new tree, all folders
    /// open, and the find matches of the new lines. An open comment box is kept as an unsent
    /// comment first.
    fn replace_diff(&mut self, files: Vec<FileDiff>) {
        self.add_comment();
        let mode = self.diff.mode();
        self.diff = DiffState::new(files);
        self.diff.set_mode(mode);
        let changes = |f: &FileDiff| (f.additions() as u32, f.deletions() as u32);
        self.tree = FileTree::new().build(self.diff.files().iter().map(|f| (f.path(), changes(f))));
        self.tree.expand_all();
        self.tree_search.clear();
        self.find_matches = find_lines(self.diff.files(), &self.find_edit.text);
        self.find_at = None;
    }

    /// Shows a new diff with every file folded.
    pub(crate) fn show_files(&mut self, files: Vec<FileDiff>) {
        self.replace_diff(files);
        self.diff.set_collapsed_where(|_| true);
        self.diff_ready = true;
        self.annotate();
    }

    /// The repository and number of a PR, for a toast.
    fn pr_name(&self, id: &str) -> String {
        self.pr(id)
            .map_or("the PR".into(), |p| format!("{}#{}", p.repo, p.number))
    }

    /// Sends the re-runs the action asked for, unless the PR or its head is no longer open.
    fn runs_arrived(&mut self, answer: Result<Vec<Run>, String>) {
        let Some(mut action) = self.ci_action.take() else {
            return;
        };
        let name = self.pr_name(&action.id);
        let runs = match answer {
            Ok(runs) => runs,
            Err(e) => return self.notify(format!("Re-run on {name} failed: {e}"), true),
        };
        let open = self.selected.as_deref() == Some(action.id.as_str())
            && self
                .current_detail()
                .is_some_and(|d| d.head_commit == action.head);
        if !open {
            let head = &action.head;
            return self.notify(
                format!("No re-run sent: {name} at {head:.7} is no longer open."),
                true,
            );
        }
        let Some(which) = action.rerun else {
            return;
        };
        let picked: Vec<u64> = runs
            .iter()
            .filter(|r| which.wants(r))
            .map(|r| r.id)
            .collect();
        if picked.is_empty() {
            let text = if runs.is_empty() {
                format!("{name} has no GitHub Actions run on this commit.")
            } else {
                format!("{name} has no finished run to re-run.")
            };
            return self.notify(text, false);
        }
        for &run in &picked {
            self.net.send(github::rerun(&action.repo, run, which), |a| {
                Msg::Reran(a.map(|_| ()))
            });
        }
        action.sent = picked.len();
        action.due = picked.len();
        self.ci_action = Some(action);
    }

    /// Counts one re-run answer. The last one ends the action with one toast.
    fn reran(&mut self, answer: Result<(), String>) {
        let Some(action) = self.ci_action.as_mut() else {
            return;
        };
        action.due = action.due.saturating_sub(1);
        if let Err(e) = answer {
            action.errors.push(e);
        }
        if action.due > 0 {
            return;
        }
        let Some(action) = self.ci_action.take() else {
            return;
        };
        let name = self.pr_name(&action.id);
        let runs = |n: usize| {
            if n == 1 {
                "1 run".to_owned()
            } else {
                format!("{n} runs")
            }
        };
        match action.errors.first() {
            None => self.notify(format!("Re-ran {} of {name}", runs(action.sent)), false),
            Some(e) => {
                let failed = action.errors.len();
                let text = format!(
                    "Re-run of {failed} of {} of {name} failed: {e}",
                    runs(action.sent)
                );
                self.notify(text, true);
            }
        }
        if action.errors.len() < action.sent {
            self.ci_done(&action.id);
        }
    }

    /// After a CI action: drops the cached PR and reloads it when it is open.
    fn ci_done(&mut self, id: &str) {
        self.cache.retain(|c| c.id != id);
        if self.selected.as_deref() == Some(id) {
            self.reload();
        }
    }

    /// Shows a toast for its full time, even over one still showing.
    fn notify(&mut self, text: String, failed: bool) {
        self.toast = Some(Notice { text, failed });
        self.toast_due = None;
    }

    fn submitted(
        &mut self,
        id: &str,
        head: &str,
        sent: &[Draft],
        answer: Result<MyReview, String>,
    ) {
        self.sending = false;
        let name = self.pr_name(id);
        let review = match answer {
            Ok(review) => review,
            Err(e) => {
                self.notify(format!("Review of {name} failed: {e}"), true);
                return;
            }
        };
        let verb = match review.state {
            ReviewState::Approved => "Approved",
            ReviewState::Commented => "Commented on",
            ReviewState::ChangesRequested => "Requested changes on",
        };
        let mut text = format!("{verb} {name}");
        if let Some(pr) = self.prs.iter_mut().find(|p| p.id == id) {
            pr.my_reviews.push(review);
        }
        if let Some(drafts) = self.drafts.get_mut(&(id.to_owned(), head.to_owned())) {
            for d in sent {
                if let Some(at) = drafts.iter().position(|x| x == d) {
                    drafts.remove(at);
                }
            }
        }
        self.cache.retain(|c| c.id != id);
        if self.selected.as_deref() == Some(id) {
            self.review = None;
            self.review_edit = TextEdit::new("");
            match self.step_after_review(id) {
                Some(next) => {
                    text = format!("{text}, now on {}", self.pr_name(&next));
                    self.reveal_row = Some(next);
                }
                None => {
                    self.annotate();
                    self.reload();
                }
            }
        }
        self.notify(text, false);
        self.refilter();
    }

    /// Opens the PR on the row below `id`, unless a comment box is open, and returns it.
    /// Returns None when it opened nothing.
    fn step_after_review(&mut self, id: &str) -> Option<String> {
        if self.composing.is_some() {
            return None;
        }
        self.step(1).filter(|next| next != id)
    }

    /// Sends a resolve of `thread` on the open PR, or an unresolve, unless one is on the way.
    pub fn resolve(&mut self, thread: &str, resolve: bool) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if !self.resolving.insert(thread.to_owned()) {
            return;
        }
        let thread = thread.to_owned();
        self.net
            .send(github::resolve(&thread, resolve), move |a| Msg::Resolved {
                id,
                thread,
                resolve,
                answer: a.and_then(|t| github::parse_resolve(&t, resolve)),
            });
    }

    /// Sets the thread from GitHub's answer in the open detail and the cache. An older detail
    /// request still on the way would show the old state, so a reload replaces it.
    fn resolved(
        &mut self,
        id: &str,
        thread: &str,
        resolve: bool,
        answer: Result<ThreadState, String>,
    ) {
        self.resolving.remove(thread);
        if self.selected.as_deref() != Some(id) {
            self.cache.retain(|c| c.id != id);
            return;
        }
        let state = match answer {
            Ok(state) => state,
            Err(e) => {
                let verb = if resolve { "Resolve" } else { "Unresolve" };
                let place = self
                    .current_detail()
                    .and_then(|d| d.threads.iter().find(|t| t.id == thread))
                    .map_or(String::new(), |t| match t.line.or(t.original_line) {
                        Some(line) => format!(" at {}:{line}", t.path),
                        None => format!(" on {}", t.path),
                    });
                let name = self.pr_name(id);
                return self.notify(
                    format!("{verb} of the thread{place} on {name} failed: {e}"),
                    true,
                );
            }
        };
        let set = |threads: &mut Vec<Thread>| {
            for t in threads.iter_mut().filter(|t| t.id == thread) {
                t.resolved = state.resolved;
                t.can_resolve = state.can_resolve;
                t.can_unresolve = state.can_unresolve;
            }
        };
        if let Some(Ok(d)) = self.detail.as_mut() {
            set(&mut d.threads);
        }
        for c in self.cache.iter_mut().filter(|c| c.id == id) {
            set(&mut c.detail.threads);
        }
        self.opened_threads.remove(thread);
        if self.detail_due {
            self.reload();
        }
        self.annotate();
    }

    /// Sends the text of the comment box as a comment on the open PR, unless the text is empty
    /// or a comment of that PR is on the way.
    pub fn send_comment(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let body = self.pr_comment.text.trim();
        if body.is_empty() || self.current_detail().is_none() || self.commenting.contains(&id) {
            return;
        }
        let req = github::add_comment(&id, body);
        self.commenting.insert(id.clone());
        self.net.send(req, move |a| Msg::Commented {
            id,
            answer: a.and_then(|t| github::parse_add_comment(&t)),
        });
    }

    /// Adds the new comment to the open detail and the cache, as `resolved` sets a thread.
    fn commented(&mut self, id: &str, answer: Result<Entry, String>) {
        self.commenting.remove(id);
        if self.selected.as_deref() != Some(id) {
            self.cache.retain(|c| c.id != id);
            return;
        }
        let name = self.pr_name(id);
        let entry = match answer {
            Ok(entry) => entry,
            Err(e) => return self.notify(format!("Comment on {name} failed: {e}"), true),
        };
        // A detail answer read after the comment already holds it.
        let add = |entries: &mut Vec<Entry>| {
            if !entries.contains(&entry) {
                entries.push(entry.clone());
            }
        };
        if let Some(Ok(d)) = self.detail.as_mut() {
            add(&mut d.entries);
        }
        for c in self.cache.iter_mut().filter(|c| c.id == id) {
            add(&mut c.detail.entries);
        }
        self.pr_comment = TextEdit::new("");
        if self.detail_due {
            self.reload();
        }
        self.notify(format!("Commented on {name}"), false);
    }

    /// Opens the reply box of `thread`, in place of one open on another thread.
    pub fn start_reply(&mut self, thread: &str) {
        if self.reply_to.as_deref() != Some(thread) {
            self.reply_to = Some(thread.to_owned());
            self.reply_edit = TextEdit::new("");
        }
        self.reply_edit.focused = true;
    }

    pub fn cancel_reply(&mut self) {
        self.reply_to = None;
        self.reply_edit = TextEdit::new("");
    }

    /// Sends the reply in the box, unless it is empty or a reply is on the way.
    pub fn send_reply(&mut self) {
        let (Some(id), Some(thread)) = (self.selected.clone(), self.reply_to.clone()) else {
            return;
        };
        let body = self.reply_edit.text.trim();
        if body.is_empty() || self.reply_sending {
            return;
        }
        self.reply_sending = true;
        let req = github::reply(&thread, body);
        self.net.send(req, move |a| Msg::Replied {
            id,
            thread,
            answer: a.and_then(|t| github::parse_reply(&t)),
        });
    }

    /// Adds the reply from GitHub's answer to its thread, and closes the box. A failure keeps
    /// the text in the box.
    fn replied(&mut self, id: &str, thread: &str, answer: Result<Entry, String>) {
        self.reply_sending = false;
        if self.selected.as_deref() != Some(id) {
            self.cache.retain(|c| c.id != id);
            return;
        }
        let name = self.pr_name(id);
        let entry = match answer {
            Ok(entry) => entry,
            Err(e) => return self.notify(format!("Reply on {name} failed: {e}"), true),
        };
        // A detail answer read after the reply already holds it.
        let add = |threads: &mut Vec<Thread>| {
            for t in threads.iter_mut().filter(|t| t.id == thread) {
                if !t.comments.contains(&entry) {
                    t.comments.push(entry.clone());
                }
            }
        };
        if let Some(Ok(d)) = self.detail.as_mut() {
            add(&mut d.threads);
        }
        for c in self.cache.iter_mut().filter(|c| c.id == id) {
            add(&mut c.detail.threads);
        }
        if self.reply_to.as_deref() == Some(thread) {
            self.cancel_reply();
        }
        if self.detail_due {
            self.reload();
        }
        self.notify(format!("Replied on {name}"), false);
    }

    /// The entry `id` of the open PR: its description, a conversation entry or a line comment.
    pub fn find_entry(&self, id: &str) -> Option<&Entry> {
        let d = self.current_detail()?;
        std::iter::once(&d.description)
            .chain(&d.entries)
            .chain(d.threads.iter().flat_map(|t| &t.comments))
            .find(|e| e.id == id)
    }

    /// Runs `change` on the open detail and on the cached copy of the PR `id`.
    fn change_detail(&mut self, id: &str, change: impl Fn(&mut Detail)) {
        if let Some(Ok(d)) = self.detail.as_mut() {
            change(d);
        }
        for c in self.cache.iter_mut().filter(|c| c.id == id) {
            change(&mut c.detail);
        }
    }

    /// Opens the edit box of the entry `id` with its text, in place of another one.
    pub fn start_edit(&mut self, id: &str) {
        let Some(e) = self.find_entry(id).filter(|e| e.can_edit) else {
            return;
        };
        let mut edit = TextEdit::new(&e.body);
        edit.focused = true;
        self.editing = Some(Editing {
            entry: id.to_owned(),
            kind: e.kind,
            edit,
            sending: false,
        });
        self.deleting = None;
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// Sends the text in the edit box, unless it is empty or a save is on the way.
    pub fn save_edit(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(e) = self.editing.as_mut() else {
            return;
        };
        let body = e.edit.text.trim().to_owned();
        if body.is_empty() || e.sending {
            return;
        }
        e.sending = true;
        let req = github::edit(e.kind, &e.entry, &body);
        let entry = e.entry.clone();
        self.net.send(req, move |a| Msg::Edited {
            id,
            entry,
            body,
            answer: a.and_then(|t| github::parse_done(&t)),
        });
    }

    /// Puts the saved text in place and closes the box. A failure keeps the box.
    fn edited(&mut self, id: &str, entry: &str, body: &str, answer: Result<(), String>) {
        if let Some(e) = self.editing.as_mut().filter(|e| e.entry == entry) {
            e.sending = false;
        }
        if self.selected.as_deref() != Some(id) {
            self.cache.retain(|c| c.id != id);
            return;
        }
        let name = self.pr_name(id);
        if let Err(e) = answer {
            return self.notify(format!("Edit on {name} failed: {e}"), true);
        }
        self.change_detail(id, |d| {
            let all = std::iter::once(&mut d.description)
                .chain(d.entries.iter_mut())
                .chain(d.threads.iter_mut().flat_map(|t| t.comments.iter_mut()));
            for e in all.filter(|e| e.id == entry) {
                e.body = body.to_owned();
            }
        });
        if self.editing.as_ref().is_some_and(|e| e.entry == entry) {
            self.editing = None;
        }
        self.notify(format!("Saved on {name}"), false);
    }

    /// The first click on Delete: the entry asks "Delete?".
    pub fn ask_delete(&mut self, id: &str) {
        if self.find_entry(id).is_some_and(|e| e.can_delete) {
            self.deleting = Some(id.to_owned());
        }
    }

    pub fn cancel_delete(&mut self) {
        if !self.delete_sending {
            self.deleting = None;
        }
    }

    /// Sends the delete the entry asked about, unless one is on the way.
    pub fn confirm_delete(&mut self) {
        let (Some(id), Some(entry)) = (self.selected.clone(), self.deleting.clone()) else {
            return;
        };
        let kind = self.find_entry(&entry).map(|e| e.kind);
        let req = kind.and_then(|k| github::delete(k, &entry));
        let Some(req) = req.filter(|_| !self.delete_sending) else {
            return;
        };
        self.delete_sending = true;
        self.net.send(req, move |a| Msg::Deleted {
            id,
            entry,
            answer: a.and_then(|t| github::parse_done(&t)),
        });
    }

    /// Takes the deleted entry out, and its thread when it was the last comment there.
    fn deleted(&mut self, id: &str, entry: &str, answer: Result<(), String>) {
        self.delete_sending = false;
        if self.deleting.as_deref() == Some(entry) {
            self.deleting = None;
        }
        if self.selected.as_deref() != Some(id) {
            self.cache.retain(|c| c.id != id);
            return;
        }
        let name = self.pr_name(id);
        if let Err(e) = answer {
            return self.notify(format!("Delete on {name} failed: {e}"), true);
        }
        self.change_detail(id, |d| {
            d.entries.retain(|e| e.id != entry);
            d.threads.retain_mut(|t| {
                let before = t.comments.len();
                t.comments.retain(|e| e.id != entry);
                t.comments.len() == before || !t.comments.is_empty()
            });
        });
        self.annotate();
        self.notify(format!("Deleted on {name}"), false);
    }

    /// Copies the link of the open PR, or its name such as `acme/api#12`, and says so in a toast.
    pub fn copy_pr(&mut self, link: bool, tx: &mut Tx) {
        let Some(pr) = self.current() else {
            return;
        };
        let name = self.pr_name(&pr.id);
        let (text, what) = match link {
            true => (pr.url.clone(), format!("the link of {name}")),
            false => (name.clone(), name),
        };
        tx.copy(text);
        self.notify(format!("Copied {what}"), false);
    }

    /// A comment of the open PR is on the way, so the box is off.
    pub fn sending_comment(&self) -> bool {
        let id = self.selected.as_ref();
        id.is_some_and(|id| self.commenting.contains(id))
    }

    /// `m`: opens the conversation, puts the cursor in the comment box and scrolls it into view.
    fn open_pr_comment(&mut self, tx: &mut Tx) {
        if self.current_detail().is_none() {
            return;
        }
        self.conversation_open = true;
        self.pr_comment.focused = !self.sending_comment();
        tx.scroll_into_view_in("conversation", "pr-comment");
    }

    /// Opens a resolved thread, or folds it again.
    pub fn toggle_thread(&mut self, thread: &str) {
        if !self.opened_threads.remove(thread) {
            self.opened_threads.insert(thread.to_owned());
        }
        self.annotate();
    }

    /// Where thread `n` of `d` sits in diff order: its file, its line, the old side before the
    /// new, then its place in the list, so two threads on one line both get a turn. A thread
    /// on a file the diff lacks comes last.
    fn thread_place(&self, d: &Detail, n: usize) -> (usize, u32, bool, usize) {
        match self.thread_spot(&d.threads[n]) {
            Some((file, at, _)) => (file, at.line, at.side == Side::New, n),
            None => (usize::MAX, 0, false, n),
        }
    }

    /// Where thread `t` shows in the diff: its file, the line it sits under, and whether that
    /// is the line it was made on. An outdated thread goes to the line that holds its code
    /// now; one with no such line, or a thread on a whole file, goes under the first line of
    /// its file. None for a file the diff lacks.
    pub fn thread_spot(&self, t: &Thread) -> Option<(usize, LineRef, bool)> {
        let file = self.diff.file_index(&t.path)?;
        let side = side(t.left);
        if let Some(line) = t.line {
            return Some((file, LineRef::new(side, line), true));
        }
        let hunks = &self.diff.files()[file].hunks;
        let lines: Vec<(u32, &str)> = hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter_map(|l| Some((if t.left { l.old } else { l.new }?, l.text.as_str())))
            .collect();
        let moved = t
            .original_line
            .and(model::relocate(&t.hunk, t.left, &lines));
        if let Some(line) = moved {
            return Some((file, LineRef::new(side, line), true));
        }
        let first = hunks.first()?.lines.first()?;
        let at = match (first.new, first.old) {
            (Some(n), _) => LineRef::new(Side::New, n),
            (None, Some(n)) => LineRef::new(Side::Old, n),
            (None, None) => return None,
        };
        Some((file, at, false))
    }

    /// The open threads the shown diff holds, as indexes into the detail's threads, in diff
    /// order.
    pub fn open_threads(&self) -> Vec<usize> {
        self.threads_where(|t| !t.resolved && self.thread_spot(t).is_some())
    }

    /// The threads that `keep` keeps, in diff order.
    fn threads_where(&self, keep: impl Fn(&Thread) -> bool) -> Vec<usize> {
        let Some(d) = self.current_detail() else {
            return Vec::new();
        };
        let mut found: Vec<_> = (0..d.threads.len())
            .filter(|&n| keep(&d.threads[n]))
            .map(|n| self.thread_place(d, n))
            .collect();
        found.sort_unstable();
        found.into_iter().map(|(.., n)| n).collect()
    }

    /// The groups of the list of threads, in order, each with its items. No group is empty.
    pub fn thread_groups(&self) -> Vec<(&'static str, Vec<Item>)> {
        let Some(d) = self.current_detail() else {
            return Vec::new();
        };
        let threads = |keep: fn(&Thread) -> bool| {
            let list = self.threads_where(keep);
            list.into_iter().map(Item::Thread).collect()
        };
        let groups: [(&str, Vec<Item>); 4] = [
            ("Open", threads(|t| !t.resolved && !t.outdated)),
            ("Outdated", threads(|t| !t.resolved && t.outdated)),
            ("Resolved", threads(|t| t.resolved)),
            ("Comments", (0..d.entries.len()).map(Item::Entry).collect()),
        ];
        groups
            .into_iter()
            .filter(|(_, items)| !items.is_empty())
            .collect()
    }

    /// A click in the list of threads. A thread gets its line selected, as `u` does, with a
    /// resolved one opened; a thread on a file the diff lacks opens the panel that holds it;
    /// an entry shows in the unfolded conversation.
    pub fn go_to(&mut self, item: Item, tx: &mut Tx) {
        let Some(d) = self.current_detail() else {
            return;
        };
        match item {
            Item::Thread(n) => {
                if let Some(t) = d.threads.get(n).filter(|t| t.resolved) {
                    self.opened_threads.insert(t.id.clone());
                    self.annotate();
                }
                if !self.go_to_thread(n, tx) {
                    self.unplaced_open = true;
                }
            }
            Item::Entry(n) => {
                self.conversation_open = true;
                self.show_all = true;
                tx.scroll_into_view_in("conversation", format!("entry-{n}"));
            }
        }
    }

    /// Opens the file of thread `n` and selects its line. Returns false for a thread on a file
    /// the diff does not have.
    fn go_to_thread(&mut self, n: usize, tx: &mut Tx) -> bool {
        let Some(t) = self.current_detail().and_then(|d| d.threads.get(n)) else {
            return false;
        };
        let Some((file, line, _)) = self.thread_spot(t) else {
            return false;
        };
        self.thread_at = Some(t.id.clone());
        self.diff.scroll_to_line(file, line, tx);
        self.diff.set_selection(Some(LineRange::line(file, line)));
        true
    }

    /// `u` or `Shift+U`: shows the next or previous open thread after the one last shown,
    /// around the ends, opens its file and selects its line.
    pub fn step_thread(&mut self, forward: bool, tx: &mut Tx) -> bool {
        let open = self.open_threads();
        let (Some(d), Some(&first), Some(&last)) =
            (self.current_detail(), open.first(), open.last())
        else {
            return false;
        };
        let place = |n: usize| self.thread_place(d, n);
        let kept = self
            .thread_at
            .as_ref()
            .and_then(|id| d.threads.iter().position(|t| &t.id == id))
            .filter(|&n| place(n).0 != usize::MAX);
        let at = match (kept, forward) {
            (None, true) => first,
            (None, false) => last,
            (Some(k), true) => *open
                .iter()
                .find(|&&n| place(n) > place(k))
                .unwrap_or(&first),
            (Some(k), false) => *open
                .iter()
                .rfind(|&&n| place(n) < place(k))
                .unwrap_or(&last),
        };
        self.go_to_thread(at, tx)
    }

    /// Recomputes the drawer's list. The open PR stays listed until another one is picked,
    /// so the drawer does not jump under the pointer.
    pub fn refilter(&mut self) {
        self.authors = model::author_chips(&self.prs, &self.picked);
        let mut shown = model::visible(
            &self.prs,
            &self.me,
            self.quick,
            &self.picked,
            &self.pr_search.text,
            self.settings.sort,
        );
        let open = self
            .selected
            .as_ref()
            .and_then(|id| self.prs.iter().position(|p| &p.id == id));
        if let Some(i) = open.filter(|i| !shown.contains(i)) {
            let sort = self.settings.sort;
            let at = shown.partition_point(|&j| {
                model::compare(sort, &self.me, &self.prs[j], &self.prs[i]).is_lt()
            });
            shown.insert(at, i);
        }
        let searching = !self.pr_search.text.trim().is_empty();
        self.rows = model::drawer_rows(
            &self.prs,
            &shown,
            &self.settings.folded,
            searching,
            self.settings.sort,
            &self.me,
        );
        self.shown = shown;
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort_menu = false;
        self.settings.sort = sort;
        self.store.save(&self.settings);
        self.refilter();
    }

    pub fn set_quick(&mut self, q: Quick) {
        self.quick = q;
        self.refilter();
    }

    pub fn toggle_author(&mut self, author: &str) {
        if !self.picked.remove(author) {
            self.picked.insert(author.to_owned());
        }
        self.save_picked();
    }

    pub fn clear_authors(&mut self) {
        self.picked.clear();
        self.save_picked();
    }

    /// Picks the authors of the group `name`, or clears them when they are already picked.
    pub fn toggle_author_group(&mut self, name: &str) {
        let Some(group) = self.settings.author_groups.get(name) else {
            return;
        };
        self.picked = if self.picked == *group {
            BTreeSet::new()
        } else {
            group.clone()
        };
        self.save_picked();
    }

    /// Saves the picked authors as a group under the typed name. A group with that name is
    /// replaced.
    pub fn save_group(&mut self) {
        let name = self.group_edit.text.trim().to_owned();
        if name.is_empty() || self.picked.is_empty() {
            return;
        }
        self.settings
            .author_groups
            .insert(name, self.picked.clone());
        self.store.save(&self.settings);
        self.close_group_dialog();
    }

    pub fn close_group_dialog(&mut self) {
        self.naming_group = false;
        self.group_edit = TextEdit::new("");
    }

    fn save_picked(&mut self) {
        self.settings.authors = self.picked.iter().cloned().collect();
        self.store.save(&self.settings);
        self.refilter();
    }

    /// Opens a PR: from the cache when it holds the listed head commit, else from GitHub. A PR
    /// from the cache still asks GitHub for its detail, to show new threads and commits. Then
    /// the next PRs load ahead.
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        let Some(head) = self.pr(id).map(|p| p.head.clone()) else {
            return;
        };
        self.close_pr();
        self.game_open = false;
        self.selected = Some(id.to_owned());
        self.refilter();
        match self.cache.iter().position(|c| c.id == id && c.head == head) {
            Some(i) => {
                let hit = self.cache.remove(i);
                self.detail = Some(Ok(hit.detail.clone()));
                self.show_files(parse(&hit.patch));
                self.patch = hit.patch.clone();
                self.cache.push(hit);
                self.fetch_detail(id);
            }
            None => self.fetch_detail(id),
        }
        self.load_ahead();
    }

    /// Leaves no PR open. Answers still on the way for the old one are dropped.
    fn close_pr(&mut self) {
        self.load += 1;
        self.detail_due = false;
        self.thread_at = None;
        self.opened_threads.clear();
        self.selected = None;
        self.detail = None;
        self.clear_diff();
        self.show_all = false;
        self.review = None;
        self.review_edit = TextEdit::new("");
        self.pr_comment = TextEdit::new("");
        self.cancel_reply();
        self.editing = None;
        self.deleting = None;
        self.unplaced_open = false;
        self.tree_search.focused = false;
        self.find_edit.focused = false;
    }

    fn clear_diff(&mut self) {
        self.replace_diff(Vec::new());
        self.patch.clear();
        self.diff_ready = false;
        self.diff_error = None;
    }

    pub fn start_hints(&mut self) {
        self.hinting = true;
        self.hint_keys.clear();
        self.hint_typed.clear();
    }

    pub fn stop_hints(&mut self) {
        self.hinting = false;
        self.hint_typed.clear();
    }

    /// The PRs the drawer lists, top to bottom: their row and their index into `prs`.
    fn listed(&self) -> Vec<(usize, usize)> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(n, r)| match r {
                Row::Pr(i) => Some((n, *i)),
                Row::Header { .. } => None,
            })
            .collect()
    }

    /// Selects the PR `step` rows away in the drawer, over the PRs it lists, and returns its id.
    pub fn step(&mut self, step: isize) -> Option<String> {
        let listed = self.listed();
        let next = match self.current() {
            None => 0,
            Some(open) => match listed.iter().position(|&(_, i)| self.prs[i].id == open.id) {
                Some(at) => at
                    .saturating_add_signed(step)
                    .min(listed.len().saturating_sub(1)),
                // The open PR is in a folded group: step from the group's header.
                None => {
                    let header = self.rows.iter().position(
                        |r| matches!(r, Row::Header { repo, .. } if *repo == open.repo),
                    )?;
                    let below = listed.partition_point(|&(n, _)| n < header);
                    if step > 0 {
                        below
                    } else {
                        below.checked_sub(1)?
                    }
                }
            },
        };
        let id = self.prs[listed.get(next)?.1].id.clone();
        self.select(&id);
        Some(id)
    }

    /// Puts review threads, unsent comments and the open comment box under their lines.
    /// Ids: `t<n>` a thread, `d<n>` an unsent comment, `compose` the comment box.
    pub fn annotate(&mut self) {
        let mut notes = Vec::new();
        let threads = self.current_detail().map_or(&[][..], |d| &d.threads);
        for (n, t) in threads.iter().enumerate() {
            if let Some((f, at, _)) = self.thread_spot(t) {
                notes.push(Annotation::new(f, at, format!("t{n}")));
            }
        }
        for (n, d) in self.current_drafts().iter().enumerate() {
            if let Some(f) = self.diff.file_index(&d.path) {
                notes.push(Annotation::new(
                    f,
                    LineRef::new(side(d.left), d.end),
                    format!("d{n}"),
                ));
            }
        }
        if let Some(r) = self.composing {
            notes.push(Annotation::new(r.file, r.end, "compose"));
        }
        self.diff.set_annotations(notes);
    }

    /// Opens the comment box under `lines`, also while a review is on the way, but never on
    /// the viewer's own PR.
    pub fn start_comment(&mut self, lines: LineRange) {
        if !self.current().is_some_and(|pr| pr.author != self.me) {
            return;
        }
        self.add_comment();
        self.composing = Some(lines);
        self.comment_edit = TextEdit::new("");
        self.comment_edit.focused = true;
        self.annotate();
    }

    /// Keeps the comment in the box as an unsent comment, which the next review sends.
    pub fn add_comment(&mut self) {
        if let Some((id, head, draft)) = self.take_comment() {
            self.drafts.entry((id, head)).or_default().push(draft);
        }
        self.annotate();
    }

    /// Sends the comment in the box at once, as a review with that one comment. Until the
    /// answer, it shows as an unsent comment that no review sends.
    pub fn send_line_comment(&mut self) {
        let Some((id, head, draft)) = self.take_comment() else {
            return self.annotate();
        };
        let key = (id.clone(), head.clone());
        self.drafts.entry(key).or_default().push(draft.clone());
        self.posting.push((id.clone(), head.clone(), draft.clone()));
        let req = github::submit(
            &id,
            &head,
            ReviewState::Commented,
            "",
            std::slice::from_ref(&draft),
        );
        self.net.send(req, move |a| Msg::LineSent {
            id,
            head,
            draft,
            answer: a.and_then(|t| github::parse_submit(&t)),
        });
        self.annotate();
    }

    /// Closes the comment box and returns its comment for the head of the open diff, unless
    /// the text is empty. A selection that crosses sides keeps only its end line.
    fn take_comment(&mut self) -> Option<(String, String, Draft)> {
        let r = self.composing.take()?;
        let id = self.selected.clone()?;
        let head = self.current_detail()?.head_commit.clone();
        let body = self.comment_edit.text.trim().to_owned();
        if body.is_empty() {
            return None;
        }
        let first = if r.start.side == r.end.side {
            r.start
        } else {
            r.end
        };
        let draft = Draft {
            path: self.diff.files()[r.file].path().to_owned(),
            start: first.line.min(r.end.line),
            end: first.line.max(r.end.line),
            left: r.end.side == Side::Old,
            body,
        };
        Some((id, head, draft))
    }

    /// Whether the open PR's unsent comment `d` is on the way to GitHub.
    pub fn is_posting(&self, d: &Draft) -> bool {
        let (Some(id), Some(detail)) = (&self.selected, self.current_detail()) else {
            return false;
        };
        let at = |p: &&(String, String, Draft)| p.0 == *id && p.1 == detail.head_commit;
        self.posting.iter().filter(at).any(|p| p.2 == *d)
    }

    /// On success, drops the unsent copy of the comment and reloads the PR so its thread
    /// shows. On failure, the comment stays unsent for the next review.
    fn line_sent(&mut self, id: &str, head: &str, draft: Draft, answer: Result<MyReview, String>) {
        let mine = |p: &(String, String, Draft)| p.0 == id && p.1 == head && p.2 == draft;
        if let Some(at) = self.posting.iter().position(mine) {
            self.posting.remove(at);
        }
        let review = match answer {
            Ok(review) => review,
            Err(e) => {
                let name = self.pr_name(id);
                let text =
                    format!("Comment on {name} was not sent: {e}. It goes with your review.");
                self.notify(text, true);
                return self.annotate();
            }
        };
        if let Some(pr) = self.prs.iter_mut().find(|p| p.id == id) {
            pr.my_reviews.push(review);
        }
        if let Some(list) = self.drafts.get_mut(&(id.to_owned(), head.to_owned()))
            && let Some(at) = list.iter().position(|d| *d == draft)
        {
            list.remove(at);
        }
        self.cache.retain(|c| c.id != id);
        if self.selected.as_deref() == Some(id) {
            self.reload();
        }
        self.annotate();
        self.refilter();
    }

    pub fn cancel_comment(&mut self) {
        self.composing = None;
        self.annotate();
    }

    pub fn remove_draft(&mut self, n: usize) {
        if let Some(head) = self.current_detail().map(|d| d.head_commit.clone()) {
            self.remove_draft_of(&head, n);
        }
    }

    /// Removes the `n`th unsent line comment the open PR keeps for `head`.
    pub fn remove_draft_of(&mut self, head: &str, n: usize) {
        if let Some(id) = &self.selected
            && let Some(d) = self.drafts.get_mut(&(id.clone(), head.to_owned()))
            && n < d.len()
        {
            d.remove(n);
        }
        self.annotate();
    }

    pub fn open_composer(&mut self, state: ReviewState) {
        if !self.can_review() {
            return;
        }
        self.review_menu = false;
        self.review = Some(state);
        self.review_edit.focused = true;
    }

    /// Sends a review of the head commit the open diff shows.
    pub fn submit(&mut self, state: ReviewState) {
        let allowed = match state {
            ReviewState::Approved => self.can_approve(),
            _ => self.can_review(),
        };
        let (Some(id), Some(detail)) = (self.selected.clone(), self.current_detail()) else {
            return;
        };
        if !allowed {
            self.notify("You can't send this review now.".into(), true);
            return;
        }
        let body = match self.review {
            Some(_) => self.review_edit.text.trim(),
            None => "",
        };
        let sent: Vec<Draft> = (self.current_drafts().iter())
            .filter(|d| !self.is_posting(d))
            .cloned()
            .collect();
        if state != ReviewState::Approved && body.is_empty() && sent.is_empty() {
            self.notify("Write something first.".into(), true);
            return;
        }
        let head = detail.head_commit.clone();
        let req = github::submit(&id, &head, state, body, &sent);
        self.sending = true;
        self.review_menu = false;
        self.net.send(req, move |a| Msg::Submitted {
            id,
            head,
            sent,
            answer: a.and_then(|t| github::parse_submit(&t)),
        });
    }

    /// Starts a CI action on the open PR at the head commit the diff shows, and returns its
    /// first request: the runs of that commit for a re-run, else the branch update.
    fn start_ci(&mut self, rerun: Option<Rerun>) -> Option<github::Request> {
        let (Some(pr), Some(d)) = (self.current(), self.current_detail()) else {
            return None;
        };
        let action = CiAction {
            id: pr.id.clone(),
            repo: pr.repo.clone(),
            head: d.head_commit.clone(),
            rerun,
            sent: 0,
            due: 0,
            errors: Vec::new(),
        };
        let req = match rerun {
            Some(_) => github::runs(&action.repo, &action.head),
            None => github::update_branch(&action.id, &action.head),
        };
        self.ci_action = Some(action);
        Some(req)
    }

    /// Re-runs the GitHub Actions runs of the head commit the open diff shows.
    pub fn rerun(&mut self, which: Rerun) {
        if !self.can_run_ci() {
            return;
        }
        if let Some(req) = self.start_ci(Some(which)) {
            self.net
                .send(req, |a| Msg::Runs(a.and_then(|t| github::parse_runs(&t))));
        }
    }

    /// Merges the base into the head commit the open diff shows.
    pub fn update_branch(&mut self) {
        if !self.can_update_branch() {
            return;
        }
        if let Some(req) = self.start_ci(None) {
            self.net.send(req, |a| {
                Msg::BranchUpdated(a.and_then(|t| github::parse_update_branch(&t)))
            });
        }
    }

    /// Gives up the keyboard focus, and shows the file at `path` from its header, open. A
    /// folder path shows nothing.
    pub fn jump_to_file(&mut self, path: &str, tx: &mut Tx) {
        tx.blur();
        self.show_file(path, tx);
    }

    /// Opens the file at `path` and scrolls the diff to it, and leaves the focus where it is.
    pub fn show_file(&mut self, path: &str, tx: &mut Tx) {
        let Some(file) = self.diff.file_index(path) else {
            return;
        };
        self.diff.set_collapsed(file, false);
        self.diff.scroll_to_file(file, tx);
    }

    /// Selects in the tree the file the diff shows at its top, and opens the folders above it
    /// unless the tree search filters the tree.
    pub fn follow_file(&mut self, file: usize) {
        let Some(path) = self.diff.files().get(file).map(|f| f.path().to_owned()) else {
            return;
        };
        if !self.tree.is_filtering() {
            self.tree.expand_to(&path);
        }
        self.tree.select(&path);
    }

    /// Folds every file when one is open, else opens every file but lock files, generated
    /// files and snapshots.
    pub fn fold_all(&mut self) {
        let fold = (0..self.diff.files().len()).any(|f| !self.diff.collapsed(f));
        let noise: Vec<bool> = self
            .diff
            .files()
            .iter()
            .map(|f| is_noise(f.path()))
            .collect();
        self.diff.set_collapsed_where(|f| fold || noise[f]);
    }

    /// `]` or `[`: shows the next or previous file, open.
    fn step_file(&mut self, forward: bool, tx: &mut Tx) -> bool {
        if !self.diff.next_file(forward, tx) {
            return false;
        }
        let file = self.diff.visible_file();
        self.diff.set_collapsed(file, false);
        true
    }

    /// `n` or `p`: the next or previous hunk of the open files. With none left, `n` first opens
    /// the folded file at the top; else both act as `]` and `[`.
    fn step_hunk(&mut self, forward: bool, tx: &mut Tx) -> bool {
        if self.diff.next_hunk(forward, tx) {
            return true;
        }
        let top = self.diff.visible_file();
        if forward && self.diff.collapsed(top) {
            self.diff.set_collapsed(top, false);
            if self.diff.next_hunk(true, tx) {
                return true;
            }
        }
        self.step_file(forward, tx)
    }

    pub fn set_oldest_first(&mut self, on: bool) {
        self.settings.oldest_first = on;
        self.store.save(&self.settings);
    }

    pub fn toggle_tree(&mut self) {
        self.settings.tree_closed = !self.settings.tree_closed;
        self.tree_search.focused &= !self.settings.tree_closed;
        self.store.save(&self.settings);
    }

    /// Opens or closes the drawer. A closed drawer gives up the cursor of its search field.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar.toggle();
        self.pr_search.focused &= self.sidebar.is_open();
    }

    /// The tree is on screen: the user has not closed it, and the pane has room for it.
    pub fn tree_shown(&self) -> bool {
        self.tree_fits() && !self.settings.tree_closed
    }

    /// The PR pane has room for the tree at its narrowest and, beside it, the room of the diff.
    pub fn tree_fits(&self) -> bool {
        self.pane_width >= self.tree_width.min_width() + DIFF_ROOM
    }

    /// The width the tree draws at: its own, narrowed so the diff keeps its room. The saved
    /// width does not change.
    pub fn tree_drawn_width(&self) -> f32 {
        self.tree_width.width().min(self.tree_room())
    }

    /// The widest tree that leaves the diff its room.
    fn tree_room(&self) -> f32 {
        (self.pane_width - DIFF_ROOM).floor()
    }

    /// Notes the width of the PR pane. A tree it hides gives up the cursor.
    /// The game shows: over the open PR, or in the empty pane once the head is gone.
    pub fn game_shows(&self) -> bool {
        self.game_open || (self.current().is_none() && !self.head_shows())
    }

    /// The winking cat head shows in the empty pane while the list loads.
    pub fn head_shows(&self) -> bool {
        !self.game_open && self.current().is_none() && self.loading && self.prs.is_empty()
    }

    pub fn open_game(&mut self) {
        self.game_open = true;
        self.peek = None;
    }

    pub fn set_pane_width(&mut self, width: f32) {
        self.pane_width = width;
        self.tree_search.focused &= self.tree_fits();
    }

    /// A drag on the edge of the tree, `travel` points from where it started at `start` points.
    /// The diff keeps its room.
    pub fn drag_tree(&mut self, start: f32, travel: f32) {
        self.tree_width
            .set_width((start + travel).min(self.tree_room()));
    }

    /// Saves the width of the tree, when it is not the default.
    pub fn save_tree_width(&mut self) {
        let width = self.tree_width.width();
        self.settings.tree_width = (width != self.tree_width.default_width()).then_some(width);
        self.store.save(&self.settings);
    }

    /// Puts back the saved width of the tree, after a cancelled drag.
    pub fn restore_tree_width(&mut self) {
        let saved = self.settings.tree_width;
        self.tree_width
            .set_width(saved.unwrap_or(self.tree_width.default_width()));
    }

    /// Puts the default width of the tree back, and saves it.
    pub fn reset_tree_width(&mut self) {
        self.tree_width.reset_width();
        self.save_tree_width();
    }

    /// The cursor is in one of the search fields or in the comment box, on screen.
    pub fn typing_in_search(&self) -> bool {
        let detail = self.current_detail().is_some();
        (self.pr_search.focused && self.sidebar.is_open())
            || (self.tree_search.focused && self.tree_shown() && self.diff_ready)
            || (self.find_edit.focused && self.find_open && detail)
            || (self.pr_comment.focused && self.conversation_open && detail)
            || (self.reply_edit.focused && self.reply_to.is_some() && detail)
            || (self.editing.as_ref().is_some_and(|e| e.edit.focused) && detail)
    }

    pub fn set_pr_search(&mut self, text: &str) {
        set_text(&mut self.pr_search, text);
        self.refilter();
    }

    /// Keeps in the tree the files whose path holds `text`, with the folders above them.
    pub fn set_tree_search(&mut self, text: &str) {
        set_text(&mut self.tree_search, text);
        if text.trim().is_empty() {
            self.tree.clear_filter();
        } else {
            self.tree
                .filter(|n| !n.dir && model::matches(&n.path, text));
        }
    }

    pub fn set_find(&mut self, text: &str) {
        set_text(&mut self.find_edit, text);
        self.find_matches = find_lines(self.diff.files(), text);
        self.find_at = None;
    }

    /// Shows the next or previous match of find, around the ends, and selects its line.
    pub fn step_find(&mut self, forward: bool, tx: &mut Tx) {
        let count = self.find_matches.len();
        if count == 0 {
            return;
        }
        let at = match (self.find_at, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(at), true) => (at + 1) % count,
            (Some(at), false) => (at + count - 1) % count,
        };
        let (file, line) = self.find_matches[at];
        self.find_at = Some(at);
        self.diff.scroll_to_line(file, line, tx);
        self.diff.set_selection(Some(LineRange::line(file, line)));
    }

    /// Closes find, empties it and clears the line it selected.
    pub fn close_find(&mut self) {
        self.find_open = false;
        self.find_edit.focused = false;
        self.set_find("");
        self.diff.set_selection(None);
    }

    /// Opens the drawer and puts the cursor in its search field.
    pub fn open_pr_search(&mut self) {
        self.sidebar.set_open(true);
        self.pr_search.focused = true;
    }

    /// Opens the tree and puts the cursor in its search field. Does nothing in a pane too
    /// narrow for the tree, or before the diff loads.
    pub fn open_tree_search(&mut self) {
        if !self.tree_fits() || !self.diff_ready {
            return;
        }
        if self.settings.tree_closed {
            self.toggle_tree();
        }
        self.threads_pane = false;
        self.tree_search.focused = true;
    }

    /// Opens find and puts the cursor in its field. Does nothing before the diff loads.
    pub fn open_find(&mut self) {
        if !self.diff_ready {
            return;
        }
        self.find_open = true;
        self.find_edit.focused = true;
    }

    /// Folds or opens the drawer group of `repo`. Does nothing while the PR search holds text,
    /// as the drawer then shows every group open.
    pub fn toggle_group(&mut self, repo: &str) {
        if !self.pr_search.text.trim().is_empty() {
            return;
        }
        if !self.settings.folded.remove(repo) {
            self.settings.folded.insert(repo.to_owned());
        }
        self.store.save(&self.settings);
        self.refilter();
    }

    pub fn add_scope(&mut self) {
        let scope = self.scope_edit.text.trim().to_owned();
        self.scope_edit = TextEdit::new("");
        self.push_scope(scope);
    }

    pub fn push_scope(&mut self, scope: String) {
        if !model::is_scope(&scope) {
            self.scope_error = Some(format!(
                "\"{scope}\" is not a scope: use org:<name>, user:<name> or repo:<owner>/<name>."
            ));
            return;
        }
        self.scope_error = None;
        if !self.settings.scopes.contains(&scope) {
            self.settings.scopes.push(scope);
        }
    }

    /// Closes Settings without Save: the scopes go back to the saved ones.
    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings.scopes = self.store.settings().scopes.clone();
        self.settings.author_groups = self.store.settings().author_groups.clone();
        if self.settings.scopes.is_empty() {
            self.settings.scopes = first_scopes(&self.suggestions);
        }
    }

    pub fn save_settings(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let token = self.token_edit.text.trim().to_owned();
            if !token.is_empty() {
                crate::store::save_token(&token);
                self.net.token = Some(token);
                self.token_edit = TextEdit::new("");
                self.me.clear();
            }
        }
        self.settings_open = false;
        self.store.save(&self.settings);
        self.refresh();
    }

    #[cfg(target_arch = "wasm32")]
    pub fn forget_token(&mut self) {
        crate::store::remove_token();
        self.net.token = None;
        self.me.clear();
    }
}

/// The side of the diff a thread or a comment on the old side (`left`) sits on.
fn side(left: bool) -> Side {
    if left { Side::Old } else { Side::New }
}

/// Puts `text` in a field, unless it already holds it, so the cursor stays where it is.
fn set_text(edit: &mut TextEdit, text: &str) {
    if edit.text != text {
        edit.replace_text(text);
    }
}

/// The lowercased character of a key typed with no modifier but Shift. Caps Lock changes the
/// case of the typed text, so the shortcuts read Shift from the modifiers.
fn letter(ev: &KeyEvent) -> Option<char> {
    let m = ev.mods;
    if m.cmd || m.ctrl || m.alt || m.logo {
        return None;
    }
    match &ev.key {
        Key::Text(t) if t.chars().count() == 1 => t.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    }
}

pub fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.toggle_sidebar();
        return true;
    }
    if s.keys_open {
        if letter(ev) == Some('?') {
            s.keys_open = false;
            return true;
        }
        return false;
    }
    if s.settings_open
        || s.naming_group
        || s.review.is_some()
        || s.composing.is_some()
        || s.typing_in_search()
    {
        return false;
    }
    if s.hinting {
        return hint_key(s, ev, tx);
    }
    if ev.key == Key::Escape && s.game_open {
        s.game_open = false;
        return true;
    }
    if ev.key == Key::Escape && tx.focused() == Some("files") {
        tx.blur();
        return true;
    }
    if ev.is_command('f') {
        s.open_find();
        return true;
    }
    // Ctrl+C copies selected text or diff lines first, else the PR's name.
    if ev.is_command('c') {
        if ev.mods.shift {
            s.copy_pr(true, tx);
        } else if tx.text_selection().is_some() {
            return false;
        } else if !s.diff.key(ev, tx) {
            s.copy_pr(false, tx);
        }
        return true;
    }
    let typed = letter(ev);
    match (typed, ev.mods.shift) {
        (Some('a'), true) => s.submit(ReviewState::Approved),
        (Some('r'), true) => s.reload(),
        (Some('t'), true) => s.open_tree_search(),
        (Some('u'), true) => return s.step_thread(false, tx),
        (Some('/'), _) => s.open_pr_search(),
        (Some('?'), _) => s.keys_open = true,
        (Some(' '), false) => s.start_hints(),
        _ => return plain_key(s, typed.filter(|_| !ev.mods.shift), ev, tx),
    }
    true
}

/// A key in hint mode: a letter that names a target presses it. Any key that names no
/// target, Escape among them, leaves the mode.
fn hint_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match letter(ev) {
        Some(c) if c.is_ascii_lowercase() => s.hint_typed.push(c),
        _ => {
            s.stop_hints();
            return true;
        }
    }
    let labels = keys::labels(s.hint_keys.len());
    match labels.iter().position(|l| *l == s.hint_typed) {
        Some(at) => {
            let key = s.hint_keys[at].clone();
            s.stop_hints();
            let hint = keys::targets(s).into_iter().find(|(k, _)| *k == key);
            if let Some((_, hint)) = hint {
                keys::run(hint, s, tx);
            }
        }
        None if !labels.iter().any(|l| l.starts_with(&s.hint_typed)) => s.stop_hints(),
        None => {}
    }
    true
}

/// The keys typed with no modifier, and the arrows.
fn plain_key(s: &mut App, typed: Option<char>, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match (typed, &ev.key) {
        (_, Key::Up) if s.game_shows() => s.cat.press(crate::cat::Move::Jump),
        (_, Key::Down) if s.game_shows() => s.cat.press(crate::cat::Move::Roll),
        (_, Key::Right) if s.game_shows() => s.cat.press(crate::cat::Move::Fire),
        (Some('j'), _) | (_, Key::Down) => {
            if let Some(id) = s.step(1) {
                tx.scroll_into_view(id);
            }
        }
        (Some('k'), _) | (_, Key::Up) => {
            if let Some(id) = s.step(-1) {
                tx.scroll_into_view(id);
            }
        }
        (Some('c'), _) => s.open_composer(ReviewState::Commented),
        (Some('m'), _) => s.open_pr_comment(tx),
        (Some('f'), _) => s.conversation_open = !s.conversation_open,
        (Some('t'), _) if s.tree_fits() => s.toggle_tree(),
        (Some('z'), _) => s.fold_all(),
        (Some('u'), _) => return s.step_thread(true, tx),
        (Some('n'), _) => return s.step_hunk(true, tx),
        (Some('p'), _) => return s.step_hunk(false, tx),
        (Some(']'), _) => return s.step_file(true, tx),
        (Some('['), _) => return s.step_file(false, tx),
        (Some('1'), _) => s.set_quick(Quick::ToReview),
        (Some('2'), _) => s.set_quick(Quick::ActedOn),
        (Some('3'), _) => s.set_quick(Quick::All),
        (Some('b'), _) => s.toggle_sidebar(),
        (Some('r'), _) => s.refresh(),
        (Some('o'), _) => {
            if let Some(url) = s.current().map(|p| p.url.clone()) {
                let _ = tx.open_url(&url);
            }
        }
        _ => return s.diff.key(ev, tx),
    }
    true
}

/// Enter or Shift+Enter, with no other modifier.
fn enter(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    ev.key == Key::Enter && !(m.cmd || m.ctrl || m.alt || m.logo)
}

/// The line comment box: Ctrl+Shift+Enter keeps the comment for the review. Ctrl+Enter, which
/// sends it at once, is the box's submit.
pub fn comment_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    let command = ev.mods.cmd || ev.mods.ctrl;
    if !(ev.key == Key::Enter && command && ev.mods.shift) {
        return false;
    }
    s.add_comment();
    true
}

/// The reply box: Escape closes it.
/// The edit box: Escape closes it.
pub fn edit_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    if ev.key != Key::Escape {
        return false;
    }
    s.cancel_edit();
    true
}

pub fn reply_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    if ev.key != Key::Escape {
        return false;
    }
    s.cancel_reply();
    true
}

/// The PR search field: Enter opens the first PR of the list that the search keeps, Escape
/// empties the field. Both leave the field.
pub fn pr_search_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if enter(ev) {
        let kept = model::visible(
            &s.prs,
            &s.me,
            s.quick,
            &s.picked,
            &s.pr_search.text,
            s.settings.sort,
        );
        let first = s.rows.iter().find_map(|r| match r {
            Row::Pr(i) if kept.contains(i) => Some(s.prs[*i].id.clone()),
            _ => None,
        });
        if let Some(id) = first {
            s.select(&id);
            tx.scroll_into_view(id);
        }
    } else if ev.key == Key::Escape {
        s.set_pr_search("");
    } else {
        return false;
    }
    tx.blur();
    true
}

/// The tree search field: Enter shows the first file the tree lists, Down puts the keyboard in
/// the tree on that file, Escape empties the field.
pub fn tree_search_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let first = (0..s.tree.visible_len())
        .filter_map(|row| s.tree.row(row))
        .find(|(node, _)| !node.dir)
        .map(|(node, _)| node.path.clone());
    if enter(ev) {
        match first {
            Some(path) => s.jump_to_file(&path, tx),
            None => tx.blur(),
        }
    } else if ev.key == Key::Down {
        if let Some(path) = first {
            s.tree.select(&path);
            s.show_file(&path, tx);
        }
        s.tree_search.focused = false;
        tx.focus("files", true);
    } else if ev.key == Key::Escape {
        s.set_tree_search("");
        tx.blur();
    } else {
        return false;
    }
    true
}

/// The find field: Enter shows the next match, Shift+Enter the previous one, Escape closes find.
pub fn find_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if enter(ev) {
        s.step_find(!ev.mods.shift, tx);
    } else if ev.key == Key::Escape {
        s.close_find();
        tx.blur();
    } else {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Request;
    use crate::model::{Entry, Mark};

    fn pr(id: &str, head: &str) -> Pr {
        Pr {
            id: id.into(),
            repo: "acme/api".into(),
            number: 1,
            title: id.into(),
            url: String::new(),
            author: "ann".into(),
            draft: false,
            head: head.into(),
            last_commit: "2026-11-14T10:00:00Z".into(),
            updated: String::new(),
            created: String::new(),
            additions: 1,
            deletions: 1,
            my_reviews: Vec::new(),
            ci: None,
            head_branch: String::new(),
            base_branch: String::new(),
            fork: false,
        }
    }

    fn detail(head: &str) -> Detail {
        Detail {
            base_branch: "main".into(),
            head_branch: "feature".into(),
            base_commit: "base".into(),
            head_commit: head.into(),
            ci: None,
            can_update_branch: false,
            conflicts: false,
            description: Entry {
                author: "ann".into(),
                ..Entry::default()
            },
            entries: Vec::new(),
            threads: Vec::new(),
            truncated: false,
        }
    }

    const PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 fn a() {}
-fn b() {}
+fn c() {}
 fn d() {}
";

    /// An app signed in as `me` with one scope and the given PRs listed.
    fn app(prs: Vec<Pr>) -> App {
        let mut app = App::new(Store::memory(), None);
        app.me = "me".into();
        app.settings.scopes = vec!["org:acme".into()];
        app.prs = prs;
        app.loading = false;
        app.refilter();
        app
    }

    fn sent(app: &App) -> Vec<Request> {
        std::mem::take(&mut *app.net.sent.lock().unwrap())
    }

    /// Four files: a folder inside a folder, a folder of its own, and a lock file at the top.
    const TREE_PATCH: &str = "diff --git a/src/net/retry.rs b/src/net/retry.rs
--- a/src/net/retry.rs
+++ b/src/net/retry.rs
@@ -1,3 +1,3 @@
 fn retry_send() {
-    Retry::new(3)
+    Retry::new(5)
 }
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
 mod net;
+mod retry;
 mod a;
diff --git a/config/retry.toml b/config/retry.toml
new file mode 100644
--- /dev/null
+++ b/config/retry.toml
@@ -0,0 +1 @@
+attempts = 5
diff --git a/Cargo.lock b/Cargo.lock
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,1 +1,1 @@
-version = 1
+version = 2
";

    /// Opens `id` and answers its detail and diff requests.
    fn open(app: &mut App, id: &str, head: &str) {
        open_with(app, id, head, PATCH);
    }

    fn open_with(app: &mut App, id: &str, head: &str, patch: &str) {
        app.select(id);
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail(head)),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(patch.into()),
        });
    }

    /// A key typed with no modifier.
    fn key(text: &str) -> KeyEvent {
        KeyEvent {
            key: Key::Text(text.into()),
            mods: Default::default(),
        }
    }

    /// The files the tree shows, top to bottom.
    fn tree_files(app: &App) -> Vec<&str> {
        (0..app.tree.visible_len())
            .filter_map(|row| app.tree.visible_id(row))
            .filter(|id| app.diff.file_index(id).is_some())
            .collect()
    }

    fn shown_ids(app: &App) -> Vec<&str> {
        app.shown.iter().map(|&i| app.prs[i].id.as_str()).collect()
    }

    #[test]
    fn a_sort_reorders_the_drawer_and_is_saved() {
        let created = |id: &str, day: &str| Pr {
            created: format!("2026-01-{day}T00:00:00Z"),
            ..pr(id, "h")
        };
        let mut app = app(vec![created("A", "01"), created("B", "02")]);
        app.quick = Quick::All;
        app.set_sort(Sort::Oldest);
        assert_eq!(shown_ids(&app), ["A", "B"]);
        app.set_sort(Sort::Newest);
        assert_eq!(shown_ids(&app), ["B", "A"]);
        assert_eq!(app.store.settings().sort, Sort::Newest);
    }

    #[test]
    fn a_late_answer_for_another_pr_does_not_replace_the_open_one() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        app.select("A");
        let load_a = app.load;
        app.select("B");
        app.apply(Msg::Detail {
            load: load_a,
            answer: Ok(detail("a1")),
        });
        app.apply(Msg::Diff {
            load: load_a,
            answer: Ok(PATCH.into()),
        });
        assert!(app.detail.is_none());
        assert!(app.diff.files().is_empty());

        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("b1")),
        });
        assert_eq!(app.current_detail().unwrap().head_commit, "b1");
    }

    #[test]
    fn a_stale_list_page_is_dropped_after_a_refresh() {
        let mut app = app(Vec::new());
        app.refresh();
        let old = app.generation;
        app.settings.scopes = vec!["org:other".into()];
        app.refresh();
        let new = app.generation;
        let page = |page, prs, next| Msg::Prs {
            generation: new,
            page,
            answer: Ok((prs, next)),
        };
        app.apply(page(0, vec![pr("NEW", "n1")], Next::Page("c".into())));
        app.apply(Msg::Prs {
            generation: old,
            page: 0,
            answer: Ok((vec![pr("OLD", "o1")], Next::Done)),
        });
        app.apply(page(1, vec![pr("NEW2", "n2")], Next::Done));
        let ids: Vec<&str> = app.prs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["NEW", "NEW2"]);
    }

    #[test]
    fn a_pr_i_just_reviewed_stays_listed_until_i_pick_another() {
        // A is on the last row, so the review keeps it open.
        let mut app = app(vec![pr("B", "b1"), pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.submit(ReviewState::Approved);
        app.apply(approved("A", "a1", Vec::new()));
        assert_eq!(
            model::mark(app.pr("A").unwrap(), "me"),
            Mark::Acted(ReviewState::Approved)
        );
        assert!(shown_ids(&app).contains(&"A"));

        app.select("B");
        assert_eq!(shown_ids(&app), ["B"]);
    }

    #[test]
    fn a_failed_review_keeps_the_line_comments() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 2)));
        app.comment_edit = TextEdit::new("rename this");
        app.add_comment();
        app.open_composer(ReviewState::Commented);
        app.review_edit = TextEdit::new("a few notes");
        app.submit(ReviewState::Commented);
        assert!(app.sending);
        app.apply(Msg::Submitted {
            id: "A".into(),
            head: "a1".into(),
            sent: app.current_drafts().to_vec(),
            answer: Err("No network".into()),
        });
        assert_eq!(app.current_drafts().len(), 1);
        assert_eq!(app.review_edit.text, "a few notes");
        assert!(app.toast.as_ref().is_some_and(|t| t.failed));
        assert!(!app.sending);
    }

    #[test]
    fn new_commits_load_a_fresh_diff() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        app.select("B");
        app.refresh();
        app.apply(Msg::Prs {
            generation: app.generation,
            page: 0,
            answer: Ok((vec![pr("A", "a2"), pr("B", "b1")], Next::Done)),
        });
        sent(&app);

        app.select("A");
        assert!(app.detail.is_none());
        assert!(app.diff.files().is_empty());
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a2")),
        });
        let urls: Vec<String> = sent(&app).into_iter().map(|r| r.url).collect();
        assert!(
            urls.iter().any(|u| u.ends_with("compare/base...a2")),
            "{urls:?}"
        );
    }

    #[test]
    fn a_push_after_the_list_loads_reviews_the_commit_the_diff_shows() {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a2")),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(PATCH.into()),
        });
        app.submit(ReviewState::Approved);
        let review = sent(&app).into_iter().last().unwrap();
        let body: serde_json::Value = serde_json::from_str(&review.body.unwrap()).unwrap();
        assert_eq!(body["variables"]["input"]["commitOID"], "a2");
    }

    #[test]
    fn approve_stays_on_when_only_an_older_head_was_approved() {
        let mut listed = pr("A", "a1");
        listed.my_reviews.push(MyReview {
            state: ReviewState::Approved,
            commit: "a1".into(),
        });
        let mut app = app(vec![listed]);
        open(&mut app, "A", "a2");
        assert!(app.can_approve());
    }

    fn comment(app: &mut App, lines: LineRange, body: &str) {
        app.start_comment(lines);
        app.comment_edit = TextEdit::new(body);
        app.add_comment();
    }

    fn approved(id: &str, commit: &str, sent: Vec<Draft>) -> Msg {
        Msg::Submitted {
            id: id.into(),
            head: commit.into(),
            sent,
            answer: Ok(MyReview {
                state: ReviewState::Approved,
                commit: commit.into(),
            }),
        }
    }

    /// Whether a detail request for `id` went out.
    fn detail_sent(app: &App, id: &str) -> bool {
        let id = format!("\"id\":\"{id}\"");
        sent(app)
            .iter()
            .any(|r| r.body.as_ref().is_some_and(|b| b.contains(&id)))
    }

    /// Opens the comment box on line 2 of the open PR and types `body`.
    fn write(app: &mut App, body: &str) {
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 2)));
        app.comment_edit = TextEdit::new(body);
    }

    fn line_sent(id: &str, head: &str, body: &str, answer: Result<MyReview, String>) -> Msg {
        Msg::LineSent {
            id: id.into(),
            head: head.into(),
            draft: Draft {
                path: "src/lib.rs".into(),
                start: 2,
                end: 2,
                left: false,
                body: body.into(),
            },
            answer,
        }
    }

    #[test]
    fn a_line_comment_sends_at_once_and_leaves_no_draft() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        sent(&app);
        write(&mut app, "typo");
        app.send_line_comment();
        let asked = sent(&app);
        assert_eq!(asked.len(), 1);
        let body = asked[0].body.as_deref().unwrap();
        assert!(body.contains("\"event\":\"COMMENT\"") && body.contains("typo"));
        let review = MyReview {
            state: ReviewState::Commented,
            commit: "a1".into(),
        };
        app.apply(line_sent("A", "a1", "typo", Ok(review)));
        assert!(app.current_drafts().is_empty());
        assert!(detail_sent(&app, "A"), "the PR reloads to show the thread");
    }

    #[test]
    fn a_kept_comment_sends_nothing_until_the_review() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        sent(&app);
        write(&mut app, "later");
        let keep = KeyEvent {
            key: Key::Enter,
            mods: rune::Mods {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        };
        assert!(comment_key(&mut app, &keep, &mut Tx::default()));
        assert!(sent(&app).is_empty());
        assert_eq!(app.current_drafts().len(), 1);
    }

    #[test]
    fn a_failed_line_comment_stays_unsent_and_a_review_skips_one_on_the_way() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        write(&mut app, "typo");
        app.send_line_comment();
        sent(&app);
        app.submit(ReviewState::Approved);
        let review = sent(&app);
        assert!(!review[0].body.as_deref().unwrap().contains("typo"));
        app.apply(line_sent("A", "a1", "typo", Err("No network".into())));
        let left = app.current_drafts();
        assert_eq!(left.len(), 1);
        assert!(!app.is_posting(&left[0]), "the next review sends it");
    }

    #[test]
    fn a_comment_written_while_a_review_is_sent_survives() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "first",
        );
        let sent = app.current_drafts().to_vec();
        app.submit(ReviewState::Approved);
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 3)),
            "second",
        );
        app.apply(approved("A", "a1", sent));
        let left: Vec<&str> = app
            .current_drafts()
            .iter()
            .map(|d| d.body.as_str())
            .collect();
        assert_eq!(left, ["second"]);
    }

    #[test]
    fn drafts_of_an_older_head_are_not_sent() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "old",
        );
        app.select("B");
        app.refresh();
        app.apply(Msg::Prs {
            generation: app.generation,
            page: 0,
            answer: Ok((vec![pr("A", "a2"), pr("B", "b1")], Next::Done)),
        });
        open(&mut app, "A", "a2");
        assert!(app.current_drafts().is_empty());
        sent(&app);
        app.submit(ReviewState::Approved);
        let review = sent(&app).into_iter().last().unwrap();
        let body: serde_json::Value = serde_json::from_str(&review.body.unwrap()).unwrap();
        assert_eq!(body["variables"]["input"]["threads"], serde_json::json!([]));
    }

    #[test]
    fn a_push_seen_on_reload_keeps_the_old_drafts_reachable() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "old",
        );
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 3)));
        app.comment_edit = TextEdit::new("typing");
        app.refresh();
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a2")),
        });
        assert!(app.current_drafts().is_empty());
        assert_eq!(app.draft_count(&app.prs[0]), 2);
        let mut older: Vec<&str> = app
            .drafts_off_diff()
            .iter()
            .map(|d| d.2.body.as_str())
            .collect();
        older.sort();
        assert_eq!(older, ["old", "typing"]);
    }

    #[test]
    fn a_closed_row_counts_drafts_of_a_newer_head() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a2");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "new",
        );
        app.select("B");
        assert_eq!(app.draft_count(&app.prs[0]), 1);
    }

    #[test]
    fn a_new_base_keeps_the_diff_until_the_new_patch_differs() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.diff.set_collapsed(0, true);
        let new_base = |app: &mut App, base: &str| {
            app.refresh();
            sent(app);
            app.apply(Msg::Detail {
                load: app.load,
                answer: Ok(Detail {
                    base_commit: base.into(),
                    ..detail("a1")
                }),
            });
            assert!(app.diff_ready, "the shown diff stays");
            let urls: Vec<String> = sent(app).into_iter().map(|r| r.url).collect();
            assert!(
                urls.iter()
                    .any(|u| u.ends_with(&format!("compare/{base}...a1"))),
                "{urls:?}"
            );
        };
        new_base(&mut app, "base2");
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(PATCH.into()),
        });
        assert!(
            app.diff.collapsed(0),
            "the same patch keeps the files on screen"
        );

        new_base(&mut app, "base3");
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(PATCH.replace("lib.rs", "main.rs")),
        });
        assert_eq!(app.diff.files()[0].path(), "src/main.rs");
    }

    #[test]
    fn a_review_on_a_head_newer_than_the_list_moves_the_row_to_acted_on() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a2");
        app.submit(ReviewState::Approved);
        app.apply(approved("A", "a2", Vec::new()));
        assert_eq!(
            model::mark(app.pr("A").unwrap(), "me"),
            Mark::Acted(ReviewState::Approved)
        );
    }

    #[test]
    fn the_picked_mode_stays_on_the_next_pr() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        app.diff.set_mode(DiffMode::Unified);
        open(&mut app, "B", "b1");
        assert_eq!(app.diff.mode(), DiffMode::Unified);
    }

    #[test]
    fn the_first_diff_opens_in_split_mode() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        assert_eq!(app.diff.mode(), DiffMode::Split);
    }

    /// Whether each file of the diff shows only its header.
    fn folded(app: &App) -> Vec<bool> {
        (0..app.diff.files().len())
            .map(|f| app.diff.collapsed(f))
            .collect()
    }

    #[test]
    fn z_folds_every_file_and_the_next_press_opens_them() {
        let mut app = app(vec![pr("A", "a1")]);
        open_with(&mut app, "A", "a1", TREE_PATCH);
        let mut tx = Tx::default();
        on_key(&mut app, &key("z"), &mut tx);
        assert_eq!(
            folded(&app),
            [false, false, false, true],
            "all but the lock file"
        );
        on_key(&mut app, &key("z"), &mut tx);
        assert_eq!(folded(&app), [true; 4]);
    }

    #[test]
    fn a_new_diff_is_folded_and_a_reload_of_the_same_patch_keeps_the_open_files() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open_with(&mut app, "A", "a1", TREE_PATCH);
        assert_eq!(folded(&app), [true; 4], "a new diff");
        app.diff.set_collapsed(1, false);

        app.reload();
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(Detail {
                base_commit: "base2".into(),
                ..detail("a1")
            }),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(TREE_PATCH.into()),
        });
        assert_eq!(folded(&app), [true, false, true, true], "the same patch");

        open(&mut app, "B", "b1");
        app.select("A");
        assert_eq!(folded(&app), [true; 4], "a PR opened again");
    }

    #[test]
    fn the_tree_width_is_saved_and_an_old_settings_file_reads_the_default() {
        let mut app = app(Vec::new());
        app.set_pane_width(1400.0);
        app.drag_tree(240.0, 160.0);
        app.save_tree_width();
        let mut store = Store::memory();
        store.save(app.store.settings());
        let restarted = App::new(store, None);
        assert_eq!(
            restarted.tree_width.width(),
            400.0,
            "the width survives a restart"
        );

        let old: Settings = serde_json::from_str(
            r#"{"scopes": ["org:acme"], "authors": ["ann"], "tree_closed": false, "folded": []}"#,
        )
        .unwrap();
        let mut store = Store::memory();
        store.save(&old);
        let restarted = App::new(store, None);
        assert_eq!(restarted.tree_width.width(), 240.0);
    }

    #[test]
    fn a_saved_author_group_picks_its_authors_and_survives_a_restart() {
        let mut prs = vec![pr("A", "a1"), pr("B", "b1"), pr("C", "c1")];
        prs[1].author = "dependabot".into();
        prs[2].author = "renovate".into();
        let mut app = app(prs);
        app.toggle_author("dependabot");
        app.toggle_author("renovate");
        app.group_edit = TextEdit::new(" bots ");
        app.save_group();
        assert!(!app.naming_group);
        app.clear_authors();
        assert_eq!(shown_ids(&app), ["A", "B", "C"]);

        app.toggle_author_group("bots");
        assert_eq!(shown_ids(&app), ["B", "C"]);
        app.toggle_author_group("bots");
        assert_eq!(shown_ids(&app), ["A", "B", "C"], "a second press clears");

        let mut store = Store::memory();
        store.save(app.store.settings());
        let mut restarted = App::new(store, None);
        restarted.prs = app.prs.clone();
        restarted.toggle_author_group("bots");
        assert_eq!(shown_ids(&restarted), ["B", "C"]);
    }

    #[test]
    fn the_tree_selects_the_file_the_diff_shows() {
        let mut app = app(vec![pr("A", "a1")]);
        open_with(&mut app, "A", "a1", TREE_PATCH);
        app.tree.set_expanded("src", false);
        assert!(!tree_files(&app).contains(&"src/net/retry.rs"));
        app.follow_file(app.diff.file_index("src/net/retry.rs").unwrap());
        assert_eq!(
            app.tree.selected().collect::<Vec<_>>(),
            ["src/net/retry.rs"]
        );
        assert!(tree_files(&app).contains(&"src/net/retry.rs"));
    }

    #[test]
    fn tree_search_keeps_the_files_whose_path_matches() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open_with(&mut app, "A", "a1", TREE_PATCH);
        app.set_tree_search("retry");
        assert_eq!(tree_files(&app), ["config/retry.toml", "src/net/retry.rs"]);
        app.follow_file(app.diff.file_index("src/lib.rs").unwrap());
        assert_eq!(
            tree_files(&app),
            ["config/retry.toml", "src/net/retry.rs"],
            "a scroll of the diff adds no file to the search"
        );
        app.set_tree_search("net");
        assert_eq!(
            tree_files(&app),
            ["src/net/retry.rs"],
            "a folder name matches"
        );
        open_with(&mut app, "B", "b1", TREE_PATCH);
        assert_eq!(app.tree_search.text, "");
        assert_eq!(tree_files(&app).len(), 4, "a new diff shows every file");
    }

    #[test]
    fn find_lists_the_matching_lines_of_both_sides_in_diff_order() {
        let mut app = app(vec![pr("A", "a1")]);
        open_with(&mut app, "A", "a1", TREE_PATCH);
        app.open_find();
        app.set_find("retry");
        let at = |f: usize, side: Side, line: u32| (f, LineRef::new(side, line));
        let expected = [
            at(0, Side::New, 1),
            at(0, Side::Old, 2),
            at(0, Side::New, 2),
            at(1, Side::New, 2),
        ];
        assert_eq!(app.find_matches, expected);
        let mut tx = Tx::default();
        let mut step = |app: &mut App, forward: bool| {
            app.step_find(forward, &mut tx);
            app.diff.selection().map(|r| (r.file, r.end))
        };
        for want in expected {
            assert_eq!(step(&mut app, true), Some(want));
        }
        assert_eq!(step(&mut app, true), Some(expected[0]), "wraps at the end");
        assert_eq!(
            step(&mut app, false),
            Some(expected[3]),
            "wraps at the start"
        );
        app.close_find();
        assert!(!app.find_open && app.diff.selection().is_none());
    }

    #[test]
    fn prs_group_by_repo_and_a_folded_group_hides_its_rows_until_a_search_matches() {
        let at = |id: &str, repo: &str, day: &str| {
            let mut p = pr(id, id);
            p.repo = repo.into();
            p.last_commit = format!("2026-11-{day}T10:00:00Z");
            p
        };
        let mut cli = at("cli1", "acme/cli", "13");
        cli.title = "Retry the upload".into();
        let mut app = app(vec![
            at("zeta1", "acme/zeta", "10"),
            at("web1", "acme/web", "12"),
            cli,
            at("zeta2", "acme/zeta", "14"),
        ]);
        let rows = |app: &App| -> Vec<String> {
            app.rows
                .iter()
                .map(|r| match r {
                    Row::Header {
                        repo,
                        count,
                        folded,
                    } => format!("{repo} {count}{}", if *folded { " folded" } else { "" }),
                    Row::Pr(i) => app.prs[*i].id.clone(),
                })
                .collect()
        };
        assert_eq!(
            rows(&app),
            [
                "acme/zeta 2",
                "zeta2",
                "zeta1",
                "acme/cli 1",
                "cli1",
                "acme/web 1",
                "web1"
            ]
        );
        app.toggle_group("acme/cli");
        assert_eq!(
            rows(&app),
            [
                "acme/zeta 2",
                "zeta2",
                "zeta1",
                "acme/cli 1 folded",
                "acme/web 1",
                "web1"
            ]
        );
        assert!(app.store.settings().folded.contains("acme/cli"));
        app.select("zeta1");
        on_key(&mut app, &key("j"), &mut Tx::default());
        assert_eq!(
            app.selected.as_deref(),
            Some("web1"),
            "j skips the folded group"
        );
        app.select("cli1");
        on_key(&mut app, &key("j"), &mut Tx::default());
        assert_eq!(
            app.selected.as_deref(),
            Some("web1"),
            "j from a folded group"
        );
        app.select("cli1");
        on_key(&mut app, &key("k"), &mut Tx::default());
        assert_eq!(
            app.selected.as_deref(),
            Some("zeta1"),
            "k from a folded group"
        );
        app.select("web1");
        app.set_pr_search("upload");
        assert_eq!(
            rows(&app),
            ["acme/cli 1", "cli1", "acme/web 1", "web1"],
            "the search shows the match, and the open PR stays listed"
        );
        app.toggle_group("acme/web");
        assert!(app.settings.folded.contains("acme/cli"));
        assert!(
            !app.settings.folded.contains("acme/web"),
            "a header click during a search keeps the saved folds"
        );
        app.set_pr_search("");
        assert!(rows(&app).contains(&"acme/cli 1 folded".to_owned()));

        app.select("zeta2");
        app.set_pr_search("upload");
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Default::default(),
        };
        pr_search_key(&mut app, &enter, &mut Tx::default());
        assert_eq!(
            app.selected.as_deref(),
            Some("cli1"),
            "Enter opens the match below the open PR"
        );

        app.pr_search.focused = true;
        let ctrl_shift_b = KeyEvent {
            key: Key::Command('b'),
            mods: rune::Mods {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        };
        on_key(&mut app, &ctrl_shift_b, &mut Tx::default());
        assert!(!app.sidebar.is_open(), "Ctrl+Shift+B closes the drawer");
        assert!(!app.pr_search.focused, "the closed drawer keeps no cursor");
    }

    #[test]
    fn opening_another_comment_box_keeps_the_typed_text() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 2)));
        app.comment_edit = TextEdit::new("typed");
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 3)));
        let kept: Vec<&str> = app
            .current_drafts()
            .iter()
            .map(|d| d.body.as_str())
            .collect();
        assert_eq!(kept, ["typed"]);
    }

    #[test]
    fn a_failed_reload_before_the_diff_arrives_asks_for_the_diff_again() {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a1")),
        });
        app.refresh();
        sent(&app);
        app.apply(Msg::Detail {
            load: app.load,
            answer: Err("No network".into()),
        });
        let urls: Vec<String> = sent(&app).into_iter().map(|r| r.url).collect();
        assert!(
            urls.iter().any(|u| u.ends_with("compare/base...a1")),
            "{urls:?}"
        );
    }

    #[test]
    fn an_old_viewer_answer_does_not_replace_the_new_login() {
        let mut app = App::new(Store::memory(), None);
        let viewer = |asked, login: &str| Msg::Viewer {
            asked,
            answer: Ok(Viewer {
                login: login.into(),
                suggestions: Vec::new(),
            }),
        };
        app.start();
        let old = app.viewer_asked;
        app.refresh();
        app.apply(viewer(app.viewer_asked, "new"));
        app.apply(viewer(old, "old"));
        assert_eq!(app.me, "new");
    }

    #[test]
    fn a_failed_reload_keeps_the_pr_on_screen() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.refresh();
        app.apply(Msg::Detail {
            load: app.load,
            answer: Err("No network".into()),
        });
        assert!(app.current_detail().is_some() && app.diff_ready);
        assert!(app.toast.as_ref().is_some_and(|t| t.failed));
    }

    #[test]
    fn no_line_comment_on_my_own_pr() {
        let mut mine = pr("A", "a1");
        mine.author = "me".into();
        let mut app = app(vec![mine]);
        open(&mut app, "A", "a1");
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 2)));
        assert!(app.composing.is_none());
    }

    #[test]
    fn a_cross_side_selection_becomes_a_one_line_comment() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        let lines = LineRange {
            file: 0,
            start: LineRef::new(Side::Old, 2),
            end: LineRef::new(Side::New, 3),
        };
        comment(&mut app, lines, "here");
        let d = &app.current_drafts()[0];
        assert_eq!((d.start, d.end, d.left), (3, 3, false));
    }

    /// A thread of `id` on `line` of `path`, new side unless `left`.
    fn thread(id: &str, path: &str, line: u32, left: bool, resolved: bool) -> github::Thread {
        github::Thread {
            id: id.into(),
            path: path.into(),
            line: Some(line),
            original_line: Some(line),
            outdated: false,
            hunk: String::new(),
            left,
            resolved,
            can_resolve: !resolved,
            can_unresolve: resolved,
            can_reply: true,
            comments: Vec::new(),
        }
    }

    /// An outdated thread of `id` on `path`, made on the last line of `hunk`, new side.
    fn outdated(id: &str, path: &str, hunk: &str) -> github::Thread {
        github::Thread {
            line: None,
            outdated: true,
            hunk: hunk.into(),
            ..thread(id, path, 9, false, false)
        }
    }

    /// `A` open on TREE_PATCH, whose files are, in diff order, src/net/retry.rs, src/lib.rs,
    /// config/retry.toml and Cargo.lock. The threads are listed out of diff order.
    fn with_threads() -> App {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(Detail {
                threads: vec![
                    thread("lib-new", "src/lib.rs", 2, false, false),
                    thread("retry-done", "src/net/retry.rs", 2, false, true),
                    thread("toml", "config/retry.toml", 1, false, false),
                    thread("lib-old", "src/lib.rs", 2, true, false),
                    thread("gone", "src/gone.rs", 1, false, false),
                    thread("retry", "src/net/retry.rs", 1, false, false),
                    thread("toml-2", "config/retry.toml", 1, false, false),
                ],
                ..detail("a1")
            }),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(TREE_PATCH.into()),
        });
        app
    }

    fn thread_of<'a>(app: &'a App, id: &str) -> &'a github::Thread {
        let d = app.current_detail().unwrap();
        d.threads.iter().find(|t| t.id == id).unwrap()
    }

    fn resolved(thread: &str, answer: Result<github::ThreadState, String>) -> Msg {
        Msg::Resolved {
            id: "A".into(),
            thread: thread.into(),
            resolve: true,
            answer,
        }
    }

    const DONE: github::ThreadState = github::ThreadState {
        resolved: true,
        can_resolve: false,
        can_unresolve: true,
    };

    #[test]
    fn the_thread_keys_step_through_the_open_threads_in_file_order_and_wrap() {
        let mut app = with_threads();
        assert_eq!(
            app.open_threads().len(),
            5,
            "not the resolved one, not the file off the diff"
        );
        let mut tx = Tx::default();
        let mut press = |app: &mut App, shift: bool| {
            let ev = KeyEvent {
                key: Key::Text("u".into()),
                mods: rune::Mods {
                    shift,
                    ..Default::default()
                },
            };
            assert!(on_key(app, &ev, &mut tx));
            app.thread_at.clone().unwrap()
        };
        let order: Vec<String> = (0..6).map(|_| press(&mut app, false)).collect();
        assert_eq!(
            order,
            ["retry", "lib-old", "lib-new", "toml", "toml-2", "retry"],
            "file order, line order, and both threads of one line"
        );
        assert!(!app.diff.collapsed(0), "a jump opens the folded file");
        assert_eq!(
            app.diff.selection(),
            Some(LineRange::line(0, LineRef::new(Side::New, 1)))
        );
        assert_eq!(
            press(&mut app, true),
            "toml-2",
            "Shift+U goes back around the start"
        );
        assert_eq!(press(&mut app, true), "toml");

        app.find_open = true;
        app.find_edit.focused = true;
        on_key(&mut app, &key("u"), &mut Tx::default());
        assert_eq!(
            app.thread_at.as_deref(),
            Some("toml"),
            "not while typing in find"
        );
    }

    #[test]
    fn a_resolve_answer_updates_the_thread_and_a_failure_leaves_it_open() {
        let mut app = with_threads();
        sent(&app);
        app.resolve("retry", true);
        app.resolve("retry", true);
        let reqs = sent(&app);
        assert_eq!(
            reqs.len(),
            1,
            "a second click while the first is on the way sends nothing"
        );
        assert!(
            reqs[0]
                .body
                .as_ref()
                .unwrap()
                .contains("\"threadId\":\"retry\"")
        );

        app.apply(resolved("retry", Ok(DONE)));
        let t = thread_of(&app, "retry");
        assert_eq!(
            (t.resolved, t.can_resolve, t.can_unresolve),
            (true, false, true)
        );
        assert!(app.toast.is_none(), "a success shows no toast");
        assert!(sent(&app).is_empty(), "no reload");
        assert_eq!(app.open_threads().len(), 4);
        app.thread_at = None;
        app.step_thread(true, &mut Tx::default());
        assert_eq!(
            app.thread_at.as_deref(),
            Some("lib-old"),
            "a jump skips the resolved thread"
        );

        app.resolve("toml", true);
        app.apply(resolved("toml", Err("Resource not accessible".into())));
        assert!(!thread_of(&app, "toml").resolved);
        assert!(!app.resolving.contains("toml"), "the button turns on again");
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.failed);
        assert!(
            toast.text.contains("config/retry.toml")
                && toast.text.contains("acme/api#1")
                && toast.text.contains("Resource not accessible"),
            "{}",
            toast.text
        );

        // The cache keeps the resolved state for the next open.
        let mut app2 = self::app(vec![pr("A", "a1"), pr("B", "b1")]);
        std::mem::swap(&mut app2.cache, &mut app.cache);
        app2.select("A");
        assert!(thread_of(&app2, "retry").resolved, "the cached copy agrees");

        // An answer for a PR no longer open changes nothing on screen and drops its cache.
        app2.resolve("lib-new", true);
        app2.select("B");
        app2.apply(resolved("lib-new", Ok(DONE)));
        assert!(app2.cache.iter().all(|c| c.id != "A"));
        assert!(app2.toast.is_none());
    }

    #[test]
    fn a_detail_answer_asked_before_a_resolve_does_not_reopen_the_thread() {
        let mut app = with_threads();
        app.refresh();
        let older = app.load;
        app.resolve("retry", true);
        sent(&app);
        app.apply(resolved("retry", Ok(DONE)));
        assert!(
            detail_sent(&app, "A"),
            "a new reload while the older one is on the way"
        );
        app.apply(Msg::Detail {
            load: older,
            answer: Ok(Detail {
                threads: vec![thread("retry", "src/net/retry.rs", 1, false, false)],
                ..detail("a1")
            }),
        });
        assert!(thread_of(&app, "retry").resolved);
        assert_eq!(app.open_threads().len(), 4);
    }

    /// Answers a review of `id` as GitHub would: a success unless `error`.
    fn reviewed(id: &str, state: ReviewState, error: Option<&str>) -> Msg {
        Msg::Submitted {
            id: id.into(),
            head: "h".into(),
            sent: Vec::new(),
            answer: match error {
                Some(e) => Err(e.into()),
                None => Ok(MyReview {
                    state,
                    commit: "h".into(),
                }),
            },
        }
    }

    #[test]
    fn an_accepted_review_opens_the_row_below_and_others_stay() {
        let mut other = pr("C", "c1");
        other.repo = "acme/web".into();
        let mut b = pr("B", "b1");
        b.number = 2;
        let mut app = app(vec![pr("A", "a1"), b, other, pr("D", "d1")]);
        let rows = |app: &App| -> Vec<String> {
            app.rows
                .iter()
                .filter_map(|r| match r {
                    Row::Pr(i) => Some(app.prs[*i].id.clone()),
                    Row::Header { .. } => None,
                })
                .collect()
        };
        assert_eq!(rows(&app), ["A", "B", "D", "C"], "grouped by repository");

        open(&mut app, "A", "a1");
        app.apply(reviewed("A", ReviewState::Approved, Some("Timeout")));
        assert_eq!(app.selected.as_deref(), Some("A"), "a failed review stays");

        app.apply(reviewed("A", ReviewState::Approved, None));
        assert_eq!(app.selected.as_deref(), Some("B"));
        assert_eq!(app.reveal_row.take().as_deref(), Some("B"));
        let toast = &app.toast.as_ref().unwrap().text;
        assert!(toast.contains("acme/api#1, now on acme/api#2"), "{toast}");

        app.toggle_group("acme/api");
        app.apply(reviewed("B", ReviewState::ChangesRequested, None));
        assert_eq!(app.selected.as_deref(), Some("C"), "past the folded group");

        sent(&app);
        app.apply(reviewed("C", ReviewState::Commented, None));
        assert_eq!(app.selected.as_deref(), Some("C"), "the last row stays");
        assert!(detail_sent(&app, "C"), "and reloads");

        app.toggle_group("acme/api");
        open(&mut app, "A", "a1");
        app.start_comment(LineRange::line(0, LineRef::new(Side::New, 2)));
        sent(&app);
        app.apply(reviewed("A", ReviewState::Approved, None));
        assert_eq!(
            app.selected.as_deref(),
            Some("A"),
            "an open comment box stays"
        );
        assert!(detail_sent(&app, "A"), "and reloads");
        app.cancel_comment();

        app.apply(reviewed("B", ReviewState::Approved, None));
        assert_eq!(
            app.selected.as_deref(),
            Some("A"),
            "a review of another PR moves nothing"
        );
    }

    #[test]
    fn caps_lock_a_does_not_approve() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        sent(&app);
        let mut tx = Tx::default();
        let caps = KeyEvent {
            key: Key::Text("A".into()),
            mods: Default::default(),
        };
        on_key(&mut app, &caps, &mut tx);
        assert!(sent(&app).is_empty());
        let shift_with_caps = KeyEvent {
            key: Key::Text("a".into()),
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        };
        on_key(&mut app, &shift_with_caps, &mut tx);
        assert_eq!(sent(&app).len(), 1);
    }

    #[test]
    fn refresh_reloads_the_open_pr() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        sent(&app);
        app.refresh();
        assert!(detail_sent(&app, "A"));
        app.select("B");
        app.select("A");
        assert!(app.detail.is_none(), "the cache is cleared");
    }

    /// The ids of the PRs whose detail the requests ask for, in the order sent.
    fn detail_ids(reqs: &[Request]) -> Vec<&str> {
        reqs.iter()
            .filter_map(|r| {
                let body = r.body.as_deref()?;
                let rest = &body[body.find("\"id\":\"")? + 6..];
                Some(&rest[..rest.find('"')?])
            })
            .collect()
    }

    /// Answers the load ahead of `id` at `head`, from its detail to its diff.
    fn answer_ahead(app: &mut App, id: &str, head: &str) {
        let generation = app.generation;
        app.apply(Msg::Ahead {
            generation,
            id: id.into(),
            answer: Ok(detail(head)),
        });
        app.apply(Msg::AheadDiff {
            generation,
            id: id.into(),
            detail: detail(head),
            answer: Ok(PATCH.into()),
        });
    }

    #[test]
    fn opening_a_pr_loads_the_next_three_ahead() {
        let ids = ["A", "B", "C", "D", "E"];
        let mut app = app(ids.iter().map(|id| pr(id, id)).collect());
        open(&mut app, "A", "A");
        assert_eq!(detail_ids(&sent(&app)), ["A", "B", "C", "D"]);
        answer_ahead(&mut app, "B", "B");
        sent(&app);
        // C and D are on the way, so only E is new.
        app.select("B");
        assert_eq!(detail_ids(&sent(&app)), ["B", "E"]);
    }

    #[test]
    fn a_pr_loaded_ahead_opens_at_once_and_checks_for_changes() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        answer_ahead(&mut app, "B", "b1");
        sent(&app);
        app.select("B");
        assert_eq!(app.current_detail().unwrap().head_commit, "b1");
        assert!(app.diff_ready, "the diff shows before any answer");
        assert_eq!(
            detail_ids(&sent(&app)),
            ["B"],
            "the open checks for changes"
        );
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("b1")),
        });
        assert!(sent(&app).is_empty(), "an unchanged PR loads no diff");
    }

    #[test]
    fn a_load_ahead_from_before_a_refresh_is_dropped() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        let before = app.generation;
        app.refresh();
        app.apply(Msg::AheadDiff {
            generation: before,
            id: "B".into(),
            detail: detail("b1"),
            answer: Ok(PATCH.into()),
        });
        assert!(app.cache.iter().all(|c| c.id != "B"));
    }

    #[test]
    fn shift_r_reloads_only_the_open_pr() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        sent(&app);
        let shift_r = KeyEvent {
            key: Key::Text("R".into()),
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        };
        on_key(&mut app, &shift_r, &mut Tx::default());
        let asked = sent(&app);
        assert_eq!(asked.len(), 1, "no list request");
        assert_eq!(detail_ids(&asked), ["A"]);
    }

    #[test]
    fn a_sent_review_reloads_the_open_pr() {
        // A is on the last row, so the review keeps it open.
        let mut app = app(vec![pr("B", "b1"), pr("A", "a1")]);
        open(&mut app, "A", "a1");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "rename",
        );
        let drafts = app.current_drafts().to_vec();
        app.submit(ReviewState::Approved);
        sent(&app);
        app.apply(approved("A", "a1", drafts));
        assert!(detail_sent(&app, "A"));
        app.select("B");
        app.select("A");
        assert!(
            app.detail.is_none(),
            "the cached detail without the review is dropped"
        );
    }

    #[test]
    fn approve_stays_on_after_a_reply_on_a_new_head() {
        let mut listed = pr("A", "a2");
        listed.my_reviews = vec![
            MyReview {
                state: ReviewState::Approved,
                commit: "a1".into(),
            },
            MyReview {
                state: ReviewState::Commented,
                commit: "a2".into(),
            },
        ];
        let mut app = app(vec![listed]);
        open(&mut app, "A", "a2");
        assert!(app.can_approve());
    }

    #[test]
    fn no_review_goes_out_before_the_diff_loads() {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a1")),
        });
        assert!(!app.can_review());
        app.apply(Msg::Diff {
            load: app.load,
            answer: Err("No network".into()),
        });
        sent(&app);
        app.submit(ReviewState::Approved);
        assert!(sent(&app).is_empty());
    }

    fn urls(requests: &[Request]) -> Vec<&str> {
        requests.iter().map(|r| r.url.as_str()).collect()
    }

    fn runs_answer(runs: &[(u64, &str, Option<&str>)]) -> Msg {
        let runs: Vec<serde_json::Value> = runs
            .iter()
            .map(|&(id, status, conclusion)| {
                serde_json::json!({ "id": id, "status": status, "conclusion": conclusion })
            })
            .collect();
        let answer = serde_json::json!({ "total_count": runs.len(), "workflow_runs": runs });
        Msg::Runs(github::parse_runs(&answer.to_string()))
    }

    #[test]
    fn rerun_requests_name_only_the_runs_that_match() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a2");
        let runs = [
            (1, "completed", Some("failure")),
            (2, "completed", Some("success")),
            (3, "in_progress", None),
            (4, "completed", Some("timed_out")),
            (5, "completed", Some("cancelled")),
            (6, "queued", None),
        ];
        for (which, endpoint, picked) in [
            (Rerun::Failed, "rerun-failed-jobs", vec![1, 4, 5]),
            (Rerun::All, "rerun", vec![1, 2, 4, 5]),
        ] {
            sent(&app);
            app.rerun(which);
            let asked = sent(&app);
            assert_eq!(
                urls(&asked),
                ["https://api.github.com/repos/acme/api/actions/runs?head_sha=a2&per_page=100"]
            );
            assert!(asked[0].body.is_none());
            app.apply(runs_answer(&runs));
            let reruns = sent(&app);
            let expected: Vec<String> = picked
                .iter()
                .map(|n| {
                    format!("https://api.github.com/repos/acme/api/actions/runs/{n}/{endpoint}")
                })
                .collect();
            assert_eq!(urls(&reruns), expected);
            assert!(reruns.iter().all(|r| r.body.as_deref() == Some("{}")));
            for _ in &picked {
                app.apply(Msg::Reran(Ok(())));
            }
            let toast = app.toast.as_ref().unwrap();
            assert_eq!(
                (toast.text.as_str(), toast.failed),
                (
                    format!("Re-ran {} runs of acme/api#1", picked.len()).as_str(),
                    false
                )
            );
        }
    }

    #[test]
    fn update_branch_names_the_head_the_diff_shows_and_merges() {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(Detail {
                can_update_branch: true,
                ..detail("a2")
            }),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(PATCH.into()),
        });
        sent(&app);
        app.update_branch();
        let asked = sent(&app);
        assert_eq!(urls(&asked), ["https://api.github.com/graphql"]);
        let body: serde_json::Value =
            serde_json::from_str(asked[0].body.as_ref().unwrap()).unwrap();
        assert!(
            body["query"]
                .as_str()
                .unwrap()
                .contains("updatePullRequestBranch")
        );
        assert_eq!(
            body["variables"]["input"],
            serde_json::json!({ "pullRequestId": "A", "expectedHeadOid": "a2", "updateMethod": "MERGE" })
        );
    }

    #[test]
    fn a_runs_answer_for_another_pr_or_head_sends_no_rerun() {
        let failed = [(1, "completed", Some("failure"))];
        let no_rerun = |app: &App| {
            assert!(!urls(&sent(app)).iter().any(|u| u.contains("/rerun")));
            let toast = app.toast.as_ref().unwrap();
            assert!(
                toast.failed && toast.text.contains("acme/api#1"),
                "{}",
                toast.text
            );
        };
        let mut other = pr("B", "b1");
        other.number = 2;
        let mut app = app(vec![pr("A", "a1"), other]);
        open(&mut app, "A", "a1");
        app.rerun(Rerun::Failed);
        open(&mut app, "B", "b1");
        sent(&app);
        app.toast = None;
        app.apply(runs_answer(&failed));
        no_rerun(&app);

        open(&mut app, "A", "a1");
        app.rerun(Rerun::Failed);
        app.refresh();
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(detail("a2")),
        });
        sent(&app);
        app.toast = None;
        app.apply(runs_answer(&failed));
        no_rerun(&app);
    }

    #[test]
    fn each_list_page_shows_as_it_arrives_and_a_failed_page_keeps_the_rest() {
        let page = |app: &App, page, prs, next| Msg::Prs {
            generation: app.generation,
            page,
            answer: Ok((prs, next)),
        };
        let ids = |app: &App| -> Vec<String> { app.prs.iter().map(|p| p.id.clone()).collect() };

        let mut first = app(Vec::new());
        first.refresh();
        first.apply(page(
            &first,
            0,
            vec![pr("A", "a1")],
            Next::Page("c1".into()),
        ));
        assert_eq!(ids(&first), ["A"]);
        first.apply(page(
            &first,
            1,
            vec![pr("B", "b1")],
            Next::Page("c2".into()),
        ));
        assert_eq!(ids(&first), ["A", "B"]);
        first.apply(Msg::Prs {
            generation: first.generation,
            page: 2,
            answer: Err("Timeout".into()),
        });
        assert_eq!(ids(&first), ["A", "B"]);
        assert!(!first.loading);
        assert!(
            first
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not complete"))
        );

        // A refresh keeps the old rows until the new list is complete.
        let mut refreshed = app(vec![pr("OPEN", "o1"), pr("GONE", "g1"), pr("OLD", "x1")]);
        open(&mut refreshed, "OPEN", "o1");
        refreshed.refresh();
        let pushed = vec![pr("OLD", "x2"), pr("A", "a1")];
        refreshed.apply(page(&refreshed, 0, pushed, Next::Page("c1".into())));
        assert_eq!(ids(&refreshed), ["OPEN", "GONE", "OLD", "A"]);
        assert_eq!(refreshed.pr("OLD").unwrap().head, "x2");
        refreshed.apply(page(&refreshed, 1, vec![pr("OPEN", "o1")], Next::Done));
        assert_eq!(ids(&refreshed), ["OLD", "A", "OPEN"]);
        assert!(refreshed.current().is_some(), "the open PR stays open");
    }

    fn commented(id: &str, answer: Result<Entry, String>) -> Msg {
        Msg::Commented {
            id: id.into(),
            answer,
        }
    }

    fn note(author: &str, at: &str, body: &str) -> Entry {
        Entry {
            author: author.into(),
            at: at.into(),
            body: body.into(),
            ..Entry::default()
        }
    }

    /// PR A open, with a comment, a review and a line comment of the viewer, each "Old text".
    fn with_mine() -> App {
        let mut app = app(vec![pr("A", "a1")]);
        app.select("A");
        let mine = |id: &str, kind| Entry {
            author: "me".into(),
            body: "Old text".into(),
            id: id.into(),
            kind,
            can_edit: true,
            can_delete: kind != EntryKind::Review,
            ..Entry::default()
        };
        let mut retry = thread("retry", "src/net/retry.rs", 1, false, false);
        retry.comments.push(mine("RC_9", EntryKind::LineComment));
        app.apply(Msg::Detail {
            load: app.load,
            answer: Ok(Detail {
                entries: vec![
                    mine("IC_9", EntryKind::Comment),
                    mine("PRR_9", EntryKind::Review),
                ],
                threads: vec![retry],
                ..detail("a1")
            }),
        });
        app.apply(Msg::Diff {
            load: app.load,
            answer: Ok(TREE_PATCH.into()),
        });
        sent(&app);
        app
    }

    /// The text of the entry `id` in the open detail, and in the cache.
    fn texts_of(app: &App, id: &str) -> (Option<String>, Option<String>) {
        let find = |d: &Detail| {
            d.entries
                .iter()
                .chain(d.threads.iter().flat_map(|t| &t.comments))
                .find(|e| e.id == id)
                .map(|e| e.body.clone())
        };
        let cached = app.cache.iter().find(|c| c.id == "A").unwrap();
        (find(app.current_detail().unwrap()), find(&cached.detail))
    }

    #[test]
    fn an_edit_saves_the_text_and_a_failure_keeps_the_box() {
        let mut app = with_mine();
        app.start_edit("RC_9");
        assert_eq!(app.editing.as_ref().unwrap().edit.text, "Old text");
        app.editing.as_mut().unwrap().edit = TextEdit::new("  New text ");
        app.save_edit();
        app.save_edit();
        let reqs = sent(&app);
        assert_eq!(reqs.len(), 1, "a second save while the first is on the way");
        let body = reqs[0].body.as_deref().unwrap();
        assert!(
            body.contains("updatePullRequestReviewComment(")
                && body.contains("\"pullRequestReviewCommentId\":\"RC_9\"")
                && body.contains("\"body\":\"New text\""),
            "{body}"
        );
        let edited = |answer| Msg::Edited {
            id: "A".into(),
            entry: "RC_9".into(),
            body: "New text".into(),
            answer,
        };
        app.apply(edited(Err("Rate limited".into())));
        assert!(app.editing.is_some(), "a failure keeps the box");
        assert!(app.toast.as_ref().unwrap().failed);
        assert_eq!(texts_of(&app, "RC_9").0.as_deref(), Some("Old text"));

        app.save_edit();
        assert_eq!(sent(&app).len(), 1, "the box saves again after a failure");
        app.apply(edited(Ok(())));
        assert!(app.editing.is_none());
        let new = Some("New text".to_owned());
        assert_eq!(texts_of(&app, "RC_9"), (new.clone(), new));
    }

    #[test]
    fn a_delete_asks_first_then_removes_the_comment() {
        let mut app = with_mine();
        app.ask_delete("IC_9");
        assert!(sent(&app).is_empty(), "the first click only asks");
        app.confirm_delete();
        let reqs = sent(&app);
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_deref().unwrap();
        assert!(
            body.contains("deleteIssueComment(") && body.contains("\"id\":\"IC_9\""),
            "{body}"
        );
        let deleted = |entry: &str| Msg::Deleted {
            id: "A".into(),
            entry: entry.into(),
            answer: Ok(()),
        };
        app.apply(deleted("IC_9"));
        assert_eq!(texts_of(&app, "IC_9"), (None, None));

        app.ask_delete("RC_9");
        app.confirm_delete();
        sent(&app);
        app.apply(deleted("RC_9"));
        let d = app.current_detail().unwrap();
        assert!(d.threads.is_empty(), "a thread with no comment left goes");
    }

    #[test]
    fn a_reply_adds_the_comment_and_a_failure_keeps_the_text() {
        let mut app = with_threads();
        sent(&app);
        app.start_reply("retry");
        app.reply_edit = TextEdit::new("   ");
        app.send_reply();
        assert!(sent(&app).is_empty(), "empty text sends nothing");

        app.reply_edit = TextEdit::new("  Fixed in the next push. ");
        app.send_reply();
        app.send_reply();
        let reqs = sent(&app);
        assert_eq!(reqs.len(), 1, "a second send while the first is on the way");
        let body = reqs[0].body.as_deref().unwrap();
        assert!(
            body.contains("addPullRequestReviewThreadReply(")
                && body.contains("\"pullRequestReviewThreadId\":\"retry\"")
                && body.contains("\"body\":\"Fixed in the next push.\""),
            "{body}"
        );

        let reply = |answer| Msg::Replied {
            id: "A".into(),
            thread: "retry".into(),
            answer,
        };
        app.apply(reply(Err("Rate limited".into())));
        assert_eq!(
            app.reply_edit.text, "  Fixed in the next push. ",
            "a failure keeps the text"
        );
        assert!(app.toast.as_ref().unwrap().failed);

        app.send_reply();
        assert_eq!(sent(&app).len(), 1, "the box sends again after a failure");
        let new = note("me", "2026-11-15T10:00:00Z", "Fixed in the next push.");
        app.apply(reply(Ok(new.clone())));
        assert_eq!(thread_of(&app, "retry").comments.last(), Some(&new));
        let cached = app.cache.iter().find(|c| c.id == "A").unwrap();
        let cached = cached
            .detail
            .threads
            .iter()
            .find(|t| t.id == "retry")
            .unwrap();
        assert_eq!(cached.comments.last(), Some(&new), "the cache agrees");
        assert_eq!(
            (app.reply_to.as_deref(), app.reply_edit.text.as_str()),
            (None, "")
        );
        assert!(sent(&app).is_empty(), "no reload");
    }

    #[test]
    fn a_comment_answer_adds_the_comment_and_a_failure_keeps_the_text() {
        let mut app = app(vec![pr("A", "a1"), pr("B", "b1")]);
        open(&mut app, "A", "a1");
        comment(
            &mut app,
            LineRange::line(0, LineRef::new(Side::New, 2)),
            "rename",
        );
        sent(&app);
        app.pr_comment = TextEdit::new("   ");
        app.send_comment();
        assert!(sent(&app).is_empty(), "empty text sends nothing");

        app.pr_comment = TextEdit::new("  Looks good. ");
        app.send_comment();
        app.send_comment();
        let reqs = sent(&app);
        assert_eq!(reqs.len(), 1, "a second send while the first is on the way");
        let body = reqs[0].body.as_deref().unwrap();
        assert!(
            body.contains("addComment(")
                && body.contains("\"subjectId\":\"A\"")
                && body.contains("\"body\":\"Looks good.\""),
            "{body}"
        );

        let new = note("me", "2026-11-15T10:00:00Z", "Looks good.");
        app.apply(commented("A", Ok(new.clone())));
        assert_eq!(app.current_detail().unwrap().entries.last(), Some(&new));
        let cached = app.cache.iter().find(|c| c.id == "A").unwrap();
        assert_eq!(cached.detail.entries.last(), Some(&new), "the cache agrees");
        assert_eq!(app.pr_comment.text, "");
        let toast = app.toast.as_ref().unwrap();
        assert_eq!(
            (toast.text.as_str(), toast.failed),
            ("Commented on acme/api#1", false)
        );
        assert_eq!(
            app.selected.as_deref(),
            Some("A"),
            "the open PR does not move"
        );
        assert_eq!(app.current_drafts().len(), 1, "the line comment stays");
        assert!(app.pr("A").unwrap().my_reviews.is_empty());
        assert!(sent(&app).is_empty(), "no reload");

        app.pr_comment = TextEdit::new("Second");
        app.send_comment();
        app.apply(commented("A", Err("Rate limited".into())));
        assert_eq!(app.pr_comment.text, "Second", "a failure keeps the text");
        assert!(app.commenting.is_empty(), "the box turns on again");
        let toast = app.toast.as_ref().unwrap();
        assert!(
            toast.failed
                && toast.text.contains("acme/api#1")
                && toast.text.contains("Rate limited"),
            "{}",
            toast.text
        );

        // An answer for a PR no longer open changes nothing on screen and drops its cache.
        app.send_comment();
        app.select("B");
        app.toast = None;
        app.apply(commented("A", Ok(new.clone())));
        assert!(app.cache.iter().all(|c| c.id != "A"));
        assert!(app.toast.is_none() && app.detail.is_none());

        // A detail answer asked before the comment does not drop it.
        let mut app = self::app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        app.refresh();
        let older = app.load;
        app.pr_comment = TextEdit::new("Late");
        app.send_comment();
        sent(&app);
        app.apply(commented("A", Ok(new.clone())));
        assert!(
            detail_sent(&app, "A"),
            "a new reload while the older one is on the way"
        );
        app.apply(Msg::Detail {
            load: older,
            answer: Ok(detail("a1")),
        });
        assert_eq!(app.current_detail().unwrap().entries.last(), Some(&new));

        // A detail answer that already holds the comment does not get it twice.
        app.pr_comment = TextEdit::new("Late");
        app.send_comment();
        app.apply(commented("A", Ok(new.clone())));
        let entries = &app.current_detail().unwrap().entries;
        assert_eq!(entries.iter().filter(|e| **e == new).count(), 1);
    }

    #[test]
    fn the_thread_list_groups_the_items_and_a_click_goes_to_each() {
        let mut app = with_threads();
        if let Some(Ok(d)) = app.detail.as_mut() {
            d.threads
                .push(thread("lib-done", "src/lib.rs", 1, false, true));
            // Its code is now on line 2; the line of the other is gone.
            let hunk = "@@ -1,2 +1,2 @@\n fn retry_send() {\n+    Retry::new(5)";
            d.threads.push(outdated("moved", "src/net/retry.rs", hunk));
            let hunk = "@@ -1,2 +1,2 @@\n mod net;\n+mod gone;";
            d.threads.push(outdated("stale", "src/lib.rs", hunk));
            d.entries = vec![
                note("cyd", "2026-11-14T08:00:00Z", "First"),
                note("dee", "2026-11-14T11:00:00Z", "Second"),
            ];
        }
        let names = |app: &App| -> Vec<(&str, Vec<String>)> {
            let d = app.current_detail().unwrap();
            app.thread_groups()
                .into_iter()
                .map(|(group, items)| {
                    let items = items
                        .iter()
                        .map(|item| match *item {
                            Item::Thread(n) => d.threads[n].id.clone(),
                            Item::Entry(n) => d.entries[n].body.clone(),
                        })
                        .collect();
                    (group, items)
                })
                .collect()
        };
        assert_eq!(
            names(&app),
            [
                (
                    "Open",
                    vec!["retry", "lib-old", "lib-new", "toml", "toml-2", "gone"]
                ),
                ("Outdated", vec!["moved", "stale"]),
                ("Resolved", vec!["retry-done", "lib-done"]),
                ("Comments", vec!["First", "Second"]),
            ]
            .map(|(g, items)| (g, items.into_iter().map(String::from).collect()))
        );

        let item = |app: &App, id: &str| {
            let d = app.current_detail().unwrap();
            Item::Thread(d.threads.iter().position(|t| t.id == id).unwrap())
        };
        let mut tx = Tx::default();
        let lib_old = item(&app, "lib-old");
        app.go_to(lib_old, &mut tx);
        assert_eq!(
            app.diff.selection(),
            Some(LineRange::line(1, LineRef::new(Side::Old, 2)))
        );
        on_key(&mut app, &key("u"), &mut tx);
        assert_eq!(
            app.thread_at.as_deref(),
            Some("lib-new"),
            "u goes on from it"
        );

        let done = item(&app, "lib-done");
        app.go_to(done, &mut tx);
        assert!(app.opened_threads.contains("lib-done"), "the fold opens");
        assert_eq!(
            app.diff.selection(),
            Some(LineRange::line(1, LineRef::new(Side::New, 1)))
        );
        on_key(&mut app, &key("u"), &mut tx);
        assert_eq!(
            app.thread_at.as_deref(),
            Some("stale"),
            "u goes on to the outdated thread under the same line"
        );

        app.go_to(item(&app, "moved"), &mut tx);
        assert_eq!(
            app.diff.selection(),
            Some(LineRange::line(0, LineRef::new(Side::New, 2))),
            "an outdated thread goes to the line that holds its code now"
        );
        app.go_to(item(&app, "stale"), &mut tx);
        assert_eq!(
            app.diff.selection(),
            Some(LineRange::line(1, LineRef::new(Side::New, 1))),
            "one whose line is gone goes to the first line of its file"
        );
        assert!(!app.unplaced_open);
        app.go_to(item(&app, "gone"), &mut tx);
        assert!(
            app.unplaced_open,
            "one on a file the diff lacks opens its panel"
        );

        app.conversation_open = false;
        app.go_to(Item::Entry(1), &mut tx);
        assert!(app.conversation_open && app.show_all);

        if let Some(Ok(d)) = app.detail.as_mut() {
            d.threads.clear();
            d.entries.clear();
        }
        assert!(app.thread_groups().is_empty(), "no empty group");
    }

    #[test]
    fn ctrl_c_copies_the_pr_number_unless_lines_are_selected_and_ctrl_shift_c_the_link() {
        let mut app = app(vec![pr("A", "a1")]);
        open(&mut app, "A", "a1");
        let copy = |app: &mut App, shift: bool| {
            app.toast = None;
            let ev = KeyEvent {
                key: Key::Command('c'),
                mods: rune::Mods {
                    ctrl: true,
                    shift,
                    ..Default::default()
                },
            };
            assert!(on_key(app, &ev, &mut Tx::default()));
            app.toast.as_ref().map(|t| t.text.clone())
        };
        assert_eq!(copy(&mut app, false).as_deref(), Some("Copied acme/api#1"));
        assert_eq!(
            copy(&mut app, true).as_deref(),
            Some("Copied the link of acme/api#1")
        );
        let line = LineRange::line(0, LineRef::new(Side::New, 2));
        app.diff.set_collapsed(0, false);
        app.diff.set_selection(Some(line));
        assert_eq!(copy(&mut app, false), None, "the selected lines are copied");
        assert_eq!(
            copy(&mut app, true).as_deref(),
            Some("Copied the link of acme/api#1")
        );
    }
}
