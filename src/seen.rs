//! What you've already looked at, so a PR can show what's new since your
//! last visit. Kept on this computer, in seen.json next to settings.json.

use crate::github::PrDetail;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Seen {
    /// By PR id: the head commit you last looked at, and when.
    pub prs: HashMap<String, Mark>,
    /// Commits you ticked "Reviewed" on, by SHA.
    pub reviewed: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mark {
    pub head: String,
    /// RFC 3339, like GitHub's own times.
    pub at: String,
}

impl Mark {
    pub fn now(head: &str) -> Mark {
        Mark { head: head.to_string(), at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true) }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("github-prs").join("seen.json"))
}

/// A missing or broken file means nothing seen yet.
pub fn load() -> Seen {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write to a temp file and rename, so a crash never leaves half a file.
pub fn save(s: &Seen) {
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let Ok(json) = serde_json::to_vec(s) else { return };
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

fn time(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|t| t.with_timezone(&Utc))
}

/// Where "new since you last looked" starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub head: String,
    pub at: String,
    /// It's the commit your latest review was on, not a visit here.
    pub from_review: bool,
}

/// Your last visit in this app, or your latest review on GitHub (so it
/// works across computers), whichever came later.
pub fn anchor(local: Option<&Mark>, d: &PrDetail, viewer: &str) -> Option<Anchor> {
    let review = d
        .reviews
        .nodes
        .iter()
        .filter(|r| !viewer.is_empty() && r.author.as_ref().is_some_and(|a| a.login == viewer))
        .filter_map(|r| Some((r.commit.as_ref()?.oid.clone(), r.submitted_at.clone()?)))
        .filter(|(oid, _)| !oid.is_empty())
        .max_by_key(|(_, at)| time(at));
    let local = local.map(|m| Anchor { head: m.head.clone(), at: m.at.clone(), from_review: false });
    let review = review.map(|(head, at)| Anchor { head, at, from_review: true });
    match (local, review) {
        (Some(l), Some(r)) => Some(if time(&r.at) > time(&l.at) { r } else { l }),
        (l, r) => l.or(r),
    }
}

/// Something said on the PR after the anchor.
pub struct Said<'a> {
    pub who: &'a str,
    pub avatar: &'a str,
    pub at: &'a str,
    pub body: &'a str,
    /// "commented", "reviewed", or a file path for a comment on code.
    pub what: String,
}

/// What's new on a PR since the anchor.
pub struct News<'a> {
    pub anchor: Anchor,
    /// Commits pushed since, if the anchor is still in the PR's history.
    /// None after a force push (or when it's older than the last 100).
    pub commits: Option<usize>,
    /// The PR's head moved.
    pub moved: bool,
    /// Comments and reviews from other people, oldest first.
    pub said: Vec<Said<'a>>,
}

impl News<'_> {
    pub fn is_empty(&self) -> bool {
        !self.moved && self.said.is_empty()
    }

    /// "3 new commits", "updated", "2 new comments".
    pub fn short(&self) -> Option<String> {
        let plural = |n: usize, w: &str| format!("{n} new {w}{}", if n == 1 { "" } else { "s" });
        match self.commits {
            Some(n) if n > 0 => Some(plural(n, "commit")),
            _ if self.moved => Some("Updated".into()),
            _ if !self.said.is_empty() => Some(plural(self.said.len(), "comment")),
            _ => None,
        }
    }
}

pub fn news<'a>(local: Option<&Mark>, d: &'a PrDetail, viewer: &str) -> Option<News<'a>> {
    let anchor = anchor(local, d, viewer)?;
    let nodes = &d.commits.nodes;
    let commits = nodes.iter().position(|n| n.commit.oid == anchor.head).map(|i| nodes.len() - 1 - i);
    let moved = !d.head_ref_oid.is_empty() && d.head_ref_oid != anchor.head;
    let since = time(&anchor.at);
    let after = |at: &str| since.is_some() && time(at) > since;
    let mine = |who: &str| who == viewer;
    let mut said = Vec::new();
    for c in &d.comments.nodes {
        let who = c.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
        if after(&c.created_at) && !mine(who) {
            let avatar = c.author.as_ref().map(|a| a.avatar_url.as_str()).unwrap_or("");
            said.push(Said { who, avatar, at: &c.created_at, body: &c.body, what: "commented".into() });
        }
    }
    for r in &d.reviews.nodes {
        let who = r.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
        let at = r.submitted_at.as_deref().unwrap_or("");
        // Reviews that only hold code comments show up as those comments.
        let what = match r.state.as_str() {
            "APPROVED" => "approved",
            "CHANGES_REQUESTED" => "requested changes",
            _ if r.body.trim().is_empty() => continue,
            _ => "reviewed",
        };
        if after(at) && !mine(who) {
            let avatar = r.author.as_ref().map(|a| a.avatar_url.as_str()).unwrap_or("");
            said.push(Said { who, avatar, at, body: &r.body, what: what.into() });
        }
    }
    for t in &d.review_threads.nodes {
        for c in &t.comments.nodes {
            let who = c.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
            if after(&c.created_at) && !mine(who) {
                let avatar = c.author.as_ref().map(|a| a.avatar_url.as_str()).unwrap_or("");
                said.push(Said { who, avatar, at: &c.created_at, body: &c.body, what: t.path.clone() });
            }
        }
    }
    said.sort_by_key(|s| time(s.at));
    Some(News { anchor, commits, moved, said })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(json: serde_json::Value) -> PrDetail {
        let mut base = serde_json::json!({
            "id": "PR_1", "number": 1, "title": "t", "url": "u", "body": "", "state": "OPEN", "isDraft": false,
            "createdAt": "2026-01-01T00:00:00Z", "mergedAt": null, "additions": 0, "deletions": 0, "changedFiles": 0,
            "baseRefName": "main", "headRefName": "x", "mergeable": "MERGEABLE", "mergeStateStatus": null,
            "autoMergeRequest": null, "reviewDecision": null, "headRefOid": "c3", "author": null,
            "repository": { "nameWithOwner": "o/r" }, "labels": { "nodes": [] }, "assignees": { "nodes": [] },
            "reviewRequests": { "nodes": [] }, "comments": { "nodes": [] }, "reviews": { "nodes": [] },
            "commits": { "totalCount": 3, "nodes": [
                { "commit": { "oid": "c1", "abbreviatedOid": "c1", "messageHeadline": "a", "committedDate": "2026-01-01T00:00:00Z", "author": null, "statusCheckRollup": null } },
                { "commit": { "oid": "c2", "abbreviatedOid": "c2", "messageHeadline": "b", "committedDate": "2026-01-02T00:00:00Z", "author": null, "statusCheckRollup": null } },
                { "commit": { "oid": "c3", "abbreviatedOid": "c3", "messageHeadline": "c", "committedDate": "2026-01-03T00:00:00Z", "author": null, "statusCheckRollup": null } }
            ] }
        });
        for (k, v) in json.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn counts_commits_since_your_visit() {
        let d = pr(serde_json::json!({}));
        let mark = Mark { head: "c1".into(), at: "2026-01-01T12:00:00Z".into() };
        let n = news(Some(&mark), &d, "me").unwrap();
        assert_eq!(n.commits, Some(2));
        assert!(n.moved);
        assert_eq!(n.short().as_deref(), Some("2 new commits"));
    }

    #[test]
    fn force_push_has_no_count() {
        let d = pr(serde_json::json!({}));
        let mark = Mark { head: "gone".into(), at: "2026-01-01T12:00:00Z".into() };
        let n = news(Some(&mark), &d, "me").unwrap();
        assert_eq!(n.commits, None);
        assert_eq!(n.short().as_deref(), Some("Updated"));
    }

    #[test]
    fn later_review_wins_and_own_comments_dont_count() {
        let d = pr(serde_json::json!({
            "reviews": { "nodes": [{ "author": { "login": "me" }, "body": "", "state": "COMMENTED",
                "submittedAt": "2026-01-05T00:00:00Z", "url": "u", "commit": { "oid": "c3" } }] },
            "comments": { "nodes": [
                { "author": { "login": "me" }, "body": "mine", "createdAt": "2026-01-06T00:00:00Z", "url": "u" },
                { "author": { "login": "you" }, "body": "theirs", "createdAt": "2026-01-06T00:00:00Z", "url": "u" },
                { "author": { "login": "you" }, "body": "old", "createdAt": "2026-01-04T00:00:00Z", "url": "u" }
            ] }
        }));
        let mark = Mark { head: "c1".into(), at: "2026-01-01T12:00:00Z".into() };
        let n = news(Some(&mark), &d, "me").unwrap();
        assert!(n.anchor.from_review);
        assert!(!n.moved);
        assert_eq!(n.said.len(), 1);
        assert_eq!(n.said[0].body, "theirs");
        assert_eq!(n.short().as_deref(), Some("1 new comment"));
    }

    #[test]
    fn nothing_without_an_anchor() {
        assert!(news(None, &pr(serde_json::json!({})), "me").is_none());
    }
}
