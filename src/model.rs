//! The data the app shows and the rules that sort and filter it. No I/O here.

use std::collections::{BTreeSet, HashMap};

use rune_kit::search::Fold;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    Commented,
    ChangesRequested,
}

/// One of the viewer's reviews on a PR, and the commit it was made on.
#[derive(Clone, Debug, PartialEq)]
pub struct MyReview {
    pub state: ReviewState,
    pub commit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pr {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub draft: bool,
    pub head: String,
    /// ISO 8601 time of the head commit; sorts as text.
    pub last_commit: String,
    pub additions: u64,
    pub deletions: u64,
    /// The viewer's last ten reviews, oldest first.
    pub my_reviews: Vec<MyReview>,
    /// The CI state of the head commit; None when it has no CI.
    pub ci: Option<Ci>,
    pub head_branch: String,
    pub base_branch: String,
    /// The head branch lives in a fork.
    pub fork: bool,
}

/// The CI state of a commit, the same for the drawer row and the PR pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ci {
    Passes,
    Fails,
    Runs,
}

impl Ci {
    /// The state for a value of GitHub's `StatusState`.
    pub fn from_github(state: &str) -> Option<Ci> {
        match state {
            "SUCCESS" => Some(Ci::Passes),
            "FAILURE" | "ERROR" => Some(Ci::Fails),
            "PENDING" | "EXPECTED" => Some(Ci::Runs),
            _ => None,
        }
    }
}

/// What the viewer has done on a PR, shown as the mark in the drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Mine,
    Todo,
    Acted(ReviewState),
    /// The viewer reviewed it, and the author pushed after that.
    NewCommits(ReviewState),
}

pub fn mark(pr: &Pr, me: &str) -> Mark {
    if pr.author == me {
        return Mark::Mine;
    }
    let Some(newest) = pr.my_reviews.last() else {
        return Mark::Todo;
    };
    let verdict = pr
        .my_reviews
        .iter()
        .rev()
        .map(|r| r.state)
        .find(|&s| s != ReviewState::Commented)
        .unwrap_or(ReviewState::Commented);
    if newest.commit == pr.head {
        Mark::Acted(verdict)
    } else {
        Mark::NewCommits(verdict)
    }
}

/// Whether the viewer approved `head` and requested no changes after that approval.
pub fn approved_at(reviews: &[MyReview], head: &str) -> bool {
    reviews
        .iter()
        .rposition(|r| r.state == ReviewState::Approved && r.commit == head)
        .is_some_and(|at| {
            reviews[at..]
                .iter()
                .all(|r| r.state != ReviewState::ChangesRequested)
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quick {
    #[default]
    ToReview,
    ActedOn,
    All,
}

impl Quick {
    pub const ALL: [Quick; 3] = [Quick::ToReview, Quick::ActedOn, Quick::All];

    pub fn label(self) -> &'static str {
        match self {
            Quick::ToReview => "To review",
            Quick::ActedOn => "Acted on",
            Quick::All => "All",
        }
    }

    fn keeps(self, mark: Mark) -> bool {
        match self {
            Quick::ToReview => matches!(mark, Mark::Todo | Mark::NewCommits(_)),
            Quick::ActedOn => matches!(mark, Mark::Acted(_)),
            Quick::All => true,
        }
    }
}

/// The author chips: authors of the given PRs, the one with the newest commit first, then the
/// picked authors with no PR in the list.
pub fn author_chips(prs: &[Pr], picked: &BTreeSet<String>) -> Vec<String> {
    let authors = newest_first(prs, |pr| &pr.author);
    let without_prs = picked.iter().filter(|a| !authors.contains(&a.as_str()));
    authors
        .iter()
        .map(|&a| a.to_owned())
        .chain(without_prs.cloned())
        .collect()
}

/// The distinct values of `key` over `prs`, the one with the newest commit first.
fn newest_first<'a>(prs: &'a [Pr], key: impl Fn(&'a Pr) -> &'a str) -> Vec<&'a str> {
    let mut newest: HashMap<&str, &str> = HashMap::new();
    for pr in prs {
        let at = newest.entry(key(pr)).or_insert(&pr.last_commit);
        if pr.last_commit.as_str() > *at {
            *at = &pr.last_commit;
        }
    }
    let mut keys: Vec<(&str, &str)> = newest.into_iter().collect();
    keys.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    keys.into_iter().map(|(k, _)| k).collect()
}

/// A row of the drawer: the header of a repository, or a PR by its index in the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// `count` PRs of `repo` pass the filters; `folded` hides them.
    Header {
        repo: String,
        count: usize,
        folded: bool,
    },
    Pr(usize),
}

/// The rows of the drawer for the `shown` PRs. With more than one repository in the list, the
/// PRs group under a header per repository, the one with the newest commit first; a folded
/// group lists no PR, unless `searching`.
pub fn drawer_rows(
    prs: &[Pr],
    shown: &[usize],
    folded: &BTreeSet<String>,
    searching: bool,
) -> Vec<Row> {
    let repos = newest_first(prs, |pr| &pr.repo);
    if repos.len() < 2 {
        return shown.iter().map(|&i| Row::Pr(i)).collect();
    }
    let mut rows = Vec::new();
    for repo in repos {
        let group: Vec<usize> = shown
            .iter()
            .copied()
            .filter(|&i| prs[i].repo == repo)
            .collect();
        if group.is_empty() {
            continue;
        }
        let hidden = !searching && folded.contains(repo);
        rows.push(Row::Header {
            repo: repo.to_owned(),
            count: group.len(),
            folded: hidden,
        });
        if !hidden {
            rows.extend(group.into_iter().map(Row::Pr));
        }
    }
    rows
}

/// Whether `text` holds every word of `query`: no case, and a query with no accent matches a
/// text with one. An empty query matches everything.
pub fn matches(text: &str, query: &str) -> bool {
    query
        .split_whitespace()
        .all(|word| Fold::default().contains(text, word))
}

/// Whether a PR's title, repository, number or author holds every word of `query`.
fn found(pr: &Pr, query: &str) -> bool {
    let text = format!("{} {} #{} {}", pr.title, pr.repo, pr.number, pr.author);
    matches(&text, query)
}

/// Indexes of the PRs the drawer lists, newest commit first. An empty author set keeps all.
/// `search` narrows inside the quick filter and the authors, by title, repository, number and
/// author.
pub fn visible(
    prs: &[Pr],
    me: &str,
    quick: Quick,
    authors: &BTreeSet<String>,
    search: &str,
) -> Vec<usize> {
    let mut keep: Vec<usize> = (0..prs.len())
        .filter(|&i| authors.is_empty() || authors.contains(&prs[i].author))
        .filter(|&i| quick.keeps(mark(&prs[i], me)))
        .filter(|&i| found(&prs[i], search))
        .collect();
    keep.sort_by(|&a, &b| prs[b].last_commit.cmp(&prs[a].last_commit));
    keep
}

/// Whether a search scope is `org:<name>`, `user:<name>` or `repo:<owner>/<name>`, the
/// qualifiers GitHub joins with OR.
pub fn is_scope(scope: &str) -> bool {
    let name = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    match scope.split_once(':') {
        Some(("org" | "user", login)) => name(login),
        Some(("repo", repo)) => repo
            .split_once('/')
            .is_some_and(|(owner, repo)| name(owner) && name(repo)),
        _ => false,
    }
}

/// One entry of a PR's conversation: the description, a comment or a review.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub author: String,
    pub bot: bool,
    pub at: String,
    pub body: String,
    /// The verdict of a review; `None` for a comment.
    pub review: Option<ReviewState>,
}

/// The indexes of the entries under the description, newest first unless `oldest_first`.
/// Folded: at most three, picked by reviews first, then comments by people, then comments by
/// bots, the newest of each group first. Unfolded: all.
pub fn conversation(entries: &[Entry], unfolded: bool, oldest_first: bool) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..entries.len()).collect();
    if !unfolded {
        let group = |e: &Entry| match (e.review, e.bot) {
            (Some(_), _) => 0,
            (None, false) => 1,
            (None, true) => 2,
        };
        shown.sort_by(|&x, &y| {
            let (a, b) = (&entries[x], &entries[y]);
            group(a).cmp(&group(b)).then(b.at.cmp(&a.at))
        });
        shown.truncate(3);
    }
    shown.sort_by(|&x, &y| entries[x].at.cmp(&entries[y].at));
    if !oldest_first {
        shown.reverse();
    }
    shown
}

/// The open PR of `prs` whose head branch is the base branch of `pr`: same repository, not from
/// a fork, the newest commit first. An empty branch name matches nothing.
pub fn base_pr<'a>(prs: &'a [Pr], pr: &Pr) -> Option<&'a Pr> {
    if pr.base_branch.is_empty() {
        return None;
    }
    prs.iter()
        .filter(|p| p.repo == pr.repo && !p.fork && p.id != pr.id)
        .filter(|p| p.head_branch == pr.base_branch)
        .max_by(|a, b| a.last_commit.cmp(&b.last_commit))
}

/// Seconds since 1970 of a GitHub time such as `2026-11-14T17:28:51Z`.
fn unix(iso: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let days = 365 * y + y / 4 - y / 100 + y / 400 + (153 * m + 2) / 5 + d - 719_469;
    Some(days * 86_400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?)
}

/// "5m", "3h", "2d", "4mo" from a GitHub time to `now`, in seconds since 1970.
pub fn age(iso: &str, now: i64) -> String {
    let Some(t) = unix(iso) else {
        return String::new();
    };
    let s = (now - t).max(0);
    match s {
        0..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..2_592_000 => format!("{}d", s / 86_400),
        _ => format!("{}mo", s / 2_592_000),
    }
}

/// The line of `lines`, a file's lines on one side in order, that holds the code a thread was
/// made on: the last line of `hunk` on the thread's side, `left` for the old one. Of the lines
/// that hold it, the one with the most of the hunk's lines just before it wins; a tie places
/// nothing.
pub fn relocate(hunk: &str, left: bool, lines: &[(u32, &str)]) -> Option<u32> {
    let other = if left { '+' } else { '-' };
    if hunk.lines().last()?.starts_with(other) {
        return None;
    }
    let side: Vec<&str> = hunk
        .lines()
        .filter(|l| !l.starts_with("@@") && !l.starts_with(other))
        .map(|l| l.get(1..).unwrap_or("").trim())
        .collect();
    let (target, before) = side.split_last()?;
    if target.is_empty() {
        return None;
    }
    let context = |i: usize| {
        before
            .iter()
            .rev()
            .zip(lines[..i].iter().rev())
            .zip(1..)
            .take_while(|((want, (n, text)), k)| *n + k == lines[i].0 && text.trim() == **want)
            .count()
    };
    let mut scores: Vec<(usize, u32)> = (0..lines.len())
        .filter(|&i| lines[i].1.trim() == *target)
        .map(|i| (context(i), lines[i].0))
        .collect();
    scores.sort_unstable_by_key(|s| std::cmp::Reverse(s.0));
    match scores.as_slice() {
        [(best, n), (next, _), ..] if best > next => Some(*n),
        [(_, n)] => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(id: &str, author: &str, last_commit: &str, reviews: &[(ReviewState, &str)]) -> Pr {
        Pr {
            id: id.into(),
            repo: "org/repo".into(),
            number: 1,
            title: id.into(),
            url: String::new(),
            author: author.into(),
            draft: false,
            head: "head".into(),
            last_commit: last_commit.into(),
            additions: 0,
            deletions: 0,
            my_reviews: reviews
                .iter()
                .map(|&(state, commit)| MyReview {
                    state,
                    commit: commit.into(),
                })
                .collect(),
            ci: None,
            head_branch: String::new(),
            base_branch: String::new(),
            fork: false,
        }
    }

    fn entry(author: &str, bot: bool, at: &str, review: Option<ReviewState>) -> Entry {
        Entry {
            author: author.into(),
            bot,
            at: at.into(),
            body: String::new(),
            review,
        }
    }

    #[test]
    fn authors_sort_by_their_newest_commit_not_their_oldest() {
        let prs = [
            pr("a1", "old", "2025-01-01T00:00:00Z", &[]),
            pr("b1", "busy", "2024-06-01T00:00:00Z", &[]),
            pr("b2", "busy", "2026-11-01T00:00:00Z", &[]),
            pr("c1", "mid", "2026-05-01T00:00:00Z", &[]),
        ];
        assert_eq!(author_chips(&prs, &BTreeSet::new()), ["busy", "mid", "old"]);
    }

    #[test]
    fn a_picked_author_without_open_prs_keeps_a_chip() {
        let prs = [pr("a1", "ann", "2026-01-01T00:00:00Z", &[])];
        let picked = BTreeSet::from(["ann".to_owned(), "gone".to_owned()]);
        assert_eq!(author_chips(&prs, &picked), ["ann", "gone"]);
    }

    #[test]
    fn a_push_after_my_review_brings_the_pr_back() {
        let reviewed = pr("p", "bob", "t", &[(ReviewState::Approved, "head")]);
        let pushed = pr("p", "bob", "t", &[(ReviewState::Approved, "older")]);
        assert_eq!(mark(&reviewed, "me"), Mark::Acted(ReviewState::Approved));
        assert_eq!(mark(&pushed, "me"), Mark::NewCommits(ReviewState::Approved));
        assert_eq!(mark(&pr("p", "me", "t", &[]), "me"), Mark::Mine);
    }

    #[test]
    fn a_reply_after_my_approval_keeps_the_approval_mark() {
        let replied = [
            (ReviewState::ChangesRequested, "older"),
            (ReviewState::Approved, "older"),
            (ReviewState::Commented, "head"),
        ];
        assert_eq!(
            mark(&pr("p", "bob", "t", &replied), "me"),
            Mark::Acted(ReviewState::Approved)
        );
        let only_replies = [
            (ReviewState::Commented, "older"),
            (ReviewState::Commented, "older"),
        ];
        assert_eq!(
            mark(&pr("p", "bob", "t", &only_replies), "me"),
            Mark::NewCommits(ReviewState::Commented)
        );
    }

    #[test]
    fn quick_filters_split_todo_from_acted_and_respect_authors() {
        let prs = [
            pr("todo", "bob", "2026-01-03", &[]),
            pr(
                "acted",
                "bob",
                "2026-01-02",
                &[(ReviewState::Commented, "head")],
            ),
            pr(
                "pushed",
                "ann",
                "2026-01-01",
                &[(ReviewState::Commented, "older")],
            ),
            pr("mine", "me", "2026-01-04", &[]),
        ];
        let none = BTreeSet::new();
        let ids = |v: Vec<usize>| {
            v.into_iter()
                .map(|i| prs[i].id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(visible(&prs, "me", Quick::ToReview, &none, "")),
            ["todo", "pushed"]
        );
        assert_eq!(
            ids(visible(&prs, "me", Quick::ActedOn, &none, "")),
            ["acted"]
        );
        assert_eq!(
            ids(visible(&prs, "me", Quick::All, &none, "")),
            ["mine", "todo", "acted", "pushed"]
        );
        let ann = BTreeSet::from(["ann".to_owned()]);
        assert_eq!(
            ids(visible(&prs, "me", Quick::ToReview, &ann, "")),
            ["pushed"]
        );
    }

    #[test]
    fn pr_search_matches_title_repo_number_and_author_inside_the_filters() {
        let mut fix = pr("fix", "bob", "2026-01-04", &[]);
        fix.title = "Fix the Résumé upload".into();
        let mut web = pr("web", "cat", "2026-01-03", &[]);
        web.repo = "acme/web".into();
        let mut numbered = pr("numbered", "bob", "2026-01-02", &[]);
        numbered.number = 4217;
        let mut acted = pr(
            "acted",
            "bob",
            "2026-01-01",
            &[(ReviewState::Commented, "head")],
        );
        acted.title = "Fix the login".into();
        let prs = [fix, web, numbered, acted];
        let none = BTreeSet::new();
        let found = |query: &str, authors: &BTreeSet<String>| {
            visible(&prs, "me", Quick::ToReview, authors, query)
                .into_iter()
                .map(|i| prs[i].id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(found("resume", &none), ["fix"]);
        assert_eq!(found("FIX upload", &none), ["fix"]);
        assert_eq!(found("acme/web", &none), ["web"]);
        assert_eq!(found("4217", &none), ["numbered"]);
        assert_eq!(found("#4217", &none), ["numbered"]);
        assert_eq!(found("cat", &none), ["web"]);
        assert_eq!(
            found("fix", &none),
            ["fix"],
            "the quick filter hides the acted PR"
        );
        let cat = BTreeSet::from(["cat".to_owned()]);
        assert!(found("fix", &cat).is_empty(), "the author chips hide bob");
        assert_eq!(found("  ", &none).len(), 3);
    }

    #[test]
    fn conversation_puts_reviews_and_people_before_bots() {
        let entries = [
            entry("ci", true, "2026-01-01", None),
            entry(
                "ann",
                false,
                "2026-01-02",
                Some(ReviewState::ChangesRequested),
            ),
            entry("bob", false, "2026-01-03", None),
            entry("cat", false, "2026-01-04", None),
            entry("ci", true, "2026-01-05", None),
        ];
        let at = |v: Vec<usize>| {
            v.iter()
                .map(|&n| entries[n].at.as_str())
                .collect::<Vec<_>>()
        };
        let folded = ["2026-01-04", "2026-01-03", "2026-01-02"];
        assert_eq!(at(conversation(&entries, false, false)), folded);
        let oldest: Vec<&str> = folded.iter().rev().copied().collect();
        assert_eq!(at(conversation(&entries, false, true)), oldest);
        let in_time_order: Vec<&str> = entries.iter().map(|e| e.at.as_str()).collect();
        assert_eq!(at(conversation(&entries, true, true)), in_time_order);
        let newest: Vec<&str> = in_time_order.iter().rev().copied().collect();
        assert_eq!(at(conversation(&entries, true, false)), newest);
    }

    #[test]
    fn scopes_accept_only_owner_and_repo_qualifiers() {
        for scope in [
            "org:acme",
            "user:bob-1",
            "repo:acme/api.rs",
            "repo:acme/my_api",
        ] {
            assert!(is_scope(scope), "{scope}");
        }
        for scope in [
            "involves:@me",
            "review-requested:@me",
            "is:pr",
            "acme",
            "org:",
            "repo:acme",
            "repo:acme/",
            "repo:a/b/c",
            "user:bob/x",
            "org:acme is:draft",
        ] {
            assert!(!is_scope(scope), "{scope}");
        }
    }

    #[test]
    fn github_times_convert_to_unix_seconds() {
        assert_eq!(unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix("2026-11-14T17:28:51Z"), Some(1_794_677_331));
    }

    #[test]
    fn the_base_pr_is_the_open_pr_of_the_same_repo_whose_head_is_the_base_branch() {
        let at = |id: &str, repo: &str, head: &str, base: &str, fork: bool, when: &str| Pr {
            repo: repo.into(),
            head_branch: head.into(),
            base_branch: base.into(),
            fork,
            ..pr(id, "ann", when, &[])
        };
        let prs = [
            at(
                "top",
                "org/repo",
                "ann/step-2",
                "ann/step-1",
                false,
                "2026-11-01",
            ),
            at(
                "older",
                "org/repo",
                "ann/step-1",
                "main",
                false,
                "2026-11-02",
            ),
            at(
                "newer",
                "org/repo",
                "ann/step-1",
                "main",
                false,
                "2026-11-03",
            ),
            at("fork", "org/repo", "ann/step-1", "main", true, "2026-11-09"),
            at(
                "other",
                "org/web",
                "ann/step-1",
                "main",
                false,
                "2026-11-09",
            ),
            at("fork-main", "org/repo", "main", "main", true, "2026-11-09"),
            at("no-base", "org/repo", "ann/x", "", false, "2026-11-01"),
            at("no-head", "org/repo", "", "main", false, "2026-11-01"),
        ];
        let base = |i: usize| base_pr(&prs, &prs[i]).map(|p| p.id.as_str());
        assert_eq!(
            base(0),
            Some("newer"),
            "the newest of two, no fork, same repo"
        );
        assert_eq!(base(1), None, "a fork whose head branch is main is no base");
        assert_eq!(base(6), None, "an empty branch name matches nothing");
    }

    #[test]
    fn relocate_finds_the_one_line_that_holds_the_code() {
        let hunk = "@@ -1,4 +1,4 @@\n fn cap() {\n-    let n = 2;\n+    let n = 4;\n     n * size";
        let moved = [
            (10, "fn cap() {"),
            (11, "    let n = 4;"),
            (12, "    n * size"),
        ];
        assert_eq!(
            relocate(hunk, false, &moved),
            Some(12),
            "the line moved down"
        );
        let twice = [
            (3, "    let n = 9;"),
            (4, "    n * size"),
            (20, "    let n = 4;"),
            (21, "    n * size"),
        ];
        assert_eq!(
            relocate(hunk, false, &twice),
            Some(21),
            "the line before breaks the tie"
        );
        let tie = [(3, "    n * size"), (9, "    n * size")];
        assert_eq!(
            relocate(hunk, false, &tie),
            None,
            "two equal matches place nothing"
        );
        let gone = [(10, "fn cap() {"), (11, "    n * size * 2")];
        assert_eq!(
            relocate(hunk, false, &gone),
            None,
            "an edited line places nothing"
        );
        let gap = [
            (5, "    let n = 4;"),
            (40, "    n * size"),
            (60, "    n * size"),
        ];
        assert_eq!(
            relocate(hunk, false, &gap),
            None,
            "lines apart in the file are no context"
        );
        let old = [(30, "fn cap() {"), (31, "    let n = 2;")];
        assert_eq!(
            relocate("@@ -1,2 +1,2 @@\n fn cap() {\n-    let n = 2;", true, &old),
            Some(31),
            "a thread on the old side matches the old lines"
        );
        assert_eq!(
            relocate("@@ -1,2 +1,1 @@\n fn cap() {\n-    let n = 2;", false, &old),
            None,
            "a hunk that ends on the other side holds no line of this side"
        );
    }
}
