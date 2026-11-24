//! Canned PRs and one open PR, for snapshots, scripts and the view tests. No network, no token.

use rune_kit::diff::parse;

use crate::app::App;
use crate::github::{Detail, Draft, Thread};
use crate::model::{Ci, Entry, MyReview, Pr, ReviewState};

const PATCH: &str = "diff --git a/src/net/retry.rs b/src/net/retry.rs
--- a/src/net/retry.rs
+++ b/src/net/retry.rs
@@ -1,15 +1,16 @@
 use std::time::Duration;
\x20
-/// How long to wait before the next try.
+/// How long to wait before the next try, with jitter so clients spread out.
 pub struct Backoff {
     base: Duration,
     max: Duration,
     attempt: u32,
+    jitter: f32,
 }
\x20
 impl Backoff {
     pub fn next(&mut self) -> Duration {
-        let wait = self.base * 2u32.pow(self.attempt);
+        let wait = self.base.saturating_mul(2u32.saturating_pow(self.attempt));
         self.attempt += 1;
         wait.min(self.max)
     }
diff --git a/src/net/pool.rs b/src/net/pool.rs
--- a/src/net/pool.rs
+++ b/src/net/pool.rs
@@ -8,11 +8,12 @@ impl Pool {
     pub fn checkout(&self) -> Result<Conn, Error> {
         let mut tries = Backoff::new(self.config.base, self.config.max);
         loop {
             match self.try_checkout() {
                 Ok(conn) => return Ok(conn),
-                Err(e) if e.is_busy() => std::thread::sleep(tries.next()),
+                Err(e) if e.is_busy() => std::thread::sleep(tries.next_with_jitter()),
                 Err(e) => return Err(e),
             }
         }
     }
+    // A busy pool waits with jitter, so a burst of clients does not retry at once.
 }
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 pub mod config;
 pub mod net;
-pub use net::retry::Backoff;
+pub use net::retry::{Backoff, Jitter};
 pub use net::pool::Pool;
diff --git a/config/retry.toml b/config/retry.toml
--- a/config/retry.toml
+++ b/config/retry.toml
@@ -1,3 +1,4 @@
 [retry]
 base_ms = 100
 max_ms = 1000
+jitter = 0.2
diff --git a/docs/retry.md b/docs/retry.md
--- a/docs/retry.md
+++ b/docs/retry.md
@@ -3,2 +3,4 @@
 A failed request waits before the next try.
 The wait doubles after each try, up to the cap.
+Each wait adds a random share of up to `jitter`,
+so many clients do not retry at the same moment.
";

const DESCRIPTION: &str =
    "Retries from many clients line up after an outage. This adds **jitter** to the \
backoff so they spread out.

- `jitter` defaults to 0.2
- the doubling saturates instead of overflowing";

fn entry(author: &str, bot: bool, at: &str, review: Option<ReviewState>, body: &str) -> Entry {
    Entry {
        author: author.into(),
        bot,
        at: at.into(),
        body: body.into(),
        review,
        id: format!("{author}-{at}"),
        can_edit: author == "me",
        can_delete: author == "me",
        ..Entry::default()
    }
}

fn pr(n: u64, repo: &str, author: &str, title: &str, at: &str) -> Pr {
    Pr {
        id: format!("PR_{n}"),
        repo: repo.into(),
        number: n,
        title: title.into(),
        url: format!("https://github.com/{repo}/pull/{n}"),
        author: author.into(),
        draft: !n.is_multiple_of(3),
        head: format!("head{n}"),
        last_commit: at.into(),
        additions: 3,
        deletions: 2,
        my_reviews: Vec::new(),
        ci: None,
        head_branch: format!("{author}/branch-{n}"),
        base_branch: "main".into(),
        fork: false,
    }
}

/// Thirty PRs by eight authors and the viewer in three repositories, most of them drafts, with
/// each CI state and none; the group of acme/infra is folded. The first PR is open in Split
/// mode, stacked on #4207, with a conversation, two open review threads (one the viewer may not
/// resolve), a resolved one and one with no line, five files in nested folders, the first one
/// open, and one unsent line comment.
pub fn sample(s: &mut App) {
    const TITLES: [&str; 6] = [
        "Add jitter to the retry backoff",
        "Move the partner registry to its own stack",
        "Fix the hourly compaction lock",
        "Show the CI state in the PR header",
        "Drop the legacy delivery folder layout",
        "Cache parsed diffs by head commit",
    ];
    const REPOS: [&str; 3] = ["acme/api", "acme/web", "acme/infra"];
    const AUTHORS: [&str; 8] = ["ann", "bob", "cyd", "dee", "eli", "fay", "gus", "hal"];
    let mut prs: Vec<Pr> = (0..30)
        .map(|i| {
            let at = format!("2026-11-{:02}T{:02}:00:00Z", 14 - i / 3, 20 - i % 12);
            let title = format!("{} ({})", TITLES[i % TITLES.len()], i + 1);
            pr(4210 - i as u64, REPOS[i % 3], AUTHORS[i % 8], &title, &at)
        })
        .collect();
    prs[0].title = TITLES[0].into();
    // The open PR is stacked on #4207: its base branch is the head branch of that PR.
    prs[0].head_branch = "ann/retry-jitter".into();
    prs[0].base_branch = prs[3].head_branch.clone();
    prs[10].author = "me".into();
    let reviewed = |state, head: &str| MyReview {
        state,
        commit: head.into(),
    };
    prs[4].my_reviews = vec![reviewed(ReviewState::Approved, "old")];
    prs[7].my_reviews = vec![reviewed(ReviewState::Approved, "head4203")];
    prs[8].my_reviews = vec![reviewed(ReviewState::ChangesRequested, "head4202")];
    prs[9].my_reviews = vec![reviewed(ReviewState::Commented, "head4201")];
    let states = [Some(Ci::Passes), Some(Ci::Fails), Some(Ci::Runs), None];
    for (i, pr) in prs.iter_mut().enumerate() {
        pr.ci = states[i % states.len()];
    }

    s.me = "me".into();
    s.settings.scopes = vec!["org:acme".into()];
    s.settings.folded = ["acme/infra".to_owned()].into();
    s.settings.authors.clear();
    s.settings.tree_closed = false;
    s.picked.clear();
    s.suggestions = vec![
        "org:acme".into(),
        "org:acme-labs".into(),
        "repo:me/dotfiles".into(),
    ];
    s.prs = prs;
    s.loading = false;
    s.selected = Some("PR_4210".into());
    s.refilter();

    let thread = |id: &str, path: &str, line, resolved, can_resolve, comments| Thread {
        id: id.into(),
        path: path.into(),
        line: Some(line),
        original_line: Some(line),
        outdated: false,
        hunk: String::new(),
        left: false,
        resolved,
        can_resolve,
        can_unresolve: resolved,
        can_reply: true,
        comments,
    };
    let outdated = |id: &str, path: &str, line, hunk: &str, comments| Thread {
        line: None,
        outdated: true,
        hunk: hunk.into(),
        ..thread(id, path, line, false, true, comments)
    };
    // Its line still holds the code, nine lines on.
    let moved = outdated(
        "RT_4",
        "src/net/retry.rs",
        6,
        "@@ -4,3 +4,3 @@\n pub struct Backoff {\n     max: Duration,\n     attempt: u32,",
        vec![entry(
            "eli",
            false,
            "2026-11-14T14:00:00Z",
            None,
            "Does `attempt` reset after a success?",
        )],
    );
    // Its line is gone.
    let gone = outdated(
        "RT_5",
        "src/net/pool.rs",
        9,
        "@@ -7,3 +7,3 @@\n impl Pool {\n     pub fn checkout(&self) -> Result<Conn, Error> {\n+        let mut tries = 3;",
        vec![entry(
            "bob",
            false,
            "2026-11-14T13:30:00Z",
            None,
            "Why 3 tries and not the config value?",
        )],
    );
    let open = thread(
        "RT_1",
        "src/net/retry.rs",
        15,
        false,
        true,
        vec![
            entry(
                "bob",
                false,
                "2026-11-14T15:00:00Z",
                None,
                "Can `saturating_pow` hide a bug here?",
            ),
            entry(
                "ann",
                false,
                "2026-11-14T16:00:00Z",
                None,
                "It caps at `max` anyway.",
            ),
        ],
    );
    let resolved = thread(
        "RT_2",
        "src/net/pool.rs",
        13,
        true,
        false,
        vec![entry(
            "cyd",
            false,
            "2026-11-14T15:30:00Z",
            None,
            "Should the pool share one jitter source?",
        )],
    );
    // A thread the viewer may not resolve.
    let locked = thread(
        "RT_3",
        "src/lib.rs",
        3,
        false,
        false,
        vec![entry(
            "dee",
            false,
            "2026-11-14T17:00:00Z",
            None,
            "Is `Jitter` part of the public API now?",
        )],
    );
    s.detail = Some(Ok(Detail {
        base_branch: "dee/branch-4207".into(),
        head_branch: "ann/retry-jitter".into(),
        base_commit: "base4210".into(),
        head_commit: "head4210".into(),
        ci: Some(Ci::Passes),
        can_update_branch: false,
        conflicts: false,
        description: entry("ann", false, "2026-11-14T09:00:00Z", None, DESCRIPTION),
        entries: vec![
            entry(
                "github-actions",
                true,
                "2026-11-14T10:00:00Z",
                None,
                "Coverage: 91% (+0.4%)",
            ),
            entry(
                "cyd",
                false,
                "2026-11-14T11:00:00Z",
                None,
                "Does the web client need the same change?",
            ),
            entry(
                "me",
                false,
                "2026-11-14T11:30:00Z",
                None,
                "Yes, the web client retries the same way.",
            ),
            entry(
                "bob",
                false,
                "2026-11-14T12:00:00Z",
                Some(ReviewState::ChangesRequested),
                "Please add a test for the cap.",
            ),
            entry(
                "dee",
                false,
                "2026-11-14T13:00:00Z",
                None,
                "Linked from the outage notes.",
            ),
        ],
        threads: vec![open, resolved, locked, moved, gone],
        truncated: false,
    }));
    s.show_files(parse(PATCH));
    s.diff.set_collapsed(0, false);
    s.drafts.insert(
        ("PR_4210".into(), "head4210".into()),
        vec![Draft {
            path: "config/retry.toml".into(),
            start: 4,
            end: 4,
            left: false,
            body: "Name the unit: `jitter_ratio`?".into(),
        }],
    );
    s.annotate();
}

/// The app before the first PR arrives: the drawer stacks its placeholder cards.
pub fn booting(_: &mut App) {}
