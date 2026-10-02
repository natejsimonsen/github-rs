//! GitHub API client. GraphQL for lists and PR details (one request each),
//! REST for per-file diffs and for posting comments and reviews.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use serde_json::{Value, json};
use std::time::Duration;

const API: &str = "https://api.github.com";

pub struct Client {
    agent: ureq::Agent,
    token: String,
}

pub type Result<T> = std::result::Result<T, Error>;

/// When set, every request that would change something on GitHub fails
/// instead. Offscreen snapshot runs turn this on, so a scripted click can
/// never merge, comment, or resolve anything for real.
pub static READ_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Cache-only mode for screenshot reviews: API reads are skipped too, so
/// the app shows what's on disk and spends none of the GitHub rate limit.
pub static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn check_online() -> Result<()> {
    if OFFLINE.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(Error::Offline);
    }
    Ok(())
}

fn check_writable() -> Result<()> {
    if READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(Error::Other("read-only mode, so nothing was sent to GitHub".into()));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Error {
    /// The token is missing, expired, or lacks access.
    Unauthorized(String),
    Other(String),
    /// Cache-only mode: nothing was asked of GitHub. Not shown as an error.
    Offline,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unauthorized(m) | Error::Other(m) => f.write_str(m),
            Error::Offline => f.write_str("offline"),
        }
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Other(format!("Network error: {e}"))
    }
}

/// HTTP client shared by API calls and image downloads. Certificates are
/// checked with the OS trust store, so corporate proxies with custom root
/// certificates work.
pub fn http_agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent("github-prs")
        .tls_config(tls)
        .build()
        .into()
}

// ---------- Models ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub login: String,
    #[serde(default, alias = "avatar_url")]
    pub avatar_url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub name_with_owner: String,
    // Merge settings. Only loaded on the PR page.
    #[serde(default)]
    pub auto_merge_allowed: bool,
    #[serde(default)]
    pub merge_commit_allowed: bool,
    #[serde(default)]
    pub squash_merge_allowed: bool,
    #[serde(default)]
    pub rebase_merge_allowed: bool,
    #[serde(default)]
    pub viewer_default_merge_method: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub is_resolved: bool,
    #[serde(default)]
    pub id: String,
    /// The code it points at changed since.
    #[serde(default)]
    pub is_outdated: bool,
    #[serde(default)]
    pub path: String,
    pub line: Option<u64>,
    pub original_line: Option<u64>,
    #[serde(default)]
    pub viewer_can_resolve: bool,
    #[serde(default)]
    pub viewer_can_unresolve: bool,
    #[serde(default)]
    pub viewer_can_reply: bool,
    pub resolved_by: Option<Actor>,
    #[serde(default)]
    pub comments: Nodes<ThreadComment>,
}

/// A comment inside a review thread (a conversation on a line of code).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ThreadComment {
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
    #[serde(default)]
    pub diff_hunk: String,
    /// The review this comment was posted in.
    pub pull_request_review: Option<Id>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Id {
    pub id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Opinion {
    pub state: String,
    pub author: Option<Actor>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    pub total_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Nodes<T> {
    #[serde(default = "Vec::new")]
    pub nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Nodes { nodes: Vec::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Rollup {
    pub state: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeadCommit {
    pub status_check_rollup: Option<Rollup>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CommitNode<T> {
    pub commit: T,
}

/// One row in a PR list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrSummary {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub author: Option<Actor>,
    pub repository: Repo,
    #[serde(default)]
    pub comments: Count,
    #[serde(default)]
    pub labels: Nodes<Label>,
    #[serde(default)]
    pub commits: Nodes<CommitNode<HeadCommit>>,
    /// Filled in by a separate, slower request. See `Client::merge_states`.
    #[serde(default)]
    pub merge_state_status: Option<String>,
    #[serde(default)]
    pub auto_merge: bool,
    #[serde(default = "no_nodes")]
    pub assignees: Nodes<Actor>,
    /// APPROVED, CHANGES_REQUESTED or REVIEW_REQUIRED.
    #[serde(default)]
    pub review_decision: Option<String>,
}

fn no_nodes<T>() -> Nodes<T> {
    Nodes { nodes: Vec::new() }
}

impl PrSummary {
    pub fn ci(&self) -> Option<&str> {
        self.commits.nodes.first()?.commit.status_check_rollup.as_ref().map(|r| r.state.as_str())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    #[serde(default)]
    pub id: String,
    pub author: Option<Actor>,
    pub body: String,
    pub state: String,
    pub submitted_at: Option<String>,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitAuthor {
    pub name: Option<String>,
    pub user: Option<Actor>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub oid: String,
    pub abbreviated_oid: String,
    pub message_headline: String,
    /// The rest of the message, after the first line.
    #[serde(default)]
    pub message_body: String,
    pub committed_date: String,
    pub author: Option<CommitAuthor>,
    pub status_check_rollup: Option<Rollup>,
    /// Signed and verified by GitHub.
    #[serde(default)]
    pub signature: Option<Signature>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub is_valid: bool,
}

/// Something that happened on the PR besides comments and reviews:
/// merged, closed, commits pushed, labels added, and so on. Fields are
/// filled in depending on `kind` (GitHub's type name, like "MergedEvent").
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(rename = "__typename")]
    pub kind: String,
    #[serde(default)]
    pub created_at: String,
    pub actor: Option<Actor>,
    /// MergedEvent and PullRequestCommit.
    pub commit: Option<EventCommit>,
    pub merge_ref_name: Option<String>,
    pub head_ref_name: Option<String>,
    pub before_commit: Option<EventCommit>,
    pub after_commit: Option<EventCommit>,
    pub label: Option<Label>,
    pub requested_reviewer: Option<Reviewer>,
    pub assignee: Option<Reviewer>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventCommit {
    #[serde(default)]
    pub oid: String,
    #[serde(default)]
    pub abbreviated_oid: String,
    #[serde(default)]
    pub message_headline: String,
    #[serde(default)]
    pub committed_date: String,
    pub author: Option<CommitAuthor>,
    /// Overall CI state (SUCCESS, FAILURE, PENDING, ...), if any ran.
    #[serde(default)]
    pub status_check_rollup: Option<RollupState>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RollupState {
    pub state: String,
}

/// A CI check. GitHub has two kinds (check runs and legacy statuses); both
/// are flattened into this one shape.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Check {
    pub name: String,
    pub group: Option<String>,
    /// SUCCESS, FAILURE, PENDING, SKIPPED, NEUTRAL, CANCELLED, ...
    pub result: String,
    /// Required by branch rules to merge.
    #[serde(default)]
    pub required: bool,
    pub description: Option<String>,
    pub url: Option<String>,
    /// The app's picture (GitHub Actions, CircleCI, ...).
    #[serde(default)]
    pub avatar: Option<String>,
    /// What started it, like "pull_request" or "push".
    #[serde(default)]
    pub event: Option<String>,
    /// Run time in seconds, when finished.
    #[serde(default)]
    pub seconds: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Reviewer {
    User {
        login: String,
        #[serde(rename = "avatarUrl")]
        avatar_url: String,
    },
    Team { name: String },
    Other {},
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub requested_reviewer: Option<Reviewer>,
}

/// Everything shown in the detail pane except file diffs.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrDetail {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub state: String,
    pub is_draft: bool,
    pub created_at: String,
    pub merged_at: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub mergeable: String,
    /// CLEAN, BLOCKED, BEHIND, DIRTY, UNSTABLE, HAS_HOOKS, DRAFT or UNKNOWN.
    pub merge_state_status: Option<String>,
    pub auto_merge_request: Option<AutoMerge>,
    pub review_decision: Option<String>,
    #[serde(default)]
    pub head_ref_oid: String,
    #[serde(default)]
    pub viewer_can_enable_auto_merge: bool,
    #[serde(default)]
    pub viewer_can_disable_auto_merge: bool,
    /// Each reviewer's latest approve / request-changes verdict.
    #[serde(default)]
    pub latest_opinionated_reviews: Nodes<Opinion>,
    #[serde(default)]
    pub review_threads: Nodes<Thread>,
    pub author: Option<Actor>,
    pub repository: Repo,
    pub labels: Nodes<Label>,
    pub assignees: Nodes<Actor>,
    pub review_requests: Nodes<ReviewRequest>,
    pub comments: Nodes<Comment>,
    pub reviews: Nodes<Review>,
    pub commits: CommitList,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub timeline_items: Nodes<Event>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutoMerge {
    /// MERGE, SQUASH or REBASE.
    pub merge_method: String,
    pub enabled_by: Option<Actor>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitList {
    pub total_count: u64,
    pub nodes: Vec<CommitNode<Commit>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileDiff {
    pub filename: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
    pub previous_filename: Option<String>,
    /// You ticked "Viewed" on GitHub for this file.
    #[serde(default)]
    pub viewed: bool,
}

/// One commit with its changed files, from the REST API.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommitDetail {
    pub sha: String,
    pub html_url: String,
    pub commit: CommitMeta,
    /// The GitHub account behind the commit, if GitHub could tell.
    pub author: Option<Actor>,
    pub committer: Option<Actor>,
    #[serde(default)]
    pub parents: Vec<Parent>,
    #[serde(default)]
    pub files: Arc<Vec<FileDiff>>,
}

impl CommitDetail {
    pub fn headline(&self) -> &str {
        self.commit.message.lines().next().unwrap_or("").trim()
    }

    /// The message after its first line.
    pub fn body(&self) -> &str {
        self.commit.message.split_once('\n').map(|(_, b)| b.trim()).unwrap_or("")
    }

    pub fn short_sha(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CommitMeta {
    #[serde(default)]
    pub message: String,
    pub author: Option<GitSignature>,
    pub committer: Option<GitSignature>,
    pub verification: Option<Verification>,
}

/// Who wrote or committed, as recorded in git itself.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GitSignature {
    pub name: Option<String>,
    pub date: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Verification {
    #[serde(default)]
    pub verified: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Parent {
    pub sha: String,
}

/// A sidebar section: a name, an icon, and the search that fills it.
pub struct Section {
    pub title: &'static str,
    pub query: &'static str,
}

/// Rows per page. Small on purpose: GitHub search time grows with row count.
pub const LIST_SIZE: usize = 10;
/// Rows per "Load more", like a page on github.com.
pub const MORE_SIZE: usize = 25;

pub const SECTIONS: &[Section] = &[
    Section { title: "Created", query: "author:@me" },
    Section { title: "Assigned", query: "assignee:@me" },
    Section { title: "Mentioned", query: "mentions:@me" },
    Section { title: "Review requests", query: "review-requested:@me" },
    Section { title: "Involved", query: "involves:@me" },
];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ListResult {
    pub open: u64,
    pub closed: u64,
    pub rows: Vec<PrSummary>,
    /// Where the next page starts, if there is one.
    #[serde(default)]
    pub next: Option<String>,
}

/// Fields for the first, fast pass over a list. Every extra field makes
/// GitHub's search slower, so the rest come in `EXTRA_FIELDS`.
const FAST_FIELDS: &str = "
  nodes { ... on PullRequest {
    id number title url state isDraft createdAt updatedAt closedAt mergedAt
    author { login }
    repository { nameWithOwner }
  } }";

/// Fetched by PR id after the list shows, which skips search entirely.
const EXTRA_FIELDS: &str = "
    id
    author { login avatarUrl(size: 64) }
    comments { totalCount }
    assignees(first: 3) { nodes { login avatarUrl(size: 64) } }
    reviewDecision
    labels(first: 20) { nodes { name color } }
    commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 100) { pageInfo { hasNextPage } nodes {
      __typename ... on CheckRun { name status conclusion } ... on StatusContext { context state }
    } } } } } }";

/// `closed` switches every search between open and closed PRs.
pub fn full_query(q: &str, closed: bool) -> String {
    let state = if closed { "is:closed" } else { "is:open" };
    format!("is:pr {state} archived:false {q}")
}

impl Client {
    pub fn new(token: String) -> Self {
        Client { agent: http_agent(), token }
    }

    fn check_status(status: u16, body: &str) -> Result<()> {
        if (200..300).contains(&status) {
            return Ok(());
        }
        let msg = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| body.chars().take(200).collect());
        if status == 401 {
            return Err(Error::Unauthorized(format!("GitHub rejected the token: {msg}")));
        }
        Err(Error::Other(format!("GitHub error {status}: {msg}")))
    }

    fn read(resp: ureq::http::Response<ureq::Body>) -> Result<String> {
        let status = resp.status().as_u16();
        let mut body = resp.into_body();
        let body = body.with_config().limit(64 * 1024 * 1024).read_to_string()?;
        Self::check_status(status, &body)?;
        Ok(body)
    }

    /// Like `graphql`, but any error fails. For search, where a bad query
    /// otherwise just looks like "no results".
    fn graphql_checked(&self, query: &str, vars: Value) -> Result<Value> {
        check_online()?;
        let resp = self
            .agent
            .post(format!("{API}/graphql"))
            .header("Authorization", format!("bearer {}", self.token))
            .send_json(json!({ "query": query, "variables": vars }))?;
        let body = Self::read(resp)?;
        let mut v: Value = serde_json::from_str(&body).map_err(|e| Error::Other(format!("Bad JSON: {e}")))?;
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array()).filter(|e| !e.is_empty()) {
            let msg: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
            return Err(Error::Other(msg.join("\n")));
        }
        Ok(v["data"].take())
    }

    fn graphql(&self, query: &str, vars: Value) -> Result<Value> {
        check_online()?;
        let resp = self
            .agent
            .post(format!("{API}/graphql"))
            .header("Authorization", format!("bearer {}", self.token))
            .send_json(json!({ "query": query, "variables": vars }))?;
        let body = Self::read(resp)?;
        let mut v: Value =
            serde_json::from_str(&body).map_err(|e| Error::Other(format!("Bad JSON: {e}")))?;
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array()) {
            // GitHub returns partial data with errors (for example when an
            // org blocks the token). Only fail if there's no data at all.
            if v.get("data").is_none_or(|d| d.is_null()) {
                let msg: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
                return Err(Error::Other(msg.join("\n")));
            }
        }
        Ok(v["data"].take())
    }

    fn rest_get(&self, path: &str) -> Result<String> {
        check_online()?;
        let resp = self
            .agent
            .get(format!("{API}{path}"))
            .header("Authorization", format!("bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .call()?;
        Self::read(resp)
    }

    /// A file's full text at a commit, for expanding diff context.
    pub fn file_text(&self, repo: &str, path: &str, git_ref: &str) -> Result<String> {
        let path: String = path.split('/').map(|p| p.replace('%', "%25").replace(' ', "%20").replace('#', "%23").replace('?', "%3F")).collect::<Vec<_>>().join("/");
        let resp = self
            .agent
            .get(format!("{API}/repos/{repo}/contents/{path}?ref={git_ref}"))
            .header("Authorization", format!("bearer {}", self.token))
            .header("Accept", "application/vnd.github.raw+json")
            .call()?;
        Self::read(resp)
    }

    fn rest_post(&self, path: &str, body: Value) -> Result<()> {
        check_writable()?;
        let resp = self
            .agent
            .post(format!("{API}{path}"))
            .header("Authorization", format!("bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .send_json(body)?;
        Self::read(resp).map(|_| ())
    }

    pub fn viewer(&self) -> Result<String> {
        let data = self.graphql("query { viewer { login } }", json!({}))?;
        Ok(data["viewer"]["login"].as_str().unwrap_or_default().to_string())
    }

    /// Step 1 of loading a list: a lean search so rows show up fast.
    /// `first` rows, starting after the cursor `after`. Also returns the
    /// cursor for the page after these, if there is one.
    pub fn search(&self, query: &str, closed: bool, first: usize, after: Option<&str>) -> Result<(Vec<PrSummary>, Option<String>)> {
        let q = format!(
            "query($q: String!, $after: String) {{ search(query: $q, type: ISSUE, first: {}, after: $after) {{
              pageInfo {{ hasNextPage endCursor }} {FAST_FIELDS} }} }}",
            first.clamp(1, 100)
        );
        let data = self.graphql_checked(&q, json!({ "q": full_query(query, closed), "after": after }))?;
        let page = &data["search"]["pageInfo"];
        let next = (page["hasNextPage"] == true).then(|| page["endCursor"].as_str().map(str::to_string)).flatten();
        Ok((parse_search(&data["search"]).1, next))
    }

    /// Step 2: avatars, labels, comment counts and CI status for those rows
    /// (looked up by id), plus the open and closed totals.
    pub fn enrich(&self, query: &str, mut rows: Vec<PrSummary>) -> Result<ListResult> {
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let q = format!(
            "query($ids: [ID!]!, $open: String!, $closed: String!) {{
              nodes(ids: $ids) {{ ... on PullRequest {{ {EXTRA_FIELDS} }} }}
              open: search(query: $open, type: ISSUE, first: 0) {{ issueCount }}
              closed: search(query: $closed, type: ISSUE, first: 0) {{ issueCount }}
            }}"
        );
        let vars = json!({ "ids": ids, "open": full_query(query, false), "closed": full_query(query, true) });
        let data = self.graphql(&q, vars)?;
        let extras: Vec<Value> = data["nodes"].as_array().cloned().unwrap_or_default();
        for (row, extra) in rows.iter_mut().zip(&extras) {
            if extra["id"].as_str() != Some(row.id.as_str()) {
                continue;
            }
            if let Ok(a) = serde_json::from_value(extra["author"].clone()) {
                row.author = Some(a);
            }
            row.comments = serde_json::from_value(extra["comments"].clone()).unwrap_or_default();
            row.labels = serde_json::from_value(extra["labels"].clone()).unwrap_or_default();
            row.assignees = serde_json::from_value(extra["assignees"].clone()).unwrap_or_else(|_| no_nodes());
            row.review_decision = extra["reviewDecision"].as_str().map(str::to_string);
            row.commits = summary_commits(&extra["commits"]);
        }
        Ok(ListResult {
            open: data["open"]["issueCount"].as_u64().unwrap_or(0),
            closed: data["closed"]["issueCount"].as_u64().unwrap_or(0),
            rows,
            next: None,
        })
    }

    /// One PR by repo and number, with everything a list row shows.
    pub fn pull_request(&self, repo: &str, number: u64) -> Result<PrSummary> {
        let q = format!(
            "query($owner: String!, $name: String!, $n: Int!) {{
              repository(owner: $owner, name: $name) {{ pullRequest(number: $n) {{
                number title url state isDraft createdAt updatedAt closedAt mergedAt repository {{ nameWithOwner }}
                mergeStateStatus autoMergeRequest {{ mergeMethod }}
                {EXTRA_FIELDS} }} }} }}"
        );
        let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
        let data = self.graphql(&q, json!({ "owner": owner, "name": name, "n": number }))?;
        let node = &data["repository"]["pullRequest"];
        if node.is_null() {
            let what = self
                .graphql(
                    "query($owner: String!, $name: String!, $n: Int!) { repository(owner: $owner, name: $name) { issue(number: $n) { id } } }",
                    json!({ "owner": owner, "name": name, "n": number }),
                )
                .ok()
                .filter(|d| !d["repository"]["issue"].is_null());
            return Err(Error::Other(match what {
                Some(_) => format!("{repo}#{number} is an issue, not a pull request."),
                None => format!("No pull request {repo}#{number}, or you don't have access to it."),
            }));
        }
        let mut pr: PrSummary =
            serde_json::from_value(node.clone()).map_err(|e| Error::Other(format!("Unexpected response: {e}")))?;
        pr.commits = summary_commits(&node["commits"]);
        pr.auto_merge = !node["autoMergeRequest"].is_null();
        Ok(pr)
    }

    /// Step 3: merge status for list rows. GitHub computes this per PR and it
    /// is slow (1-3 s for 10), so it runs on its own and never holds up the list.
    /// Returns (id, mergeStateStatus, auto-merge enabled).
    pub fn merge_states(&self, ids: &[String]) -> Result<Vec<(String, Option<String>, bool)>> {
        let q = "query($ids: [ID!]!) { nodes(ids: $ids) { ... on PullRequest {
            id mergeStateStatus autoMergeRequest { mergeMethod } } } }";
        let data = self.graphql(q, json!({ "ids": ids }))?;
        Ok(data["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| {
                        let id = n["id"].as_str()?.to_string();
                        let state = n["mergeStateStatus"].as_str().map(str::to_string);
                        Some((id, state, !n["autoMergeRequest"].is_null()))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn detail(&self, id: &str) -> Result<PrDetail> {
        const Q: &str = r#"
query($id: ID!) { node(id: $id) { ... on PullRequest {
  id number title url body state isDraft createdAt mergedAt
  additions deletions changedFiles baseRefName headRefName mergeable reviewDecision headRefOid
  mergeStateStatus autoMergeRequest { mergeMethod enabledBy { login avatarUrl(size: 80) } }
  viewerCanEnableAutoMerge viewerCanDisableAutoMerge
  latestOpinionatedReviews(first: 30) { nodes { state author { login } } }
  reviewThreads(first: 100) { nodes {
    id isResolved isOutdated path line originalLine viewerCanResolve viewerCanUnresolve viewerCanReply
    resolvedBy { login avatarUrl(size: 80) }
    comments(first: 50) { nodes { author { login avatarUrl(size: 80) } body createdAt url diffHunk pullRequestReview { id } } }
  } }
  author { login avatarUrl(size: 80) }
  repository { nameWithOwner autoMergeAllowed mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed viewerDefaultMergeMethod }
  labels(first: 20) { nodes { name color } }
  assignees(first: 10) { nodes { login avatarUrl(size: 80) } }
  reviewRequests(first: 20) { nodes { requestedReviewer {
    ... on User { login avatarUrl(size: 80) } ... on Team { name } } } }
  comments(first: 100) { nodes { author { login avatarUrl(size: 80) } body createdAt url } }
  reviews(first: 100) { nodes {
    id author { login avatarUrl(size: 80) } body state submittedAt url
  } }
  commits(last: 100) { totalCount nodes { commit {
    oid abbreviatedOid messageHeadline messageBody committedDate
    author { name user { login avatarUrl(size: 80) } }
    statusCheckRollup { state }
    signature { isValid }
  } } }
  timelineItems(last: 100, itemTypes: [MERGED_EVENT, CLOSED_EVENT, REOPENED_EVENT, HEAD_REF_DELETED_EVENT,
      HEAD_REF_FORCE_PUSHED_EVENT, READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT, REVIEW_REQUESTED_EVENT,
      LABELED_EVENT, UNLABELED_EVENT, ASSIGNED_EVENT, AUTO_MERGE_ENABLED_EVENT, AUTO_MERGE_DISABLED_EVENT,
      PULL_REQUEST_COMMIT]) { nodes {
    __typename
    ... on MergedEvent { createdAt actor { login avatarUrl(size: 40) } mergeRefName commit { oid abbreviatedOid } }
    ... on ClosedEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on ReopenedEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on HeadRefDeletedEvent { createdAt actor { login avatarUrl(size: 40) } headRefName }
    ... on HeadRefForcePushedEvent { createdAt actor { login avatarUrl(size: 40) }
      beforeCommit { oid abbreviatedOid } afterCommit { oid abbreviatedOid } }
    ... on ReadyForReviewEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on ConvertToDraftEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on ReviewRequestedEvent { createdAt actor { login avatarUrl(size: 40) }
      requestedReviewer { ... on User { login avatarUrl(size: 40) } ... on Team { name } } }
    ... on LabeledEvent { createdAt actor { login avatarUrl(size: 40) } label { name color } }
    ... on UnlabeledEvent { createdAt actor { login avatarUrl(size: 40) } label { name color } }
    ... on AssignedEvent { createdAt actor { login avatarUrl(size: 40) } assignee { ... on User { login avatarUrl(size: 40) } } }
    ... on AutoMergeEnabledEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on AutoMergeDisabledEvent { createdAt actor { login avatarUrl(size: 40) } }
    ... on PullRequestCommit { commit { oid abbreviatedOid messageHeadline committedDate
      statusCheckRollup { state } author { name user { login avatarUrl(size: 40) } } } }
  } }
  head: commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
    __typename
    ... on CheckRun { name status conclusion detailsUrl title startedAt completedAt isRequired(pullRequestId: $id)
      checkSuite { app { logoUrl(size: 40) } workflowRun { event workflow { name } } } }
    ... on StatusContext { context state targetUrl description avatarUrl(size: 40) isRequired(pullRequestId: $id) }
  } } } } } }
} } }"#;
        let mut data = self.graphql(Q, json!({ "id": id }))?;
        let node = data["node"].take();
        let checks = parse_checks(&node["head"]);
        let mut d: PrDetail = serde_json::from_value(node)
            .map_err(|e| Error::Other(format!("Unexpected response: {e}")))?;
        d.checks = checks;
        Ok(d)
    }

    /// All changed files with their patches. GitHub caps this at 3000 files.
    pub fn files(&self, repo: &str, number: u64) -> Result<Vec<FileDiff>> {
        // Viewed state only exists in GraphQL; fetch it alongside the diffs.
        let (files, viewed) = std::thread::scope(|s| {
            let viewed = s.spawn(|| self.viewed_files(repo, number));
            (self.file_diffs(repo, number), viewed.join())
        });
        let mut files = files?;
        // Missing viewed state shouldn't hide the diff.
        if let Ok(Ok(viewed)) = viewed {
            for f in &mut files {
                f.viewed = viewed.contains(&f.filename);
            }
        }
        Ok(files)
    }

    /// One commit and every file it changed. GitHub sends 300 files per
    /// page, so big commits take a few requests.
    pub fn commit(&self, repo: &str, sha: &str) -> Result<CommitDetail> {
        let mut out: Option<CommitDetail> = None;
        for page in 1..=10 {
            let body = self.rest_get(&format!("/repos/{repo}/commits/{sha}?per_page=300&page={page}"))?;
            let c: CommitDetail = serde_json::from_str(&body).map_err(|e| Error::Other(format!("Unexpected response: {e}")))?;
            let done = c.files.len() < 300;
            match &mut out {
                None => out = Some(c),
                Some(first) => Arc::make_mut(&mut first.files).extend(c.files.iter().cloned()),
            }
            if done {
                break;
            }
        }
        Ok(out.expect("one page at least"))
    }

    fn viewed_files(&self, repo: &str, number: u64) -> Result<std::collections::HashSet<String>> {
        const Q: &str = "query($owner: String!, $name: String!, $n: Int!, $after: String) {
          repository(owner: $owner, name: $name) { pullRequest(number: $n) {
            files(first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { path viewerViewedState } } } } }";
        let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
        let mut out = std::collections::HashSet::new();
        let mut after = Value::Null;
        for _ in 0..30 {
            let data = self.graphql(Q, json!({ "owner": owner, "name": name, "n": number, "after": after }))?;
            let files = &data["repository"]["pullRequest"]["files"];
            for n in files["nodes"].as_array().into_iter().flatten() {
                if n["viewerViewedState"] == "VIEWED" {
                    if let Some(p) = n["path"].as_str() {
                        out.insert(p.to_string());
                    }
                }
            }
            if files["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = files["pageInfo"]["endCursor"].clone();
        }
        Ok(out)
    }

    fn file_diffs(&self, repo: &str, number: u64) -> Result<Vec<FileDiff>> {
        let mut out = Vec::new();
        for page in 1..=30 {
            let body = self.rest_get(&format!(
                "/repos/{repo}/pulls/{number}/files?per_page=100&page={page}"
            ))?;
            let batch: Vec<FileDiff> = serde_json::from_str(&body)
                .map_err(|e| Error::Other(format!("Unexpected response: {e}")))?;
            let done = batch.len() < 100;
            out.extend(batch);
            if done {
                break;
            }
        }
        Ok(out)
    }

    /// Unlike reads, a change fails if GitHub reports any error at all.
    fn mutate(&self, query: &str, vars: Value) -> Result<()> {
        check_writable()?;
        let resp = self
            .agent
            .post(format!("{API}/graphql"))
            .header("Authorization", format!("bearer {}", self.token))
            .send_json(json!({ "query": query, "variables": vars }))?;
        let body = Self::read(resp)?;
        let v: Value = serde_json::from_str(&body).map_err(|e| Error::Other(format!("Bad JSON: {e}")))?;
        match v.get("errors").and_then(|e| e.as_array()) {
            Some(errs) if !errs.is_empty() => {
                let msg: Vec<&str> = errs.iter().filter_map(|e| e["message"].as_str()).collect();
                Err(Error::Other(msg.join("\n")))
            }
            _ => Ok(()),
        }
    }

    pub fn ready_for_review(&self, pr_id: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { clientMutationId } }",
            json!({ "id": pr_id }),
        )
    }

    pub fn close_pull_request(&self, pr_id: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!) { closePullRequest(input: { pullRequestId: $id }) { clientMutationId } }",
            json!({ "id": pr_id }),
        )
    }

    /// Answer in a conversation on a line of code.
    pub fn reply_to_thread(&self, thread_id: &str, body: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!, $body: String!) {
               addPullRequestReviewThreadReply(input: { pullRequestReviewThreadId: $id, body: $body }) { clientMutationId } }",
            json!({ "id": thread_id, "body": body }),
        )
    }

    /// "Resolve conversation" / "Unresolve conversation".
    pub fn set_thread_resolved(&self, thread_id: &str, resolved: bool) -> Result<()> {
        let q = if resolved {
            "mutation($id: ID!) { resolveReviewThread(input: { threadId: $id }) { clientMutationId } }"
        } else {
            "mutation($id: ID!) { unresolveReviewThread(input: { threadId: $id }) { clientMutationId } }"
        };
        self.mutate(q, json!({ "id": thread_id }))
    }

    /// GitHub's emoji list: `name` (as in `:name:`) -> picture URL.
    pub fn emojis(&self) -> Result<std::collections::HashMap<String, String>> {
        let body = self.rest_get("/emojis")?;
        serde_json::from_str(&body).map_err(|e| Error::Other(format!("Unexpected response: {e}")))
    }

    /// The "Viewed" checkbox on a file in Files changed.
    pub fn set_viewed(&self, pr_id: &str, path: &str, viewed: bool) -> Result<()> {
        let q = if viewed {
            "mutation($id: ID!, $path: String!) { markFileAsViewed(input: { pullRequestId: $id, path: $path }) { clientMutationId } }"
        } else {
            "mutation($id: ID!, $path: String!) { unmarkFileAsViewed(input: { pullRequestId: $id, path: $path }) { clientMutationId } }"
        };
        self.mutate(q, json!({ "id": pr_id, "path": path }))
    }

    /// `method` is MERGE, SQUASH or REBASE. `head` makes GitHub refuse the
    /// merge if someone pushed after you looked.
    pub fn merge(&self, pr_id: &str, method: &str, head: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!, $m: PullRequestMergeMethod!, $head: GitObjectID) {
               mergePullRequest(input: { pullRequestId: $id, mergeMethod: $m, expectedHeadOid: $head }) { clientMutationId } }",
            json!({ "id": pr_id, "m": method, "head": if head.is_empty() { Value::Null } else { json!(head) } }),
        )
    }

    pub fn enable_auto_merge(&self, pr_id: &str, method: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!, $m: PullRequestMergeMethod!) {
               enablePullRequestAutoMerge(input: { pullRequestId: $id, mergeMethod: $m }) { clientMutationId } }",
            json!({ "id": pr_id, "m": method }),
        )
    }

    pub fn disable_auto_merge(&self, pr_id: &str) -> Result<()> {
        self.mutate(
            "mutation($id: ID!) { disablePullRequestAutoMerge(input: { pullRequestId: $id }) { clientMutationId } }",
            json!({ "id": pr_id }),
        )
    }

    pub fn comment(&self, repo: &str, number: u64, body: &str) -> Result<()> {
        self.rest_post(&format!("/repos/{repo}/issues/{number}/comments"), json!({ "body": body }))
    }

    /// `event` is APPROVE, REQUEST_CHANGES, or COMMENT.
    pub fn review(&self, repo: &str, number: u64, event: &str, body: &str) -> Result<()> {
        let mut payload = json!({ "event": event });
        if !body.is_empty() {
            payload["body"] = json!(body);
        }
        self.rest_post(&format!("/repos/{repo}/pulls/{number}/reviews"), payload)
    }
}

fn parse_search(v: &Value) -> (u64, Vec<PrSummary>) {
    let count = v["issueCount"].as_u64().unwrap_or(0);
    let rows = v["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                // Skip nodes we can't read (e.g. repos blocked by SSO).
                .filter_map(|n| serde_json::from_value(n.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    (count, rows)
}

fn parse_checks(head: &Value) -> Vec<Check> {
    let nodes = &head["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"];
    let Some(nodes) = nodes.as_array() else { return Vec::new() };
    let s = |v: &Value| v.as_str().map(str::to_string);
    nodes
        .iter()
        .map(|n| {
            let mut c = parse_check(n, s);
            c.required = n["isRequired"].as_bool().unwrap_or(false);
            // policy-bot reports the same failure on every PR, so it's noise.
            if is_policy_bot(&c.name) {
                c.result = "SUCCESS".into();
            }
            c
        })
        .collect()
}

/// The head commit with its overall CI state, except that a failure from
/// policy-bot alone doesn't count, same as the Checks tab. Only overrides
/// GitHub when every check was fetched; otherwise GitHub's word stands.
fn summary_commits(v: &Value) -> Nodes<CommitNode<HeadCommit>> {
    let mut out: Nodes<CommitNode<HeadCommit>> = serde_json::from_value(v.clone()).unwrap_or_default();
    let contexts = &v["nodes"][0]["commit"]["statusCheckRollup"]["contexts"];
    let complete = contexts["pageInfo"]["hasNextPage"] == Value::Bool(false);
    if let (Some(nodes), Some(rollup), true) = (
        contexts["nodes"].as_array(),
        out.nodes.first_mut().and_then(|n| n.commit.status_check_rollup.as_mut()),
        complete,
    ) {
        if matches!(rollup.state.as_str(), "FAILURE" | "ERROR") {
            let s = |v: &Value| v.as_str().map(str::to_string);
            let rank = nodes
                .iter()
                .map(|n| parse_check(n, s))
                .filter(|c| !is_policy_bot(&c.name))
                .map(|c| crate::views::check_rank(&c.result))
                .min();
            rollup.state = match rank {
                Some(0) => "FAILURE",
                Some(1) => "PENDING",
                _ => "SUCCESS",
            }
            .into();
        }
    }
    out
}

pub fn is_policy_bot(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("policy-bot")
}

fn parse_check(n: &Value, s: impl Fn(&Value) -> Option<String>) -> Check {
    if n["__typename"] == "CheckRun" {
        let result = match n["status"].as_str() {
            Some("COMPLETED") => n["conclusion"].as_str().unwrap_or("NEUTRAL"),
            _ => "PENDING",
        };
        let time = |k: &str| n[k].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
        Check {
            name: s(&n["name"]).unwrap_or_default(),
            group: s(&n["checkSuite"]["workflowRun"]["workflow"]["name"]),
            result: result.to_string(),
            required: false,
            description: s(&n["title"]),
            url: s(&n["detailsUrl"]),
            avatar: s(&n["checkSuite"]["app"]["logoUrl"]),
            event: s(&n["checkSuite"]["workflowRun"]["event"]),
            seconds: time("startedAt").zip(time("completedAt")).map(|(a, b)| (b - a).num_seconds()),
        }
    } else {
        let result = match n["state"].as_str() {
            Some("SUCCESS") => "SUCCESS",
            Some("ERROR") | Some("FAILURE") => "FAILURE",
            _ => "PENDING",
        };
        Check {
            name: s(&n["context"]).unwrap_or_default(),
            group: None,
            result: result.to_string(),
            required: false,
            description: s(&n["description"]),
            url: s(&n["targetUrl"]),
            avatar: s(&n["avatarUrl"]),
            event: None,
            seconds: None,
        }
    }
}


