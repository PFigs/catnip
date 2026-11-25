//! GitHub requests and the parsing of their answers. The transport is in `net`.

use serde_json::{Value, json};

use crate::model::{Ci, Entry, EntryKind, MyReview, Pr, ReviewState, is_scope};

const GRAPHQL: &str = "https://api.github.com/graphql";
/// Pages of 50 the PR list fetches at most.
pub const MAX_PAGES: usize = 4;

pub struct Request {
    pub url: String,
    pub body: Option<String>,
    pub accept: &'static str,
}

fn graphql(query: &str, variables: Value) -> Request {
    Request {
        url: GRAPHQL.into(),
        body: Some(json!({ "query": query, "variables": variables }).to_string()),
        accept: "application/json",
    }
}

fn data(text: &str) -> Result<Value, String> {
    data_ignoring(text, |_| false)
}

/// `data`, also when every error is one that `ignore` accepts.
fn data_ignoring(text: &str, ignore: impl Fn(&Value) -> bool) -> Result<Value, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad JSON from GitHub: {e}"))?;
    if let Some(errors) = v
        .get("errors")
        .and_then(Value::as_array)
        .filter(|errors| !errors.iter().all(&ignore))
    {
        let messages: Vec<&str> = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect();
        return Err(messages.join("; "));
    }
    Ok(v["data"].clone())
}

/// An error on the CI state alone, which a token that may not read CI gets.
fn refused_ci(error: &Value) -> bool {
    error
        .pointer("/path")
        .and_then(Value::as_array)
        .and_then(|path| path.last())
        .and_then(Value::as_str)
        == Some("statusCheckRollup")
}

fn ci_at(v: &Value, pointer: &str) -> Option<Ci> {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .and_then(Ci::from_github)
}

fn str_at(v: &Value, pointer: &str) -> String {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn nodes<'a>(v: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    v.pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The author's login; a deleted account reads as `ghost`, as on GitHub.
fn login(v: &Value) -> String {
    v.pointer("/author/login")
        .and_then(Value::as_str)
        .unwrap_or("ghost")
        .to_owned()
}

fn review_state(s: &str) -> Option<ReviewState> {
    match s {
        "APPROVED" => Some(ReviewState::Approved),
        "COMMENTED" => Some(ReviewState::Commented),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        _ => None,
    }
}

pub struct Viewer {
    pub login: String,
    /// Search scopes from the viewer's orgs and the repos they recently worked in.
    pub suggestions: Vec<String>,
}

pub fn viewer() -> Request {
    graphql(
        "query { viewer { login organizations(first: 50) { nodes { login } }
          repositoriesContributedTo(first: 50, includeUserRepositories: true,
            contributionTypes: [COMMIT, PULL_REQUEST, PULL_REQUEST_REVIEW],
            orderBy: {field: PUSHED_AT, direction: DESC}) { nodes { nameWithOwner } } } }",
        json!({}),
    )
}

pub fn parse_viewer(text: &str) -> Result<Viewer, String> {
    let v = data(text)?;
    let scopes = |pointer: &str, field: &str, prefix: &str| -> Vec<String> {
        nodes(&v, pointer)
            .filter_map(|n| n[field].as_str().map(|s| format!("{prefix}{s}")))
            .collect()
    };
    let mut suggestions = scopes("/viewer/organizations/nodes", "login", "org:");
    suggestions.extend(scopes(
        "/viewer/repositoriesContributedTo/nodes",
        "nameWithOwner",
        "repo:",
    ));
    Ok(Viewer {
        login: str_at(&v, "/viewer/login"),
        suggestions,
    })
}

/// The search for open PRs in the given scopes. Anything but a scope is left out.
pub fn search_query(scopes: &[String]) -> String {
    let scopes: Vec<&str> = scopes
        .iter()
        .map(String::as_str)
        .filter(|s| is_scope(s))
        .collect();
    format!(
        "is:pr is:open archived:false sort:updated-desc {}",
        scopes.join(" ")
    )
}

pub fn prs(scopes: &[String], me: &str, after: Option<&str>) -> Request {
    graphql(
        "query($q: String!, $me: String!, $after: String) {
          search(query: $q, type: ISSUE, first: 50, after: $after) {
            pageInfo { hasNextPage endCursor }
            nodes { ... on PullRequest {
              id number title url isDraft additions deletions headRefOid updatedAt createdAt
              headRefName baseRefName isCrossRepository
              repository { nameWithOwner } author { login }
              commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
              reviews(last: 10, author: $me, states: [APPROVED, COMMENTED, CHANGES_REQUESTED]) {
                nodes { state commit { oid } } } } } } }",
        json!({ "q": search_query(scopes), "me": me, "after": after }),
    )
}

/// What follows a page of the PR list.
#[derive(Debug, PartialEq)]
pub enum Next {
    Done,
    /// The cursor of the next page.
    Page(String),
    /// More PRs exist, past the last page the list fetches.
    PastCap,
}

/// The PRs of page `page` (from 0) of the list, and what follows it. A token that may not read
/// the CI state still gets the PRs, with no CI state.
pub fn parse_prs(text: &str, page: usize) -> Result<(Vec<Pr>, Next), String> {
    let v = data_ignoring(text, refused_ci)?;
    let search = &v["search"];
    let prs = nodes(search, "/nodes")
        .filter(|n| n.get("id").is_some())
        .map(|n| Pr {
            id: str_at(n, "/id"),
            repo: str_at(n, "/repository/nameWithOwner"),
            number: n["number"].as_u64().unwrap_or_default(),
            title: str_at(n, "/title"),
            url: str_at(n, "/url"),
            author: login(n),
            draft: n["isDraft"].as_bool().unwrap_or_default(),
            head: str_at(n, "/headRefOid"),
            last_commit: str_at(n, "/commits/nodes/0/commit/committedDate"),
            updated: str_at(n, "/updatedAt"),
            created: str_at(n, "/createdAt"),
            additions: n["additions"].as_u64().unwrap_or_default(),
            deletions: n["deletions"].as_u64().unwrap_or_default(),
            my_reviews: nodes(n, "/reviews/nodes")
                .filter_map(|r| {
                    let state = review_state(&str_at(r, "/state"))?;
                    Some(MyReview {
                        state,
                        commit: str_at(r, "/commit/oid"),
                    })
                })
                .collect(),
            ci: ci_at(n, "/commits/nodes/0/commit/statusCheckRollup/state"),
            head_branch: str_at(n, "/headRefName"),
            base_branch: str_at(n, "/baseRefName"),
            fork: n["isCrossRepository"].as_bool().unwrap_or_default(),
        })
        .collect();
    let next = match search["pageInfo"]["hasNextPage"].as_bool() {
        Some(true) if page + 1 >= MAX_PAGES => Next::PastCap,
        Some(true) => Next::Page(str_at(search, "/pageInfo/endCursor")),
        _ => Next::Done,
    };
    Ok((prs, next))
}

/// An existing review thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    pub path: String,
    /// Its line in the diff; None for an outdated thread or a thread on a whole file.
    pub line: Option<u32>,
    /// The line GitHub first gave it; None for a thread on a whole file.
    pub original_line: Option<u32>,
    pub outdated: bool,
    /// The diff hunk its first comment was made on, which ends at the commented line.
    pub hunk: String,
    pub left: bool,
    pub resolved: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
    pub can_reply: bool,
    pub comments: Vec<Entry>,
}

/// The state of a thread after a resolve or an unresolve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadState {
    pub resolved: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub base_branch: String,
    pub head_branch: String,
    /// The commits the diff and the review cover.
    pub base_commit: String,
    pub head_commit: String,
    /// The CI state of the head commit; None when it has no CI.
    pub ci: Option<Ci>,
    /// GitHub lets the viewer merge the base into the head: the head is behind, and the viewer
    /// may push to it.
    pub can_update_branch: bool,
    /// The head and the base conflict.
    pub conflicts: bool,
    pub description: Entry,
    /// Comments, and reviews with a verdict or text, in time order.
    pub entries: Vec<Entry>,
    pub threads: Vec<Thread>,
    /// GitHub holds older comments, reviews, threads or thread replies than the ones loaded.
    pub truncated: bool,
}

pub fn detail(id: &str) -> Request {
    graphql(
        "query($id: ID!) { node(id: $id) { ... on PullRequest {
          id viewerCanUpdate body createdAt baseRefName headRefName baseRefOid headRefOid
          viewerCanUpdateBranch mergeable
          author { __typename login }
          commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
          comments(last: 100) { pageInfo { hasPreviousPage }
            nodes { id viewerCanUpdate viewerCanDelete author { __typename login } body createdAt } }
          reviews(last: 100) { pageInfo { hasPreviousPage }
            nodes { id viewerCanUpdate author { __typename login } body state submittedAt } }
          reviewThreads(last: 100) { pageInfo { hasPreviousPage } nodes {
            id isResolved isOutdated viewerCanResolve viewerCanUnresolve viewerCanReply
            path line originalLine diffSide first: comments(first: 1) { nodes { diffHunk } }
            comments(last: 50) { pageInfo { hasPreviousPage }
              nodes { id viewerCanUpdate viewerCanDelete author { __typename login } body createdAt url } } } } } } }",
        json!({ "id": id }),
    )
}

fn entry(n: &Value, at: &str, review: Option<ReviewState>, kind: EntryKind) -> Entry {
    let flag = |at: &str| n[at].as_bool().unwrap_or_default();
    Entry {
        author: login(n),
        bot: n.pointer("/author/__typename").and_then(Value::as_str) == Some("Bot"),
        at: str_at(n, at),
        body: str_at(n, "/body"),
        review,
        id: str_at(n, "/id"),
        kind,
        can_edit: flag("viewerCanUpdate"),
        can_delete: flag("viewerCanDelete"),
    }
}

/// The detail of a PR. A token that may not read the CI state still gets the PR, with no checks.
pub fn parse_detail(text: &str) -> Result<Detail, String> {
    let v = data_ignoring(text, refused_ci)?;
    let pr = &v["node"];
    if !pr.is_object() {
        return Err("GitHub returned no pull request.".into());
    }
    let cut = |list: &Value| list["pageInfo"]["hasPreviousPage"].as_bool() == Some(true);
    let mut entries: Vec<Entry> = nodes(pr, "/comments/nodes")
        .map(|n| entry(n, "/createdAt", None, EntryKind::Comment))
        .collect();
    let reviews = nodes(pr, "/reviews/nodes").map(|n| {
        let state = review_state(&str_at(n, "/state")).unwrap_or(ReviewState::Commented);
        entry(n, "/submittedAt", Some(state), EntryKind::Review)
    });
    // A review with no text and no verdict holds line comments; the threads show those.
    entries
        .extend(reviews.filter(|e| !e.body.is_empty() || e.review != Some(ReviewState::Commented)));
    entries.sort_by(|a, b| a.at.cmp(&b.at));
    let comments = |t: &Value| -> Vec<Entry> {
        nodes(t, "/comments/nodes")
            .map(|n| entry(n, "/createdAt", None, EntryKind::LineComment))
            .collect()
    };
    let line = |t: &Value, at: &str| t[at].as_u64().and_then(|l| u32::try_from(l).ok());
    let flag = |t: &Value, at: &str| t[at].as_bool().unwrap_or_default();
    let threads = nodes(pr, "/reviewThreads/nodes")
        .map(|t| Thread {
            id: str_at(t, "/id"),
            path: str_at(t, "/path"),
            line: line(t, "line"),
            original_line: line(t, "originalLine"),
            outdated: flag(t, "isOutdated"),
            hunk: str_at(t, "/first/nodes/0/diffHunk"),
            left: t["diffSide"].as_str() == Some("LEFT"),
            resolved: flag(t, "isResolved"),
            can_resolve: flag(t, "viewerCanResolve"),
            can_unresolve: flag(t, "viewerCanUnresolve"),
            can_reply: flag(t, "viewerCanReply"),
            comments: comments(t),
        })
        .collect();
    Ok(Detail {
        base_branch: str_at(pr, "/baseRefName"),
        head_branch: str_at(pr, "/headRefName"),
        base_commit: str_at(pr, "/baseRefOid"),
        head_commit: str_at(pr, "/headRefOid"),
        ci: ci_at(pr, "/commits/nodes/0/commit/statusCheckRollup/state"),
        can_update_branch: pr["viewerCanUpdateBranch"].as_bool().unwrap_or_default(),
        conflicts: pr["mergeable"].as_str() == Some("CONFLICTING"),
        description: entry(pr, "/createdAt", None, EntryKind::Description),
        entries,
        threads,
        truncated: ["comments", "reviews", "reviewThreads"]
            .iter()
            .any(|list| cut(&pr[list]))
            || nodes(pr, "/reviewThreads/nodes").any(|t| cut(&t["comments"])),
    })
}

/// The diff between the two commits of the detail, so it shows the code the review covers.
pub fn diff(repo: &str, detail: &Detail) -> Request {
    Request {
        url: format!(
            "https://api.github.com/repos/{repo}/compare/{}...{}",
            detail.base_commit, detail.head_commit
        ),
        body: None,
        accept: "application/vnd.github.diff",
    }
}

/// A line comment written in this app, sent with the next review.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub left: bool,
    pub body: String,
}

/// A review of the PR at `commit`, the head commit of the detail the viewer saw.
pub fn submit(
    pr_id: &str,
    commit: &str,
    state: ReviewState,
    body: &str,
    drafts: &[Draft],
) -> Request {
    let event = match state {
        ReviewState::Approved => "APPROVE",
        ReviewState::Commented => "COMMENT",
        ReviewState::ChangesRequested => "REQUEST_CHANGES",
    };
    let threads: Vec<Value> = drafts
        .iter()
        .map(|d| {
            let side = if d.left { "LEFT" } else { "RIGHT" };
            let mut t = json!({ "path": d.path, "line": d.end, "side": side, "body": d.body });
            if d.start != d.end {
                t["startLine"] = json!(d.start);
                t["startSide"] = json!(side);
            }
            t
        })
        .collect();
    graphql(
        "mutation($input: AddPullRequestReviewInput!) {
          addPullRequestReview(input: $input) { pullRequestReview { state commit { oid } } } }",
        json!({ "input": {
            "pullRequestId": pr_id, "commitOID": commit, "event": event, "body": body, "threads": threads,
        } }),
    )
}

pub fn parse_submit(text: &str) -> Result<MyReview, String> {
    let v = data(text)?;
    let r = &v["addPullRequestReview"]["pullRequestReview"];
    let state = review_state(&str_at(r, "/state")).ok_or("GitHub returned no review")?;
    Ok(MyReview {
        state,
        commit: str_at(r, "/commit/oid"),
    })
}

/// Which jobs a re-run starts again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rerun {
    Failed,
    All,
}

impl Rerun {
    /// Whether the re-run takes this run: a finished one, and for `Failed` one that did not pass.
    pub fn wants(self, run: &Run) -> bool {
        match self {
            Rerun::Failed => run.finished && run.failed,
            Rerun::All => run.finished,
        }
    }
}

/// A GitHub Actions run of a commit.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub id: u64,
    pub finished: bool,
    /// It failed, timed out or was cancelled.
    pub failed: bool,
}

/// The GitHub Actions runs of `head` in `repo`, the repository of the PR.
pub fn runs(repo: &str, head: &str) -> Request {
    Request {
        url: format!(
            "https://api.github.com/repos/{repo}/actions/runs?head_sha={head}&per_page=100"
        ),
        body: None,
        accept: "application/vnd.github+json",
    }
}

pub fn parse_runs(text: &str) -> Result<Vec<Run>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad JSON from GitHub: {e}"))?;
    Ok(nodes(&v, "/workflow_runs")
        .filter_map(|r| {
            let conclusion = r["conclusion"].as_str().unwrap_or_default();
            Some(Run {
                id: r["id"].as_u64()?,
                finished: r["status"].as_str() == Some("completed"),
                failed: ["failure", "timed_out", "cancelled"].contains(&conclusion),
            })
        })
        .collect())
}

/// A re-run of one run. The transport sends a POST for a request with a body.
pub fn rerun(repo: &str, run: u64, which: Rerun) -> Request {
    let endpoint = match which {
        Rerun::Failed => "rerun-failed-jobs",
        Rerun::All => "rerun",
    };
    Request {
        url: format!("https://api.github.com/repos/{repo}/actions/runs/{run}/{endpoint}"),
        body: Some("{}".into()),
        accept: "application/vnd.github+json",
    }
}

/// A merge of the base into the head of the PR, refused when the head is no longer `head`.
pub fn update_branch(pr_id: &str, head: &str) -> Request {
    graphql(
        "mutation($input: UpdatePullRequestBranchInput!) {
          updatePullRequestBranch(input: $input) { pullRequest { headRefOid } } }",
        json!({ "input": { "pullRequestId": pr_id, "expectedHeadOid": head, "updateMethod": "MERGE" } }),
    )
}

pub fn parse_update_branch(text: &str) -> Result<(), String> {
    let v = data(text)?;
    if v.pointer("/updatePullRequestBranch/pullRequest")
        .is_some_and(Value::is_object)
    {
        Ok(())
    } else {
        Err("GitHub did not update the branch".into())
    }
}

/// A comment on the conversation of the PR `pr_id`, with `body` trimmed.
pub fn add_comment(pr_id: &str, body: &str) -> Request {
    graphql(
        "mutation($input: AddCommentInput!) { addComment(input: $input) {
          commentEdge { node { id viewerCanUpdate viewerCanDelete
            author { __typename login } body createdAt } } } }",
        json!({ "input": { "subjectId": pr_id, "body": body.trim() } }),
    )
}

pub fn parse_add_comment(text: &str) -> Result<Entry, String> {
    let v = data(text)?;
    let node = &v["addComment"]["commentEdge"]["node"];
    if !node.is_object() {
        return Err("GitHub did not add the comment".into());
    }
    Ok(entry(node, "/createdAt", None, EntryKind::Comment))
}

/// A reply to the review thread `thread_id`, with `body` trimmed.
pub fn reply(thread_id: &str, body: &str) -> Request {
    graphql(
        "mutation($input: AddPullRequestReviewThreadReplyInput!) {
          addPullRequestReviewThreadReply(input: $input) {
            comment { id viewerCanUpdate viewerCanDelete author { __typename login } body createdAt } } }",
        json!({ "input": { "pullRequestReviewThreadId": thread_id, "body": body.trim() } }),
    )
}

pub fn parse_reply(text: &str) -> Result<Entry, String> {
    let v = data(text)?;
    let node = &v["addPullRequestReviewThreadReply"]["comment"];
    if !node.is_object() {
        return Err("GitHub did not add the reply".into());
    }
    Ok(entry(node, "/createdAt", None, EntryKind::LineComment))
}

/// Sets the text of the entry `id` of `kind` to `body`, trimmed.
pub fn edit(kind: EntryKind, id: &str, body: &str) -> Request {
    let (field, input, key) = match kind {
        EntryKind::Description => (
            "updatePullRequest",
            "UpdatePullRequestInput",
            "pullRequestId",
        ),
        EntryKind::Comment => ("updateIssueComment", "UpdateIssueCommentInput", "id"),
        EntryKind::Review => (
            "updatePullRequestReview",
            "UpdatePullRequestReviewInput",
            "pullRequestReviewId",
        ),
        EntryKind::LineComment => (
            "updatePullRequestReviewComment",
            "UpdatePullRequestReviewCommentInput",
            "pullRequestReviewCommentId",
        ),
    };
    let mut fields = json!({ "body": body.trim() });
    fields[key] = json!(id);
    graphql(
        &format!("mutation($input: {input}!) {{ {field}(input: $input) {{ clientMutationId }} }}"),
        json!({ "input": fields }),
    )
}

/// Deletes the entry `id` of `kind`; None for a review or a description, which GitHub keeps.
pub fn delete(kind: EntryKind, id: &str) -> Option<Request> {
    let (field, input) = match kind {
        EntryKind::Comment => ("deleteIssueComment", "DeleteIssueCommentInput"),
        EntryKind::LineComment => (
            "deletePullRequestReviewComment",
            "DeletePullRequestReviewCommentInput",
        ),
        EntryKind::Description | EntryKind::Review => return None,
    };
    Some(graphql(
        &format!("mutation($input: {input}!) {{ {field}(input: $input) {{ clientMutationId }} }}"),
        json!({ "input": { "id": id } }),
    ))
}

/// GitHub's answer to an edit or a delete: an error, or nothing to read.
pub fn parse_done(text: &str) -> Result<(), String> {
    data(text).map(|_| ())
}

/// The mutation that resolves a thread, or unresolves it, and its input type.
fn resolve_names(resolve: bool) -> (&'static str, &'static str) {
    if resolve {
        ("resolveReviewThread", "ResolveReviewThreadInput")
    } else {
        ("unresolveReviewThread", "UnresolveReviewThreadInput")
    }
}

/// Resolves the review thread `thread_id`, or unresolves it.
pub fn resolve(thread_id: &str, resolve: bool) -> Request {
    let (field, input) = resolve_names(resolve);
    graphql(
        &format!(
            "mutation($input: {input}!) {{ {field}(input: $input) {{
              thread {{ isResolved viewerCanResolve viewerCanUnresolve }} }} }}"
        ),
        json!({ "input": { "threadId": thread_id } }),
    )
}

pub fn parse_resolve(text: &str, resolve: bool) -> Result<ThreadState, String> {
    let v = data(text)?;
    let t = &v[resolve_names(resolve).0]["thread"];
    if !t.is_object() {
        return Err("GitHub did not change the thread".into());
    }
    Ok(ThreadState {
        resolved: t["isResolved"].as_bool().unwrap_or_default(),
        can_resolve: t["viewerCanResolve"].as_bool().unwrap_or_default(),
        can_unresolve: t["viewerCanUnresolve"].as_bool().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(req: &Request) -> Value {
        let body: Value = serde_json::from_str(req.body.as_deref().unwrap()).unwrap();
        body["variables"].clone()
    }

    fn detail_answer() -> Value {
        json!({ "data": { "node": {
            "id": "PR_1", "viewerCanUpdate": true,
            "body": "Adds X.", "createdAt": "2026-01-01T00:00:00Z",
            "baseRefName": "main", "headRefName": "feat",
            "baseRefOid": "b0", "headRefOid": "h1",
            "viewerCanUpdateBranch": true, "mergeable": "CONFLICTING",
            "author": { "__typename": "User", "login": "ann" },
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] },
            "comments": { "nodes": [
                { "id": "IC_1", "author": { "__typename": "Bot", "login": "ci" }, "body": "Build failed", "createdAt": "2026-01-02T00:00:00Z" },
                { "id": "IC_2", "viewerCanUpdate": true, "viewerCanDelete": true, "author": null, "body": "Old note", "createdAt": "2026-01-04T00:00:00Z" },
            ] },
            "reviews": { "nodes": [
                { "id": "PRR_1", "viewerCanUpdate": true, "author": { "__typename": "User", "login": "bob" }, "body": "", "state": "CHANGES_REQUESTED", "submittedAt": "2026-01-03T00:00:00Z" },
                { "author": { "__typename": "User", "login": "bob" }, "body": "", "state": "COMMENTED", "submittedAt": "2026-01-05T00:00:00Z" },
            ] },
            "reviewThreads": { "nodes": [
                { "id": "RT_1", "isResolved": true, "viewerCanResolve": false, "viewerCanUnresolve": true,
                  "path": "src/a.rs", "line": 12, "diffSide": "LEFT",
                  "comments": { "nodes": [{ "author": { "__typename": "User", "login": "bob" }, "body": "Why?", "createdAt": "2026-01-05T00:00:00Z" }] } },
                { "id": "RT_2", "isResolved": false, "isOutdated": true, "viewerCanReply": true,
                  "path": "src/b.rs", "line": null, "originalLine": 7, "diffSide": "RIGHT",
                  "first": { "nodes": [{ "diffHunk": "@@ -6,1 +6,2 @@\n a\n+b" }] },
                  "comments": { "nodes": [
                      { "id": "RC_2", "viewerCanUpdate": true, "viewerCanDelete": true, "author": { "__typename": "User", "login": "cyd" }, "body": "Gone now?", "createdAt": "2026-01-06T00:00:00Z", "url": "https://github.com/acme/api/pull/12#discussion_r2" },
                  ] } },
                { "isResolved": false, "path": "README.md", "line": null, "originalLine": null, "diffSide": "RIGHT",
                  "comments": { "nodes": [
                      { "author": { "__typename": "User", "login": "dee" }, "body": "On the whole file", "createdAt": "2026-01-03T00:00:00Z", "url": "https://github.com/acme/api/pull/12#discussion_r1" },
                  ] } },
                { "isResolved": true, "path": "src/c.rs", "line": null, "originalLine": 3, "diffSide": "RIGHT",
                  "comments": { "nodes": [] } },
            ] },
        } } })
    }

    #[test]
    fn detail_parse_reads_ids_and_edit_rights() {
        let d = parse_detail(&detail_answer().to_string()).unwrap();
        let rights = |e: &Entry| (e.id.clone(), e.kind, e.can_edit, e.can_delete);
        let item = |id: &str, kind, edit, delete| (id.to_owned(), kind, edit, delete);
        assert_eq!(
            rights(&d.description),
            item("PR_1", EntryKind::Description, true, false)
        );
        assert_eq!(
            d.entries.iter().map(rights).collect::<Vec<_>>(),
            [
                item("IC_1", EntryKind::Comment, false, false),
                item("PRR_1", EntryKind::Review, true, false),
                item("IC_2", EntryKind::Comment, true, true),
            ]
        );
        let thread = d.threads.iter().find(|t| t.id == "RT_2").unwrap();
        assert_eq!(
            rights(&thread.comments[0]),
            item("RC_2", EntryKind::LineComment, true, true)
        );
    }

    #[test]
    fn graphql_errors_become_one_message() {
        let answer = json!({
            "data": { "search": { "nodes": [] } },
            "errors": [{ "message": "Field 'x' doesn't exist" }, { "message": "Timeout" }],
        });
        assert_eq!(
            parse_prs(&answer.to_string(), 0).unwrap_err(),
            "Field 'x' doesn't exist; Timeout"
        );
    }

    #[test]
    fn list_parse_skips_other_nodes_and_names_a_deleted_author() {
        let answer = |has_next: bool| {
            json!({ "data": { "search": {
                "pageInfo": { "hasNextPage": has_next, "endCursor": "c1" },
                "nodes": [
                    {},
                    null,
                    { "id": "PR_1", "number": 7, "title": "Fix", "url": "u", "isDraft": false,
                      "additions": 3, "deletions": 1, "headRefOid": "h1",
                      "repository": { "nameWithOwner": "acme/api" }, "author": null,
                      "commits": { "nodes": [{ "commit": { "committedDate": "2026-01-02T00:00:00Z" } }] },
                      "reviews": { "nodes": [
                          { "state": "APPROVED", "commit": { "oid": "h0" } },
                          { "state": "COMMENTED", "commit": { "oid": "h1" } },
                      ] } },
                ],
            } } })
            .to_string()
        };
        let (prs, next) = parse_prs(&answer(true), 0).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(
            (prs[0].author.as_str(), prs[0].repo.as_str(), prs[0].number),
            ("ghost", "acme/api", 7)
        );
        assert_eq!(prs[0].last_commit, "2026-01-02T00:00:00Z");
        assert_eq!(
            prs[0].my_reviews,
            [
                MyReview {
                    state: ReviewState::Approved,
                    commit: "h0".into()
                },
                MyReview {
                    state: ReviewState::Commented,
                    commit: "h1".into()
                },
            ]
        );
        assert_eq!(next, Next::Page("c1".into()));
        assert_eq!(
            parse_prs(&answer(true), MAX_PAGES - 1).unwrap().1,
            Next::PastCap
        );
        assert_eq!(
            parse_prs(&answer(false), MAX_PAGES - 1).unwrap().1,
            Next::Done
        );
    }

    #[test]
    fn list_parse_reads_the_ci_state_and_accepts_a_refused_rollup() {
        let node = |id: &str, rollup: Value| {
            json!({ "id": id, "number": 1, "title": "t", "url": "u", "isDraft": false,
                "additions": 0, "deletions": 0, "headRefOid": "h",
                "repository": { "nameWithOwner": "acme/api" }, "author": { "login": "ann" },
                "commits": { "nodes": [{ "commit": {
                    "committedDate": "2026-01-02T00:00:00Z", "statusCheckRollup": rollup } }] },
                "reviews": { "nodes": [] } })
        };
        let state = |s: &str| json!({ "state": s });
        let mut answer = json!({ "data": { "search": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [
                node("SUCCESS", state("SUCCESS")),
                node("FAILURE", state("FAILURE")),
                node("ERROR", state("ERROR")),
                node("PENDING", state("PENDING")),
                node("EXPECTED", state("EXPECTED")),
                node("NONE", Value::Null),
            ],
        } } });
        let ci = |answer: &Value| -> Vec<Option<Ci>> {
            let (prs, _) = parse_prs(&answer.to_string(), 0).unwrap();
            prs.iter().map(|p| p.ci).collect()
        };
        assert_eq!(
            ci(&answer),
            [
                Some(Ci::Passes),
                Some(Ci::Fails),
                Some(Ci::Fails),
                Some(Ci::Runs),
                Some(Ci::Runs),
                None
            ]
        );

        let refused = |path: Value| json!([{ "message": "Resource not accessible by personal access token", "path": path }]);
        answer["data"]["search"]["nodes"][0]["commits"]["nodes"][0]["commit"]["statusCheckRollup"] =
            Value::Null;
        answer["errors"] = refused(json!([
            "search",
            "nodes",
            0,
            "commits",
            "nodes",
            0,
            "commit",
            "statusCheckRollup"
        ]));
        assert_eq!(ci(&answer)[..2], [None, Some(Ci::Fails)]);
        answer["errors"] = refused(json!(["search", "nodes", 0, "reviews"]));
        assert!(parse_prs(&answer.to_string(), 0).is_err());
    }

    #[test]
    fn detail_parse_keeps_threads_with_no_line_apart_and_orders_the_conversation() {
        let d = parse_detail(&detail_answer().to_string()).unwrap();
        assert_eq!(
            (d.base_commit.as_str(), d.head_commit.as_str()),
            ("b0", "h1")
        );
        assert_eq!(
            (d.ci, d.can_update_branch, d.conflicts),
            (Some(Ci::Fails), true, true)
        );
        assert_eq!(
            (d.description.author.as_str(), d.description.bot),
            ("ann", false)
        );
        let entries: Vec<(&str, bool, Option<ReviewState>)> = d
            .entries
            .iter()
            .map(|e| (e.author.as_str(), e.bot, e.review))
            .collect();
        assert_eq!(
            entries,
            [
                ("ci", true, None),
                ("bob", false, Some(ReviewState::ChangesRequested)),
                ("ghost", false, None)
            ]
        );
        let places: Vec<(&str, Option<u32>, Option<u32>, bool)> = d
            .threads
            .iter()
            .map(|t| (t.path.as_str(), t.line, t.original_line, t.outdated))
            .collect();
        assert_eq!(
            places,
            [
                ("src/a.rs", Some(12), None, false),
                ("src/b.rs", None, Some(7), true),
                ("README.md", None, None, false),
                ("src/c.rs", None, Some(3), false),
            ],
            "every thread, with or without a line"
        );
        let t = &d.threads[0];
        assert_eq!(
            (
                t.id.as_str(),
                t.left,
                t.resolved,
                t.can_resolve,
                t.can_unresolve,
                t.can_reply
            ),
            ("RT_1", true, true, false, true, false)
        );
        let t = &d.threads[1];
        assert_eq!(
            (t.id.as_str(), t.hunk.as_str(), t.can_reply, t.left),
            ("RT_2", "@@ -6,1 +6,2 @@\n a\n+b", true, false)
        );
        assert_eq!(t.comments[0].author, "cyd");
    }

    #[test]
    fn reply_names_the_thread_and_reads_the_new_comment() {
        let req = reply("RT_1", "  Fixed.\n ");
        assert!(
            req.body
                .as_deref()
                .unwrap()
                .contains("addPullRequestReviewThreadReply(")
        );
        assert_eq!(
            variables(&req)["input"],
            json!({ "pullRequestReviewThreadId": "RT_1", "body": "Fixed." })
        );
        let answer = json!({ "data": { "addPullRequestReviewThreadReply": { "comment": {
            "author": { "__typename": "User", "login": "me" },
            "body": "Fixed.", "createdAt": "2026-01-07T00:00:00Z",
        } } } });
        let e = parse_reply(&answer.to_string()).unwrap();
        assert_eq!(
            (e.author.as_str(), e.body.as_str(), e.at.as_str()),
            ("me", "Fixed.", "2026-01-07T00:00:00Z")
        );
        let empty = json!({ "data": { "addPullRequestReviewThreadReply": null },
            "errors": [{ "message": "Could not reply" }] });
        assert_eq!(
            parse_reply(&empty.to_string()),
            Err("Could not reply".into())
        );
    }

    #[test]
    fn add_comment_names_the_pr_and_reads_the_new_comment() {
        let req = add_comment("PR_1", "  Looks good.\n ");
        assert_eq!(
            variables(&req)["input"],
            json!({ "subjectId": "PR_1", "body": "Looks good." })
        );
        let answer = json!({ "data": { "addComment": { "commentEdge": { "node": {
            "author": { "__typename": "User", "login": "me" },
            "body": "Looks good.", "createdAt": "2026-01-07T00:00:00Z",
        } } } } });
        let e = parse_add_comment(&answer.to_string()).unwrap();
        assert_eq!(
            (e.author.as_str(), e.body.as_str(), e.at.as_str(), e.review),
            ("me", "Looks good.", "2026-01-07T00:00:00Z", None)
        );
        let refused = json!({ "data": { "addComment": null },
            "errors": [{ "message": "Resource not accessible by integration" }] });
        assert_eq!(
            parse_add_comment(&refused.to_string()),
            Err("Resource not accessible by integration".into())
        );
        let empty = json!({ "data": { "addComment": { "commentEdge": null } } });
        assert!(parse_add_comment(&empty.to_string()).is_err());
    }

    #[test]
    fn resolve_and_unresolve_name_the_thread_and_read_the_answer() {
        let resolve = super::resolve("RT_1", true);
        let body = resolve.body.as_deref().unwrap();
        assert!(body.contains("resolveReviewThread(") && !body.contains("unresolveReviewThread"));
        assert_eq!(variables(&resolve)["input"], json!({ "threadId": "RT_1" }));
        let unresolve = super::resolve("RT_2", false);
        assert!(
            unresolve
                .body
                .as_deref()
                .unwrap()
                .contains("unresolveReviewThread(")
        );
        assert_eq!(
            variables(&unresolve)["input"],
            json!({ "threadId": "RT_2" })
        );

        let answer =
            |key: &str, thread: Value| json!({ "data": { key: { "thread": thread } } }).to_string();
        let state =
            json!({ "isResolved": true, "viewerCanResolve": false, "viewerCanUnresolve": true });
        assert_eq!(
            parse_resolve(&answer("resolveReviewThread", state.clone()), true),
            Ok(ThreadState {
                resolved: true,
                can_resolve: false,
                can_unresolve: true
            })
        );
        assert!(parse_resolve(&answer("resolveReviewThread", state), false).is_err());
        assert!(parse_resolve(&answer("resolveReviewThread", Value::Null), true).is_err());
        let refused = json!({ "data": { "resolveReviewThread": null },
            "errors": [{ "message": "Resource not accessible by integration" }] });
        assert_eq!(
            parse_resolve(&refused.to_string(), true),
            Err("Resource not accessible by integration".into())
        );
    }

    #[test]
    fn a_detail_cut_at_100_entries_says_so() {
        let mut answer = detail_answer();
        assert!(!parse_detail(&answer.to_string()).unwrap().truncated);
        let cut = json!({ "hasPreviousPage": true });
        let mut thread_cut = answer.clone();
        thread_cut["data"]["node"]["reviewThreads"]["nodes"][0]["comments"]["pageInfo"] =
            cut.clone();
        assert!(parse_detail(&thread_cut.to_string()).unwrap().truncated);
        answer["data"]["node"]["reviewThreads"]["pageInfo"] = cut;
        assert!(parse_detail(&answer.to_string()).unwrap().truncated);
    }

    #[test]
    fn a_checks_permission_error_keeps_the_pr() {
        let mut answer = detail_answer();
        answer["data"]["node"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = Value::Null;
        let error = |path: Value| json!([{ "message": "Resource not accessible by personal access token", "path": path }]);
        answer["errors"] = error(json!([
            "node",
            "commits",
            "nodes",
            0,
            "commit",
            "statusCheckRollup"
        ]));
        let d = parse_detail(&answer.to_string()).unwrap();
        assert_eq!((d.head_commit.as_str(), d.ci), ("h1", None));

        answer["errors"] = error(json!(["node", "comments"]));
        assert!(parse_detail(&answer.to_string()).is_err());
        answer["errors"] = error(json!([
            "node",
            "commits",
            "nodes",
            0,
            "commit",
            "statusCheckRollup"
        ]));
        answer["data"]["node"] = Value::Null;
        assert!(parse_detail(&answer.to_string()).is_err());
    }

    #[test]
    fn diff_request_names_the_base_and_head_commits_of_the_detail() {
        let d = parse_detail(&detail_answer().to_string()).unwrap();
        let req = diff("acme/api", &d);
        assert_eq!(
            req.url,
            "https://api.github.com/repos/acme/api/compare/b0...h1"
        );
        assert_eq!(
            (req.body.is_none(), req.accept),
            (true, "application/vnd.github.diff")
        );
    }

    #[test]
    fn review_request_carries_line_ranges_side_and_commit() {
        let drafts = [
            Draft {
                path: "src/a.rs".into(),
                start: 4,
                end: 4,
                left: false,
                body: "One line".into(),
            },
            Draft {
                path: "src/b.rs".into(),
                start: 2,
                end: 5,
                left: true,
                body: "A range".into(),
            },
        ];
        let req = submit(
            "PR_1",
            "h1",
            ReviewState::ChangesRequested,
            "See notes",
            &drafts,
        );
        assert_eq!(
            variables(&req)["input"],
            json!({
                "pullRequestId": "PR_1",
                "commitOID": "h1",
                "event": "REQUEST_CHANGES",
                "body": "See notes",
                "threads": [
                    { "path": "src/a.rs", "line": 4, "side": "RIGHT", "body": "One line" },
                    { "path": "src/b.rs", "line": 5, "side": "LEFT", "startLine": 2, "startSide": "LEFT", "body": "A range" },
                ],
            })
        );
    }

    #[test]
    #[ignore = "sends read-only requests to GitHub and needs a token"]
    fn live_queries_are_accepted_by_github() {
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
            let out = std::process::Command::new("gh")
                .args(["auth", "token"])
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        });
        let send = |req: Request| {
            let auth = format!("Bearer {token}");
            let answer = match &req.body {
                Some(body) => ureq::post(&req.url)
                    .header("Authorization", &auth)
                    .header("Accept", req.accept)
                    .header("User-Agent", "catnip")
                    .send(body.as_str()),
                None => ureq::get(&req.url)
                    .header("Authorization", &auth)
                    .header("Accept", req.accept)
                    .header("User-Agent", "catnip")
                    .call(),
            };
            answer
                .unwrap()
                .body_mut()
                .with_config()
                .limit(64 << 20)
                .read_to_string()
                .unwrap()
        };

        let me = parse_viewer(&send(viewer())).unwrap().login;
        let (list, next) =
            parse_prs(&send(prs(&["repo:serde-rs/json".into()], &me, None)), 0).unwrap();
        let pr = &list[0];
        let d = parse_detail(&send(detail(&pr.id))).unwrap();
        let patch = send(diff(&pr.repo, &d));
        let actions = parse_runs(&send(runs(&pr.repo, &d.head_commit))).unwrap();
        assert!(!d.base_commit.is_empty() && !d.head_commit.is_empty());
        assert!(patch.starts_with("diff --git"), "{patch:.200}");
        println!("viewer {me}; {} PRs, next {next:?}", list.len());
        let forks = list.iter().filter(|p| p.fork).count();
        println!(
            "{} → {}, {forks} from forks",
            pr.head_branch, pr.base_branch
        );
        assert!(
            list.iter()
                .all(|p| !p.head_branch.is_empty() && !p.base_branch.is_empty()),
            "the list reads both branches"
        );
        assert!(forks > 0, "the list reads isCrossRepository");
        println!(
            "{}#{} base {} head {} ci {:?}, list ci {:?}, can update {}, conflicts {}",
            pr.repo,
            pr.number,
            d.base_commit,
            d.head_commit,
            d.ci,
            pr.ci,
            d.can_update_branch,
            d.conflicts
        );
        println!(
            "{} entries, {} threads, diff of {} bytes, {} Actions runs",
            d.entries.len(),
            d.threads.len(),
            patch.len(),
            actions.len()
        );

        // No mutation and no re-run is ever sent. The input keys of the mutations are checked
        // against the schema instead.
        let fields = |ty: &str| -> Vec<String> {
            let query =
                "query($t: String!) { __type(name: $t) { fields { name } inputFields { name } } }";
            let v = data(&send(graphql(query, json!({ "t": ty })))).unwrap();
            let names = nodes(&v, "/__type/fields").chain(nodes(&v, "/__type/inputFields"));
            names.map(|f| str_at(f, "/name")).collect()
        };
        let range = Draft {
            path: "a".into(),
            start: 1,
            end: 2,
            left: true,
            body: "b".into(),
        };
        let input = &variables(&submit(
            &pr.id,
            &d.head_commit,
            ReviewState::Commented,
            "",
            &[range],
        ))["input"];
        let known = |ty: &str, keys: &Value| {
            let fields = fields(ty);
            for key in keys.as_object().unwrap().keys() {
                assert!(fields.contains(key), "{ty} has no {key}");
            }
        };
        known("AddPullRequestReviewInput", input);
        known("DraftPullRequestReviewThread", &input["threads"][0]);
        assert!(fields("AddPullRequestReviewPayload").contains(&"pullRequestReview".to_owned()));
        let update = &variables(&update_branch(&pr.id, &d.head_commit))["input"];
        known("UpdatePullRequestBranchInput", update);
        assert!(fields("UpdatePullRequestBranchPayload").contains(&"pullRequest".to_owned()));
        known(
            "ResolveReviewThreadInput",
            &variables(&resolve("RT", true))["input"],
        );
        known(
            "UnresolveReviewThreadInput",
            &variables(&resolve("RT", false))["input"],
        );
        known(
            "AddPullRequestReviewThreadReplyInput",
            &variables(&reply("RT", "b"))["input"],
        );
        assert!(fields("AddPullRequestReviewThreadReplyPayload").contains(&"comment".to_owned()));
        for payload in ["ResolveReviewThreadPayload", "UnresolveReviewThreadPayload"] {
            assert!(fields(payload).contains(&"thread".to_owned()), "{payload}");
        }
        let thread = fields("PullRequestReviewThread");
        for field in ["isResolved", "viewerCanResolve", "viewerCanUnresolve"] {
            assert!(
                thread.contains(&field.to_owned()),
                "the thread has no {field}"
            );
        }
        known(
            "AddCommentInput",
            &variables(&add_comment("PR", "b"))["input"],
        );
        let has = |ty: &str, field: &str| {
            assert!(
                fields(ty).contains(&field.to_owned()),
                "{ty} has no {field}"
            );
        };
        has("AddCommentPayload", "commentEdge");
        has("IssueCommentEdge", "node");
        for field in ["author", "body", "createdAt"] {
            has("IssueComment", field);
        }
    }
}
