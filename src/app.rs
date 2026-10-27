//! App state and the main loop. Network calls run on background threads and
//! send a `Msg` back over a channel; the UI never waits on the network.

use crate::auth::{self, Source};
use crate::github::{self, Client, CommitDetail, Compare, FileDiff, ListResult, PrDetail, PrSummary, SECTIONS};
use crate::{cache, diff, seen, theme, util, views};
use egui::{Key, KeyboardShortcut, Modifiers};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const AUTO_REFRESH: Duration = Duration::from_secs(5 * 60);
/// Details younger than this are not re-fetched when you select a PR again.
const DETAIL_FRESH: Duration = Duration::from_secs(60);
const PREFETCH: usize = github::LIST_SIZE;
const PREFETCH_WORKERS: usize = 3;

pub enum Msg {
    Token(Option<(String, Source)>),
    Viewer(github::Result<String>),
    Orgs(github::Result<Vec<String>>),
    List {
        key: String,
        result: github::Result<ListResult>,
        stale: bool,
    },
    /// Fast first pass of a list: rows without labels, avatars or CI yet.
    ListRows {
        key: String,
        result: github::Result<(Vec<PrSummary>, Option<String>)>,
    },
    /// Merge status for list rows: (id, mergeStateStatus, auto-merge on).
    ListMerge {
        key: String,
        result: github::Result<Vec<(String, Option<String>, bool)>>,
    },
    /// The next page of a list, to add to its end.
    ListMore {
        key: String,
        result: github::Result<ListResult>,
    },
    Detail {
        id: String,
        result: github::Result<PrDetail>,
        stale: bool,
    },
    Files {
        id: String,
        result: github::Result<Vec<FileDiff>>,
        stale: bool,
    },
    /// One commit with its diff, keyed by `CommitPage::key`.
    Commit {
        key: String,
        result: github::Result<CommitDetail>,
    },
    /// The changes between two commits, keyed by `compare_key`. `pr_id` is
    /// the PR they belong to.
    Compare {
        key: String,
        pr_id: String,
        result: github::Result<Compare>,
    },
    Posted {
        id: String,
        result: github::Result<()>,
        what: &'static str,
    },
    /// A PR looked up from the ⌘K box. `seq` drops replies to old requests.
    Found {
        seq: u64,
        result: github::Result<PrSummary>,
        /// Its detail and files are already on the way.
        preloading: bool,
    },
    /// A file's full text, for showing more diff context.
    FileText {
        pr_id: String,
        path: String,
        result: github::Result<String>,
    },
    /// GitHub's emoji list (name -> picture URL).
    Emojis(github::Result<HashMap<String, String>>),
    /// Reply to answering or resolving a review thread.
    Thread {
        pr_id: String,
        thread: String,
        result: github::Result<()>,
        what: &'static str,
    },
    /// Reply to ticking a file's "Viewed" box.
    Viewed {
        id: String,
        path: String,
        viewed: bool,
        result: github::Result<()>,
    },
}

#[derive(Default)]
pub struct GoTo {
    pub text: String,
    pub loading: bool,
    pub error: Option<String>,
    /// The highlighted result (a PR id or "owner/repo#n"), moved with the
    /// arrow keys or the mouse.
    pub sel: Option<String>,
    seq: u64,
}

#[derive(Clone, PartialEq, Debug)]
pub enum View {
    Section(usize),
    Search(String),
}

impl View {
    pub fn query(&self) -> &str {
        match self {
            View::Section(i) => SECTIONS[*i].query,
            View::Search(q) => q,
        }
    }
}

/// Which sidebars are open. Saved between launches.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Panels {
    /// The PR list on the left.
    pub list: bool,
    /// Reviewers, assignees and labels next to the conversation.
    pub details: bool,
    /// The file list next to the diff.
    pub tree: bool,
    /// Diffs show one file at a time instead of all of them.
    pub one_file: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Panels { list: true, details: true, tree: true, one_file: false }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Panel {
    List,
    Details,
    Tree,
}

/// A commit opened from the PR, shown in place of the PR's tabs. With a
/// `base`, it's the changes from `base` to `oid` instead: a range of the
/// PR's commits, or both sides of a force push.
#[derive(Clone, PartialEq, Debug)]
pub struct CommitPage {
    pub repo: String,
    pub oid: String,
    pub base: Option<String>,
    /// The tab to return to.
    pub from: Tab,
}

impl CommitPage {
    pub fn key(&self) -> String {
        match &self.base {
            Some(base) => compare_key(&self.repo, base, &self.oid),
            None => format!("commit-{}-{}", self.repo, self.oid),
        }
    }
}

pub fn compare_key(repo: &str, base: &str, head: &str) -> String {
    format!("compare-{repo}-{base}-{head}")
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Conversation,
    Commits,
    Checks,
    Files,
    /// What changed since you last looked.
    New,
}

/// Data that loads in the background. `data` can be stale (from the disk
/// cache) while a fresh copy is loading.
pub struct Loaded<T> {
    pub data: Option<Arc<T>>,
    pub loading: bool,
    pub error: Option<String>,
    pub fetched: Option<Instant>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Loaded { data: None, loading: false, error: None, fetched: None }
    }
}

impl<T> Loaded<T> {
    fn apply(&mut self, result: github::Result<T>, stale: bool) {
        match result {
            Ok(v) => {
                // Never let a stale cached copy replace fresh data.
                if !(stale && self.fetched.is_some()) {
                    self.data = Some(Arc::new(v));
                }
                if !stale {
                    self.error = None;
                    self.fetched = Some(Instant::now());
                }
            }
            // Cache-only mode: keep what's on disk, no error banner.
            Err(github::Error::Offline) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
        if !stale {
            self.loading = false;
        }
    }
}

/// Things the views ask for. They are applied after the frame is drawn, which
/// keeps the drawing code free of borrow juggling.
pub enum Action {
    SetView(View),
    SetClosed(bool),
    Select(PrSummary),
    Tab(Tab),
    /// Show a commit's page: (repo, sha).
    OpenCommit(String, String),
    /// Show the changes between two commits: (repo, base, head).
    OpenRange(String, String, String),
    /// Tick or untick a commit as reviewed (kept on this computer).
    SetReviewed(String, bool),
    /// Count everything on the selected PR as seen, so New starts over.
    MarkSeen,
    /// Diffs: one file at a time (true), or all of them.
    SetOneFile(bool),
    /// One file at a time: show this file.
    FocusFile(String),
    /// The next (1) or previous (-1) file in the diff on screen.
    StepFile(i64),
    /// The next file you haven't ticked "Viewed" on.
    NextUnviewed,
    /// Tick or untick "Viewed" on the file on screen (v).
    ToggleViewedHere,
    /// On a commit's page: the next (1) or previous (-1) commit.
    StepCommit(i64),
    /// On a commit's page: tick it reviewed and go to the next one that isn't.
    ReviewedNext,
    /// Back from a commit's page to the PR.
    CloseCommit,
    Refresh,
    OpenUrl(String),
    Copy(String),
    Comment,
    Review(&'static str),
    SetMergeMethod(String),
    /// First click on Merge: ask to confirm.
    AskMerge,
    CancelMerge,
    Merge,
    EnableAutoMerge,
    DisableAutoMerge,
    ToggleFile(String),
    /// Run what's typed in the search box (a full GitHub query).
    RunSearch(String),
    /// Close the selected PR without merging.
    ClosePr,
    /// Take the selected PR out of draft.
    ReadyForReview,
    /// Fetch the next page of the list on screen.
    LoadMore,
    SetPreview(bool),
    /// ⌘K: open or close the "Go to pull request" box.
    ToggleGoTo,
    /// Open a pull request link handed to the app by the OS (macOS only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenLink(String),
    /// Open the PR typed in that box.
    GoTo,
    /// Put a comment into the main comment box as a > quote.
    QuoteReply(String),
    /// Open the reply box under a review thread.
    StartReply(String),
    CancelReply(String),
    SendReply(String),
    /// Resolve (true) or unresolve a review thread.
    ResolveThread(String, bool),
    /// Show or hide a resolved thread's comments.
    ToggleThread(String),
    SetViewed(String, bool),
    /// Show more context lines at a diff hunk (by index; one past the last
    /// hunk means the end of the file).
    ExpandHunk(String, usize, u32),
    SubmitToken,
    TogglePanel(Panel),
    /// Look for a gh / environment token again.
    FindToken,
    SignOut,
    OpenSettings,
    /// ⌘?: open or close the help window.
    ToggleHelp,
    /// Leave an owner or "owner/repo" out of every list (true), or bring it back.
    SetIgnored(String, bool),
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    client: Option<Arc<Client>>,
    pub auth_source: Option<Source>,
    pub finding_token: bool,
    pub token_input: String,
    pub token_error: Option<String>,
    pub viewer: String,
    pub closed: bool,
    pub view: View,
    pub search_text: String,
    pub focus_search: bool,
    pub lists: HashMap<String, Loaded<ListResult>>,
    pub selected: Option<PrSummary>,
    pub details: HashMap<String, Loaded<PrDetail>>,
    /// How many times we re-asked GitHub for a PR whose merge status was
    /// still being computed.
    merge_retries: HashMap<String, u32>,
    /// PR ids already queued for prefetch.
    prefetched: HashSet<String>,
    /// Sidebar sections still to load after the visible one arrives.
    pending_sections: bool,
    pub files: HashMap<String, Loaded<Vec<FileDiff>>>,
    pub tab: Tab,
    /// The commit page on screen, if any.
    pub commit: Option<CommitPage>,
    /// Commits opened so far, by `CommitPage::key`.
    pub commits: HashMap<String, Loaded<CommitDetail>>,
    /// Changes between two commits, by `compare_key`.
    pub compares: HashMap<String, Loaded<Compare>>,
    /// What you've looked at, for the New tab and the list's "new" notes.
    pub seen: seen::Seen,
    /// The PR you've looked at code on during this visit, and its head then.
    /// Saved as seen when you move on, so New stays put while you read it.
    visited: Option<(String, String)>,
    /// The New tab's diff on screen: (state key, repo, head).
    pub new_target: Option<(String, String, String)>,
    /// One file at a time: the file shown, by diff state key.
    pub file_focus: HashMap<String, String>,
    /// The first commit of a range, picked with a shift-click.
    pub range_start: Option<String>,
    pub diff_layouts: HashMap<String, Arc<diff::Layout>>,
    /// Files you opened or closed by hand, keyed by (PR id, path). Other
    /// files start closed if they're marked viewed, like on GitHub.
    pub collapsed: HashMap<(String, String), bool>,
    /// "Viewed" changes sent to GitHub but not confirmed yet. Reapplied if
    /// a refresh lands first, so the checkbox doesn't flicker back.
    viewed_pending: HashMap<(String, String), bool>,
    /// The ⌘K "Go to pull request" box, when open.
    pub goto: Option<GoTo>,
    /// settings.json, and whether its window is open.
    pub settings: crate::settings::Settings,
    pub settings_open: bool,
    /// The ⌘? help window: how the app works and every shortcut.
    pub help_open: bool,
    /// Your organizations, once Settings has asked for them.
    pub orgs: Option<Vec<String>>,
    goto_seq: u64,
    /// The window is too narrow for the list and the PR side by side.
    pub narrow: bool,
    /// Whether you opened the list anyway in a narrow window.
    pub list_open_narrow: bool,
    /// Same for the Details sidebar on a narrow page.
    pub details_open_narrow: bool,
    /// The file tree is folded for lack of room, and whether you opened it anyway.
    pub tree_narrow: bool,
    pub tree_open_narrow: bool,
    /// The Details sidebar is hidden, so its toggle sits in the tabs row.
    pub details_folded: bool,
    /// Whether the window is wide enough for the sidebar to stay open.
    pub details_wide: bool,
    /// The PR page is scrolled down, so its header shows slim.
    pub detail_scrolled: bool,
    /// Header heights seen so far, full and slim, so it can fold without a jump.
    pub header_h: [f32; 2],
    /// Last frame's PR page: which one, where its scroll area started, and
    /// how far down it was.
    pub detail_top: Option<(egui::Id, f32, f32)>,
    /// After switching lists, select its first PR once it loads.
    follow_list: bool,
    pub composer: String,
    /// Show the comment box's Markdown rendered instead of the text.
    pub composer_preview: bool,
    /// Move keyboard focus to the comment box next frame.
    pub focus_composer: bool,
    /// Reply drafts under review threads, by thread id.
    pub thread_replies: HashMap<String, String>,
    /// Threads with a request in flight.
    pub busy_threads: HashSet<String>,
    /// Lists fetching their next page.
    pub loading_more: HashSet<String>,
    /// j was pressed on the last row: step onto the next page when it lands.
    advance_on_more: bool,
    /// Resolved threads you expanded.
    pub open_threads: HashSet<String>,
    pub emojis: Arc<crate::gfm::Emojis>,
    /// Full file texts for diff context, by (PR id, path). None while loading.
    file_texts: HashMap<(String, String), Option<Arc<Vec<String>>>>,
    /// Context lines revealed per hunk, by (PR id, path).
    expanded: HashMap<(String, String), HashMap<usize, u32>>,
    /// Markdown after GitHub's extras are rewritten, by body hash.
    pub md_text: HashMap<u64, Arc<String>>,
    pub posting: bool,
    /// MERGE, SQUASH or REBASE, picked in the merge bar.
    pub merge_method: Option<String>,
    /// PR id waiting for "Confirm merge".
    pub confirm_merge: Option<String>,
    pub toast: Option<Toast>,
    pub panels: Panels,
    pub actions: Vec<Action>,
    last_refresh: Instant,
    was_focused: bool,
    started: Instant,
    shot: Option<Screenshot>,
    /// Screenshot runs: scroll the PR page this far down (GITHUB_PRS_SCROLL).
    pub shot_scroll: Option<f32>,
    /// `GITHUB_PRS_PERF=1` prints how long each frame takes to build.
    perf: bool,
}

/// Developer hook: `GITHUB_PRS_SCREENSHOT=out.png` saves a picture of the
/// window after data loads, then quits. Optional: `GITHUB_PRS_TAB`
/// (conversation/commits/checks/files), `GITHUB_PRS_THEME` (light/dark),
/// `GITHUB_PRS_ROW` (which PR to select), `GITHUB_PRS_COLLAPSE`
/// (e.g. `list,details,tree`), `GITHUB_PRS_COMMIT` (first/last/a SHA: open
/// that commit's page).
struct Screenshot {
    path: String,
    requested: bool,
    selected: bool,
    staged: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        crate::images::HttpLoader::install(&cc.egui_ctx);
        crate::images::EmojiLoader::install(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        crate::macos::wake_on_link(&cc.egui_ctx);

        let (tx, rx) = channel();
        let app = App {
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            client: None,
            auth_source: None,
            finding_token: true,
            token_input: String::new(),
            token_error: None,
            viewer: cache::load::<String>("viewer").unwrap_or_default(),
            closed: false,
            view: View::Section(0),
            search_text: String::new(),
            focus_search: false,
            lists: HashMap::new(),
            selected: None,
            details: HashMap::new(),
            prefetched: HashSet::new(),
            merge_retries: HashMap::new(),
            pending_sections: false,
            files: HashMap::new(),
            tab: Tab::Conversation,
            commit: None,
            commits: HashMap::new(),
            compares: HashMap::new(),
            seen: seen::load(),
            visited: None,
            new_target: None,
            file_focus: HashMap::new(),
            range_start: None,
            diff_layouts: HashMap::new(),
            collapsed: HashMap::new(),
            viewed_pending: HashMap::new(),
            goto: None,
            settings: crate::settings::load(),
            settings_open: false,
            help_open: false,
            orgs: None,
            goto_seq: 0,
            follow_list: false,
            narrow: false,
            list_open_narrow: false,
            details_open_narrow: false,
            details_folded: false,
            details_wide: true,
            detail_scrolled: false,
            header_h: [0.0; 2],
            detail_top: None,
            tree_narrow: false,
            tree_open_narrow: false,
            composer: String::new(),
            composer_preview: false,
            focus_composer: false,
            thread_replies: HashMap::new(),
            busy_threads: HashSet::new(),
            loading_more: HashSet::new(),
            advance_on_more: false,
            open_threads: HashSet::new(),
            emojis: Arc::new(cache::load("emojis").map(crate::gfm::Emojis::new).unwrap_or_default()),
            md_text: HashMap::new(),
            file_texts: HashMap::new(),
            expanded: HashMap::new(),
            posting: false,
            merge_method: None,
            confirm_merge: None,
            toast: None,
            panels: cc.storage.and_then(|s| eframe::get_value(s, "panels")).unwrap_or_default(),
            actions: Vec::new(),
            last_refresh: Instant::now(),
            was_focused: true,
            started: Instant::now(),
            perf: std::env::var_os("GITHUB_PRS_PERF").is_some(),
            shot_scroll: std::env::var("GITHUB_PRS_SCREENSHOT")
                .ok()
                .and_then(|_| std::env::var("GITHUB_PRS_SCROLL").ok().map(|v| if v == "bottom" { f32::MAX } else { v.parse().unwrap_or(0.0) })),
            shot: std::env::var("GITHUB_PRS_SCREENSHOT").ok().map(|path| Screenshot { path, requested: false, selected: false, staged: false }),
        };
        let mut app = app;
        if app.shot.is_some() {
            let collapse = std::env::var("GITHUB_PRS_COLLAPSE").unwrap_or_default();
            app.panels = Panels {
                list: !collapse.contains("list"),
                details: !collapse.contains("details"),
                tree: !collapse.contains("tree"),
                one_file: std::env::var_os("GITHUB_PRS_ONE_FILE").is_some(),
            };
        }
        match std::env::var("GITHUB_PRS_THEME").as_deref() {
            Ok("dark") => cc.egui_ctx.set_theme(egui::Theme::Dark),
            Ok("light") => cc.egui_ctx.set_theme(egui::Theme::Light),
            // Follow the OS light/dark setting.
            _ => cc.egui_ctx.set_theme(egui::ThemePreference::System),
        }
        // Finding the token may touch the OS credential store, which can show
        // a permission prompt. Do it off the UI thread.
        let (tx, ctx) = (app.tx.clone(), app.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Token(auth::find_token()));
            ctx.request_repaint();
        });
        app
    }

    /// A new list (section, search, open/closed) starts at the top, like
    /// GitHub, instead of scrolling to wherever the old selection sits.
    fn reset_list_scroll(&mut self) {
        self.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("list-reset-scroll"), true);
            d.insert_temp(egui::Id::new("list-shown-selection"), self.selected.as_ref().map(|s| s.id.clone()));
            d.insert_temp(egui::Id::new("rail-shown-selection"), self.selected.as_ref().map(|s| s.id.clone()).unwrap_or_default());
        });
    }

    /// The next page of the list on screen, added to its end.
    fn load_more(&mut self) {
        let view = self.view.clone();
        let key = self.list_key(&view);
        let Some(after) = self.lists.get(&key).and_then(|l| l.data.as_ref()).and_then(|d| d.next.clone()) else { return };
        if !self.loading_more.insert(key.clone()) {
            return;
        }
        let closed = self.closed;
        let ctx = self.ctx.clone();
        self.spawn(move |c, tx| {
            let result = c.search(view.query(), closed, github::MORE_SIZE, Some(&after)).and_then(|(rows, next)| {
                let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
                let page = c.enrich(view.query(), rows).map(|l| ListResult { next, ..l });
                Ok((page?, ids))
            });
            match result {
                Ok((page, ids)) => {
                    let _ = tx.send(Msg::ListMore { key: key.clone(), result: Ok(page) });
                    ctx.request_repaint();
                    let result = c.merge_states(&ids);
                    let _ = tx.send(Msg::ListMerge { key, result });
                }
                Err(e) => {
                    let _ = tx.send(Msg::ListMore { key, result: Err(e) });
                }
            }
            ctx.request_repaint();
        });
    }

    pub fn list_key(&self, view: &View) -> String {
        // `state_fits` reads the open/closed half back out of this. The ignore
        // list is part of it, so changing that never shows stale rows.
        format!("list-{}-{}{}", self.closed, view.query(), crate::settings::exclusions())
    }

    pub fn current_list(&self) -> Option<&Loaded<ListResult>> {
        self.lists.get(&self.list_key(&self.view))
    }

    /// Run `f` on a background thread with the API client and send its result back.
    fn spawn(&self, f: impl FnOnce(&Client, &Sender<Msg>) + Send + 'static) {
        let Some(client) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            f(&client, &tx);
            ctx.request_repaint();
        });
    }

    fn load_list(&mut self, view: View) {
        let key = self.list_key(&view);
        let entry = self.lists.entry(key.clone()).or_default();
        if entry.loading {
            return;
        }
        entry.loading = true;
        let first_time = entry.data.is_none();
        // A refresh keeps the pages you've already loaded.
        let first = entry.data.as_ref().map_or(github::LIST_SIZE, |d| d.rows.len()).clamp(github::LIST_SIZE, 100);
        let closed = self.closed;
        let ctx = self.ctx.clone();
        self.spawn(move |c, tx| {
            if first_time && let Some(cached) = cache::load::<ListResult>(&key) {
                let _ = tx.send(Msg::List { key: key.clone(), result: Ok(cached), stale: true });
                ctx.request_repaint();
            }
            let (rows, next) = match c.search(view.query(), closed, first, None) {
                Ok(page) => page,
                Err(e) => {
                    let _ = tx.send(Msg::List { key, result: Err(e), stale: false });
                    return;
                }
            };
            let _ = tx.send(Msg::ListRows { key: key.clone(), result: Ok((rows.clone(), next.clone())) });
            ctx.request_repaint();
            let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
            // Merge status is slow, so fetch it alongside the other extras
            // instead of after them.
            std::thread::scope(|s| {
                s.spawn(|| {
                    let result = c.merge_states(&ids);
                    let _ = tx.send(Msg::ListMerge { key: key.clone(), result });
                    ctx.request_repaint();
                });
                let result = c.enrich(view.query(), rows).map(|l| ListResult { next, ..l });
                let _ = tx.send(Msg::List { key: key.clone(), result, stale: false });
                ctx.request_repaint();
            });
        });
    }

    fn load_detail(&mut self, pr: &PrSummary, force: bool) {
        let (id, repo, number) = (pr.id.clone(), pr.repository.name_with_owner.clone(), pr.number);
        let d = self.details.entry(id.clone()).or_default();
        let fresh = d.fetched.is_some_and(|t| t.elapsed() < DETAIL_FRESH);
        if !d.loading && (force || !fresh) {
            d.loading = true;
            let first_time = d.data.is_none();
            let (id, ctx) = (id.clone(), self.ctx.clone());
            self.spawn(move |c, tx| {
                let key = format!("detail-{id}");
                if first_time && let Some(cached) = cache::load::<PrDetail>(&key) {
                    let _ = tx.send(Msg::Detail { id: id.clone(), result: Ok(cached), stale: true });
                    ctx.request_repaint();
                }
                let result = c.detail(&id);
                if let Ok(d) = &result {
                    cache::store(&key, d);
                }
                let _ = tx.send(Msg::Detail { id, result, stale: false });
            });
        }
        let f = self.files.entry(id.clone()).or_default();
        let fresh = f.fetched.is_some_and(|t| t.elapsed() < DETAIL_FRESH);
        if !f.loading && (force || !fresh) {
            f.loading = true;
            let first_time = f.data.is_none();
            let ctx = self.ctx.clone();
            self.spawn(move |c, tx| {
                let key = format!("files-{id}");
                if first_time && let Some(cached) = cache::load::<Vec<FileDiff>>(&key) {
                    let _ = tx.send(Msg::Files { id: id.clone(), result: Ok(cached), stale: true });
                    ctx.request_repaint();
                }
                let result = c.files(&repo, number);
                if let Ok(f) = &result {
                    cache::store(&key, f);
                }
                let _ = tx.send(Msg::Files { id, result, stale: false });
            });
        }
    }

    /// Quietly load the first few PRs of a list so clicking them is instant.
    /// A few workers share one queue, so prefetching never floods GitHub,
    /// and PRs you click are fetched directly instead of waiting in line.
    fn prefetch(&mut self, rows: &[PrSummary]) {
        let todo: std::collections::VecDeque<PrSummary> =
            rows.iter().take(PREFETCH).filter(|p| !self.details.contains_key(&p.id) && self.prefetched.insert(p.id.clone())).cloned().collect();
        if todo.is_empty() {
            return;
        }
        let queue = Arc::new(std::sync::Mutex::new(todo));
        for _ in 0..PREFETCH_WORKERS {
            let queue = queue.clone();
            self.spawn(move |c, tx| {
                while let Some(pr) = queue.lock().unwrap().pop_front() {
                    let result = c.detail(&pr.id);
                    if let Ok(d) = &result {
                        cache::store(&format!("detail-{}", pr.id), d);
                    }
                    let _ = tx.send(Msg::Detail { id: pr.id.clone(), result, stale: false });
                    let result = c.files(&pr.repository.name_with_owner, pr.number);
                    if let Ok(f) = &result {
                        cache::store(&format!("files-{}", pr.id), f);
                    }
                    let _ = tx.send(Msg::Files { id: pr.id, result, stale: false });
                }
            });
        }
    }

    fn refresh_all(&mut self) {
        self.last_refresh = Instant::now();
        // The list on screen first; the other sections once it arrives.
        self.load_list(self.view.clone());
        self.pending_sections = true;
        if let Some(pr) = self.selected.clone() {
            self.load_detail(&pr, true);
        }
    }

    fn set_client(&mut self, token: String, source: Source) {
        self.client = Some(Arc::new(Client::new(token)));
        self.auth_source = Some(source);
        self.token_error = None;
        self.spawn(|c, tx| {
            let _ = tx.send(Msg::Viewer(c.viewer()));
        });
        self.spawn(|c, tx| {
            let _ = tx.send(Msg::Emojis(c.emojis()));
        });
        self.refresh_all();
    }

    fn on_error(&mut self, e: &github::Error) {
        if let github::Error::Unauthorized(m) = e {
            // The saved token is bad (expired or revoked). Forget it and ask again.
            if !matches!(self.auth_source, Some(Source::Env)) {
                auth::forget_token();
            }
            self.client = None;
            self.token_error = Some(m.clone());
        }
    }

    fn handle(&mut self, msg: Msg) {
        if self.perf {
            let (what, stale) = match &msg {
                Msg::Token(_) => ("token", false),
                Msg::Viewer(_) => ("viewer", false),
                Msg::Orgs(_) => ("orgs", false),
                Msg::List { stale, .. } => ("list", *stale),
                Msg::ListRows { .. } => ("list rows", false),
                Msg::ListMore { .. } => ("more rows", false),
                Msg::ListMerge { .. } => ("list merge", false),
                Msg::Detail { stale, .. } => ("detail", *stale),
                Msg::Files { stale, .. } => ("files", *stale),
                Msg::Commit { .. } => ("commit", false),
                Msg::Compare { .. } => ("compare", false),
                Msg::Posted { .. } => ("posted", false),
                Msg::Viewed { .. } => ("viewed", false),
                Msg::Found { .. } => ("found", false),
                Msg::Emojis(_) => ("emojis", false),
                Msg::FileText { .. } => ("file text", false),
                Msg::Thread { .. } => ("thread", false),
            };
            eprintln!("event {what}{} at {} ms", if stale { " (cache)" } else { "" }, self.started.elapsed().as_millis());
        }
        match msg {
            Msg::Token(found) => {
                self.finding_token = false;
                if let Some((t, src)) = found {
                    self.set_client(t, src);
                }
            }
            Msg::Viewer(r) => match r {
                Ok(v) => {
                    // Remembered, so your avatar and "you wrote this" survive
                    // a rate limit or a bad connection.
                    cache::store("viewer", &v);
                    self.viewer = v;
                }
                Err(e) => {
                    if self.viewer.is_empty() {
                        self.viewer = cache::load::<String>("viewer").unwrap_or_default();
                    }
                    self.on_error(&e);
                }
            },
            Msg::List { key, result, stale } => {
                if let Err(e) = &result {
                    self.on_error(e);
                }
                let entry = self.lists.entry(key.clone()).or_default();
                let old = entry.data.clone();
                let result = result.map(|mut l| {
                    l.rows.retain(|r| state_fits(&key, &r.state));
                    l
                });
                entry.apply(result, stale);
                if let (Some(old), Some(new)) = (old, entry.data.as_mut()) {
                    // The merge status arrives separately; don't lose it.
                    let new = Arc::make_mut(new);
                    for r in &mut new.rows {
                        if r.merge_state_status.is_none()
                            && let Some(o) = old.rows.iter().find(|o| o.id == r.id)
                        {
                            r.merge_state_status = o.merge_state_status.clone();
                            r.auto_merge = o.auto_merge;
                        }
                    }
                }
                if !stale && let Some(d) = &entry.data {
                    let (key, d) = (key.clone(), d.clone());
                    std::thread::spawn(move || cache::store(&key, &*d));
                }
                let rows = entry.data.clone();
                // The open and closed lists share one pair of totals, so the
                // counts don't change when you switch between them.
                if let Some(d) = rows.as_ref().filter(|_| !stale) {
                    let other = key.replacen("list-false-", "list-true-", 1);
                    let other = if other == key { key.replacen("list-true-", "list-false-", 1) } else { other };
                    if let Some(data) = self.lists.get_mut(&other).and_then(|e| e.data.as_mut()) {
                        let data = Arc::make_mut(data);
                        (data.open, data.closed) = (d.open, d.closed);
                    }
                }
                if !stale && key == self.list_key(&self.view) {
                    if let Some(r) = rows {
                        self.prefetch(&r.rows);
                    }
                    if std::mem::take(&mut self.pending_sections) {
                        for i in 0..SECTIONS.len() {
                            self.load_list(View::Section(i));
                        }
                    }
                }
            }
            Msg::ListRows { key, result } => {
                let Ok((mut rows, next)) = result else { return };
                rows.retain(|r| state_fits(&key, &r.state));
                let entry = self.lists.entry(key.clone()).or_default();
                // Keep labels, avatars and CI we already have for these PRs,
                // so a refresh doesn't make them blink.
                let (mut open, mut closed) = (0, 0);
                if let Some(old) = &entry.data {
                    (open, closed) = (old.open, old.closed);
                    for r in &mut rows {
                        if let Some(o) = old.rows.iter().find(|o| o.id == r.id) {
                            r.author = o.author.clone().or(r.author.take());
                            r.comments = o.comments.clone();
                            r.labels = o.labels.clone();
                            r.commits = o.commits.clone();
                            r.merge_state_status = o.merge_state_status.clone();
                            r.auto_merge = o.auto_merge;
                            r.assignees = o.assignees.clone();
                            r.review_decision = o.review_decision.clone();
                        }
                    }
                }
                entry.data = Some(Arc::new(ListResult { open, closed, rows: rows.clone(), next }));
                if key == self.list_key(&self.view) {
                    self.prefetch(&rows);
                }
            }
            Msg::ListMore { key, result } => {
                self.loading_more.remove(&key);
                let page = match result {
                    Ok(page) => page,
                    Err(e) => {
                        self.on_error(&e);
                        return;
                    }
                };
                let Some(data) = self.lists.get_mut(&key).and_then(|e| e.data.as_mut()) else { return };
                let data = Arc::make_mut(data);
                for r in page.rows {
                    if state_fits(&key, &r.state) && !data.rows.iter().any(|o| o.id == r.id) {
                        data.rows.push(r);
                    }
                }
                (data.open, data.closed, data.next) = (page.open, page.closed, page.next);
                let d = data.clone();
                let current = key == self.list_key(&self.view);
                std::thread::spawn(move || cache::store(&key, &d));
                if std::mem::take(&mut self.advance_on_more) && current {
                    self.move_selection(1);
                }
            }
            Msg::ListMerge { key, result } => {
                let Ok(states) = result else { return };
                let Some(data) = self.lists.get_mut(&key).and_then(|e| e.data.as_mut()) else { return };
                let data = Arc::make_mut(data);
                for (id, state, auto) in states {
                    if let Some(r) = data.rows.iter_mut().find(|r| r.id == id) {
                        r.merge_state_status = state;
                        r.auto_merge = auto;
                    }
                }
                let d = data.clone();
                std::thread::spawn(move || cache::store(&key, &d));
            }
            Msg::Detail { id, result, stale } => {
                if let Err(e) = &result {
                    self.on_error(e);
                }
                // GitHub computes merge status lazily: the first answer is
                // often UNKNOWN while it works it out. Ask again shortly.
                let unknown = result
                    .as_ref()
                    .ok()
                    .filter(|d| d.state == "OPEN")
                    .is_some_and(|d| matches!(d.merge_state_status.as_deref(), None | Some("UNKNOWN")) || d.mergeable == "UNKNOWN");
                if !stale && unknown {
                    let tries = self.merge_retries.entry(id.clone()).or_insert(0);
                    *tries += 1;
                    if *tries <= 3 {
                        let (id, wait) = (id.clone(), Duration::from_secs(2 * *tries as u64));
                        self.spawn(move |c, tx| {
                            std::thread::sleep(wait);
                            let result = c.detail(&id);
                            if let Ok(d) = &result {
                                cache::store(&format!("detail-{id}"), d);
                            }
                            let _ = tx.send(Msg::Detail { id, result, stale: false });
                        });
                    }
                } else if !stale {
                    self.merge_retries.remove(&id);
                }
                self.details.entry(id).or_default().apply(result, stale);
            }
            Msg::Files { id, mut result, stale } => {
                if let Err(e) = &result {
                    self.on_error(e);
                }
                if let Ok(files) = &mut result {
                    crate::diff::sort_like_tree(files);
                }
                self.drop_layouts(&id);
                self.files.entry(id.clone()).or_default().apply(result, stale);
                let pending: Vec<(String, bool)> = self.viewed_pending.iter().filter(|((i, _), _)| *i == id).map(|((_, p), v)| (p.clone(), *v)).collect();
                for (path, v) in pending {
                    self.patch_viewed(&id, &path, v);
                }
            }
            Msg::Commit { key, mut result } => {
                if let Err(e) = &result {
                    self.on_error(e);
                }
                if let Ok(c) = &mut result {
                    crate::diff::sort_like_tree(Arc::make_mut(&mut c.files).as_mut_slice());
                    cache::store(&key, c);
                }
                self.drop_layouts(&key);
                self.commits.entry(key).or_default().apply(result, false);
            }
            Msg::Compare { key, pr_id, mut result } => {
                if let Err(e) = &result {
                    self.on_error(e);
                }
                if let Ok(c) = &mut result {
                    // After a rebase the comparison also holds the base
                    // branch's changes. Keep only files this PR touches.
                    if c.status == "diverged"
                        && let Some(pr_files) = self.files.get(&pr_id).and_then(|f| f.data.clone())
                    {
                        Arc::make_mut(&mut c.files).retain(|f| pr_files.iter().any(|p| p.filename == f.filename));
                    }
                    cache::store(&key, c);
                }
                self.drop_layouts(&key);
                self.compares.entry(key).or_default().apply(result, false);
            }
            Msg::FileText { pr_id, path, result } => match result {
                Ok(text) => {
                    let lines: Vec<String> = text.lines().map(str::to_string).collect();
                    self.file_texts.insert((pr_id.clone(), path), Some(Arc::new(lines)));
                    self.drop_layouts(&pr_id);
                }
                Err(e) => {
                    self.file_texts.remove(&(pr_id.clone(), path.clone()));
                    self.expanded.remove(&(pr_id.clone(), path));
                    self.drop_layouts(&pr_id);
                    self.show_toast(format!("Couldn't load the file: {e}"), true);
                }
            },
            Msg::Orgs(result) => match result {
                Ok(orgs) => self.orgs = Some(orgs),
                Err(e) => {
                    self.orgs = Some(Vec::new());
                    self.show_toast(format!("Couldn't load your organizations: {e}"), true);
                }
            },
            Msg::Emojis(result) => {
                // Not worth an error message: emoji just stay as text.
                if let Ok(map) = result {
                    cache::store("emojis", &map);
                    self.emojis = Arc::new(crate::gfm::Emojis::new(map));
                    self.md_text.clear();
                }
            }
            Msg::Thread { pr_id, thread, result, what } => {
                self.busy_threads.remove(&thread);
                match result {
                    Ok(()) => {
                        if what == "Reply" {
                            self.thread_replies.remove(&thread);
                        }
                        let done = match what {
                            "Reply" => "Reply posted",
                            "Resolve" => "Conversation resolved",
                            _ => "Conversation unresolved",
                        };
                        self.show_toast(done.into(), false);
                        if let Some(pr) = self.selected.clone().filter(|p| p.id == pr_id) {
                            self.load_detail(&pr, true);
                        }
                    }
                    Err(e) => self.show_toast(format!("{what} failed: {e}"), true),
                }
            }
            Msg::Found { seq, result, preloading } => {
                let Some(g) = self.goto.as_mut().filter(|g| g.seq == seq) else { return };
                match result {
                    Ok(pr) => {
                        self.goto = None;
                        if preloading {
                            // So selecting it doesn't fetch them a second time.
                            self.details.entry(pr.id.clone()).or_default().loading = true;
                            self.files.entry(pr.id.clone()).or_default().loading = true;
                        }
                        self.actions.push(Action::Select(pr));
                    }
                    Err(e) => {
                        g.loading = false;
                        g.error = Some(e.to_string());
                        self.on_error(&e);
                    }
                }
            }
            Msg::Viewed { id, path, viewed, result } => {
                let k = (id.clone(), path.clone());
                if self.viewed_pending.get(&k) == Some(&viewed) {
                    self.viewed_pending.remove(&k);
                }
                match result {
                    Ok(()) => {
                        if let Some(f) = self.files.get(&id).and_then(|f| f.data.clone()) {
                            cache::store(&format!("files-{id}"), &*f);
                        }
                    }
                    Err(e) => {
                        self.patch_viewed(&id, &path, !viewed);
                        // Undo the fold too, so the file matches its checkbox.
                        self.collapsed.insert((id.clone(), path.clone()), !viewed);
                        self.show_toast(format!("Couldn't mark the file as viewed — {e}"), true);
                    }
                }
            }
            Msg::Posted { id, result, what } => {
                self.posting = false;
                match result {
                    Ok(()) => {
                        if matches!(what, "Comment" | "Approval" | "Change request" | "Review") {
                            self.composer.clear();
                            self.composer_preview = false;
                            self.show_toast(format!("{what} submitted"), false);
                        } else {
                            self.show_toast(what.to_string(), false);
                        }
                        if let Some(pr) = self.selected.clone().filter(|p| p.id == id) {
                            self.load_detail(&pr, true);
                        }
                        self.load_list(self.view.clone());
                    }
                    Err(e) => self.show_toast(e.to_string(), true),
                }
            }
        }
    }

    /// The current list's query as GitHub would show it in its search box.
    pub fn display_query(&self) -> String {
        format!("is:pr {} {}", if self.closed { "is:closed" } else { "is:open" }, self.view.query())
    }

    /// Keep the detail pane in step with the list after switching lists.
    fn follow_list(&mut self) {
        if !self.follow_list {
            return;
        }
        let Some(rows) = self.current_list().and_then(|l| l.data.clone()) else { return };
        self.follow_list = false;
        if self.selected.as_ref().is_some_and(|s| rows.rows.iter().any(|r| r.id == s.id)) {
            return;
        }
        match rows.rows.first() {
            Some(first) => self.actions.push(Action::Select(first.clone())),
            None => self.selected = None,
        }
    }

    /// The repo a bare PR number refers to: the open PR's, else the list's.
    pub fn default_repo(&self) -> Option<String> {
        let pr = self.selected.as_ref().or_else(|| self.current_list()?.data.as_ref()?.rows.first());
        pr.map(|p| p.repository.name_with_owner.clone())
    }

    /// What the diff on screen belongs to: its state key, repo, and the
    /// commit its files are read from. A commit page when one is open,
    /// otherwise the selected PR.
    fn diff_target(&self) -> Option<(String, String, String)> {
        if let Some(c) = &self.commit {
            return Some((c.key(), c.repo.clone(), c.oid.clone()));
        }
        if self.tab == Tab::New {
            return self.new_target.clone();
        }
        let pr = self.selected.as_ref()?;
        let head = self.details.get(&pr.id).and_then(|d| d.data.as_ref()).map(|d| d.head_ref_oid.clone()).unwrap_or_default();
        Some((pr.id.clone(), pr.repository.name_with_owner.clone(), head))
    }

    fn open_commit(&mut self, repo: String, oid: String, base: Option<String>) {
        let from = self.commit.as_ref().map(|c| c.from).unwrap_or(self.tab);
        let page = CommitPage { repo, oid, base, from };
        let key = page.key();
        self.commit = Some(page.clone());
        self.note_visit();
        if let Some(base) = &page.base {
            self.ensure_compare(&page.repo, base, &page.oid);
            return;
        }
        let entry = self.commits.entry(key.clone()).or_default();
        if entry.data.is_some() || entry.loading {
            return;
        }
        // Commits never change, so a cached copy is as good as a fresh one.
        if let Some(cached) = cache::load::<CommitDetail>(&key) {
            entry.apply(Ok(cached), false);
            return;
        }
        entry.loading = true;
        self.spawn(move |c, tx| {
            let result = c.commit(&page.repo, &page.oid);
            let _ = tx.send(Msg::Commit { key, result });
        });
    }

    /// Start loading the changes from `base` to `head`, unless they're here
    /// already. Returns their state key.
    pub fn ensure_compare(&mut self, repo: &str, base: &str, head: &str) -> String {
        let key = compare_key(repo, base, head);
        let entry = self.compares.entry(key.clone()).or_default();
        if entry.data.is_some() || entry.loading || entry.error.is_some() {
            return key;
        }
        // Commits never change, so neither does what's between two of them.
        if let Some(cached) = cache::load::<Compare>(&key) {
            entry.apply(Ok(cached), false);
            return key;
        }
        entry.loading = true;
        let pr_id = self.selected.as_ref().map(|p| p.id.clone()).unwrap_or_default();
        let (repo, base, head, k) = (repo.to_string(), base.to_string(), head.to_string(), key.clone());
        self.spawn(move |c, tx| {
            let result = c.compare(&repo, &base, &head);
            let _ = tx.send(Msg::Compare { key: k, pr_id, result });
        });
        key
    }

    /// Try a failed comparison again.
    pub fn retry_compare(&mut self, key: &str) {
        self.compares.remove(key);
    }

    /// Forget the drawn diff for a state key, including its one-file views.
    fn drop_layouts(&mut self, key: &str) {
        let prefix = format!("{key}\0");
        self.diff_layouts.retain(|k, _| k != key && !k.starts_with(&prefix));
    }

    /// The files of the diff with this state key: a PR's, a commit's, or a comparison's.
    pub fn diff_files(&self, key: &str) -> Option<Arc<Vec<FileDiff>>> {
        if let Some(f) = self.files.get(key).and_then(|f| f.data.as_ref()) {
            return Some(f.clone());
        }
        if let Some(c) = self.commits.get(key).and_then(|c| c.data.as_ref()) {
            return Some(c.files.clone());
        }
        self.compares.get(key).and_then(|c| c.data.as_ref()).map(|c| c.files.clone())
    }

    /// The file on screen in a diff: the one shown alone, or the one at the top.
    pub fn current_file(&self, key: &str) -> Option<usize> {
        let files = self.diff_files(key)?;
        if self.panels.one_file {
            return Some(self.file_focus.get(key).and_then(|p| files.iter().position(|f| &f.filename == p)).unwrap_or_else(|| first_unviewed(&files)));
        }
        self.ctx.data(|d| d.get_temp::<usize>(egui::Id::new(("diff-current", key))))
    }

    /// Show file `i` of a diff: on its own, or scrolled to the top.
    fn go_to_file(&mut self, key: &str, i: usize) {
        let Some(files) = self.diff_files(key) else { return };
        let Some(f) = files.get(i) else { return };
        if self.panels.one_file {
            self.file_focus.insert(key.to_string(), f.filename.clone());
        } else if let Some(top) = self.diff_layouts.get(key).and_then(|l| l.file_top(i)) {
            self.ctx.data_mut(|d| d.insert_temp(egui::Id::new(("diff-jump", key)), top));
        }
    }

    /// The selected PR's head commit, once its details are in.
    fn selected_head(&self) -> Option<(String, String)> {
        let pr = self.selected.as_ref()?;
        let head = self.details.get(&pr.id)?.data.as_ref()?.head_ref_oid.clone();
        (!head.is_empty()).then(|| (pr.id.clone(), head))
    }

    /// You're looking at the selected PR's code.
    pub fn note_visit(&mut self) {
        if let Some(v) = self.selected_head() {
            self.visited = Some(v);
        }
    }

    /// Save the visit as seen, now that you've moved on.
    fn end_visit(&mut self) {
        if let Some((id, head)) = self.visited.take() {
            self.seen.prs.insert(id, seen::Mark::now(&head));
            seen::save(&self.seen);
        }
    }

    /// What's new on a PR since you last looked, if you've looked before.
    pub fn news<'a>(&self, d: &'a PrDetail) -> Option<seen::News<'a>> {
        seen::news(self.seen.prs.get(&d.id), d, &self.viewer)
    }

    pub fn expansion(&self, pr_id: &str, path: &str) -> diff::Expansion {
        let key = (pr_id.to_string(), path.to_string());
        (self.file_texts.get(&key).cloned().flatten(), self.expanded.get(&key).cloned().unwrap_or_default())
    }

    pub fn is_collapsed(&self, pr_id: &str, path: &str) -> bool {
        if let Some(&c) = self.collapsed.get(&(pr_id.to_string(), path.to_string())) {
            return c;
        }
        let files = self.files.get(pr_id).and_then(|f| f.data.as_ref());
        files.is_some_and(|fs| fs.iter().any(|f| f.filename == path && f.viewed))
    }

    /// Change one file's viewed flag in the loaded data and redraw the diff.
    fn patch_viewed(&mut self, pr_id: &str, path: &str, viewed: bool) {
        let Some(data) = self.files.get_mut(pr_id).and_then(|f| f.data.as_mut()) else { return };
        if let Some(f) = Arc::make_mut(data).iter_mut().find(|f| f.filename == path) {
            f.viewed = viewed;
            f.stale_view = false;
        }
        self.drop_layouts(pr_id);
    }

    pub fn show_toast(&mut self, text: String, error: bool) {
        let secs = if error { 8 } else { 3 };
        self.toast = Some(Toast { text, error, until: Instant::now() + Duration::from_secs(secs) });
    }

    fn post(&mut self, what: &'static str, event: Option<&'static str>) {
        let Some(pr) = self.selected.clone() else { return };
        let body = self.composer.trim().to_string();
        if event.is_none() && body.is_empty() {
            return;
        }
        if event == Some("REQUEST_CHANGES") && body.is_empty() {
            self.show_toast("Add a comment explaining the changes you're requesting.".into(), true);
            return;
        }
        self.posting = true;
        self.spawn(move |c, tx| {
            let repo = &pr.repository.name_with_owner;
            let result = match event {
                None => c.comment(repo, pr.number, &body),
                Some(ev) => c.review(repo, pr.number, ev, &body),
            };
            let _ = tx.send(Msg::Posted { id: pr.id, result, what });
        });
    }

    fn thread_action(&mut self, thread: String, what: &'static str, f: impl FnOnce(&Client, &str) -> github::Result<()> + Send + 'static) {
        let Some(pr) = self.selected.clone() else { return };
        self.spawn(move |c, tx| {
            let result = f(c, &thread);
            let _ = tx.send(Msg::Thread { pr_id: pr.id, thread, result, what });
        });
    }

    /// GitHub-flavored Markdown turned into what the viewer understands.
    /// Cached, since it runs for every comment on every frame.
    pub fn markdown_text(&mut self, body: &str, repo: &str) -> Arc<String> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (body, repo).hash(&mut h);
        let emojis = self.emojis.clone();
        self.md_text.entry(h.finish()).or_insert_with(|| Arc::new(crate::gfm::to_markdown(&util::clean_markdown(body), repo, &emojis))).clone()
    }

    /// Run a merge-related request for the selected PR in the background.
    fn merge_action(&mut self, done: &'static str, f: impl FnOnce(&Client, &PrDetail) -> github::Result<()> + Send + 'static) {
        let Some(pr) = self.selected.clone() else { return };
        let Some(d) = self.details.get(&pr.id).and_then(|d| d.data.clone()) else { return };
        self.posting = true;
        self.confirm_merge = None;
        self.spawn(move |c, tx| {
            let result = f(c, &d);
            let _ = tx.send(Msg::Posted { id: pr.id, result, what: done });
        });
    }

    /// The merge method to use: your pick, else the repo's default for you.
    pub fn chosen_merge_method(&self, d: &PrDetail) -> String {
        let allowed = merge_methods(d);
        if let Some(m) = self.merge_method.as_ref().filter(|m| allowed.contains(&m.as_str())) {
            return m.clone();
        }
        d.repository
            .viewer_default_merge_method
            .clone()
            .filter(|m| allowed.contains(&m.as_str()))
            .or(allowed.first().map(|m| m.to_string()))
            .unwrap_or_else(|| "MERGE".into())
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::SetView(v) => {
                self.view = v.clone();
                self.follow_list = true;
                self.reset_list_scroll();
                let stale = self.lists.get(&self.list_key(&v)).is_none_or(|l| l.fetched.is_none());
                if stale || matches!(v, View::Search(_)) {
                    self.load_list(v);
                } else if let Some(r) = self.current_list().and_then(|l| l.data.clone()) {
                    self.prefetch(&r.rows);
                }
            }
            Action::SetClosed(c) => {
                self.closed = c;
                self.follow_list = true;
                self.reset_list_scroll();
                self.load_list(self.view.clone());
            }
            Action::Select(pr) => {
                self.load_detail(&pr, false);
                if self.selected.as_ref().map(|p| &p.id) != Some(&pr.id) {
                    self.end_visit();
                    self.composer.clear();
                    self.commit = None;
                    self.range_start = None;
                    // New belongs to one PR; another PR may have nothing new.
                    if self.tab == Tab::New {
                        self.tab = Tab::Conversation;
                    }
                }
                self.selected = Some(pr);
            }
            Action::Tab(t) => {
                self.tab = t;
                self.commit = None;
                if matches!(t, Tab::Files | Tab::New) {
                    self.note_visit();
                }
            }
            Action::OpenCommit(repo, oid) => self.open_commit(repo, oid, None),
            Action::OpenRange(repo, base, head) => self.open_commit(repo, head, Some(base)),
            Action::SetReviewed(oid, on) => {
                if on {
                    self.seen.reviewed.insert(oid);
                } else {
                    self.seen.reviewed.remove(&oid);
                }
                seen::save(&self.seen);
            }
            Action::MarkSeen => {
                self.visited = None;
                if let Some((id, head)) = self.selected_head() {
                    self.seen.prs.insert(id, seen::Mark::now(&head));
                    seen::save(&self.seen);
                    self.tab = Tab::Conversation;
                    self.show_toast("Marked as seen".into(), false);
                }
            }
            Action::SetOneFile(on) => {
                // Stay on the file you were looking at.
                let here = self.diff_target().and_then(|(key, _, _)| Some((self.current_file(&key)?, key)));
                self.panels.one_file = on;
                if let Some((i, key)) = here {
                    self.go_to_file(&key, i);
                }
            }
            Action::FocusFile(path) => {
                if let Some((key, _, _)) = self.diff_target() {
                    self.file_focus.insert(key, path);
                }
            }
            Action::StepFile(delta) => {
                let Some((key, _, _)) = self.diff_target() else { return };
                let Some(n) = self.diff_files(&key).map(|f| f.len()).filter(|n| *n > 0) else { return };
                let at = self.current_file(&key).unwrap_or(0) as i64;
                self.go_to_file(&key, (at + delta).clamp(0, n as i64 - 1) as usize);
            }
            Action::NextUnviewed => {
                let Some((key, _, _)) = self.diff_target() else { return };
                let Some(files) = self.diff_files(&key) else { return };
                let at = self.current_file(&key).unwrap_or(0);
                if let Some(i) = next_unviewed(&files, at) {
                    self.go_to_file(&key, i);
                }
            }
            Action::ToggleViewedHere => {
                let Some((key, _, _)) = self.diff_target() else { return };
                // Viewed belongs to the PR's own diff only.
                if self.commit.is_some() || self.tab != Tab::Files {
                    return;
                }
                let Some(files) = self.diff_files(&key) else { return };
                let Some(i) = self.current_file(&key) else { return };
                let Some(f) = files.get(i) else { return };
                let viewed = !f.viewed;
                self.apply(Action::SetViewed(f.filename.clone(), viewed), ctx);
                // Done with this one: on to the next, like GitHub's review flow.
                if viewed
                    && self.panels.one_file
                    && let Some(next) = next_unviewed(&files, i)
                {
                    self.go_to_file(&key, next);
                }
            }
            Action::StepCommit(delta) => {
                let Some(page) = self.commit.clone().filter(|c| c.base.is_none()) else { return };
                let Some(d) = self.selected.as_ref().and_then(|p| self.details.get(&p.id)).and_then(|d| d.data.clone()) else { return };
                let nodes = &d.commits.nodes;
                let Some(at) = nodes.iter().position(|n| n.commit.oid == page.oid) else { return };
                let to = at as i64 + delta;
                if to >= 0 && (to as usize) < nodes.len() {
                    self.open_commit(page.repo, nodes[to as usize].commit.oid.clone(), None);
                }
            }
            Action::ReviewedNext => {
                let Some(page) = self.commit.clone().filter(|c| c.base.is_none()) else { return };
                self.seen.reviewed.insert(page.oid.clone());
                seen::save(&self.seen);
                let Some(d) = self.selected.as_ref().and_then(|p| self.details.get(&p.id)).and_then(|d| d.data.clone()) else { return };
                let nodes = &d.commits.nodes;
                let at = nodes.iter().position(|n| n.commit.oid == page.oid).unwrap_or(0);
                let next = nodes.iter().skip(at + 1).chain(nodes.iter().take(at)).find(|n| !self.seen.reviewed.contains(&n.commit.oid));
                match next {
                    Some(n) => self.open_commit(page.repo, n.commit.oid.clone(), None),
                    None => {
                        self.show_toast("All commits reviewed".into(), false);
                        self.apply(Action::CloseCommit, ctx);
                    }
                }
            }
            Action::CloseCommit => {
                if let Some(c) = self.commit.take() {
                    self.tab = c.from;
                }
            }
            Action::Refresh => self.refresh_all(),
            Action::OpenSettings => {
                self.settings_open = true;
                if self.orgs.is_none() {
                    self.orgs = Some(Vec::new());
                    self.spawn(|c, tx| {
                        let _ = tx.send(Msg::Orgs(c.orgs()));
                    });
                }
            }
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::SetIgnored(entry, on) => {
                self.settings.set_ignored(&entry, on);
                if let Err(e) = crate::settings::save(&self.settings) {
                    self.show_toast(format!("Couldn't save settings: {e}"), true);
                }
                self.reset_list_scroll();
                self.refresh_all();
            }
            Action::OpenUrl(u) => util::open_url(&u),
            Action::Copy(s) => {
                ctx.copy_text(s);
                self.show_toast("Copied to clipboard".into(), false);
            }
            Action::Comment => self.post("Comment", None),
            Action::Review(ev) => {
                let what = match ev {
                    "APPROVE" => "Approval",
                    "REQUEST_CHANGES" => "Change request",
                    _ => "Review",
                };
                self.post(what, Some(ev));
            }
            Action::SetMergeMethod(m) => {
                self.merge_method = Some(m);
                self.confirm_merge = None;
            }
            Action::AskMerge => self.confirm_merge = self.selected.as_ref().map(|p| p.id.clone()),
            Action::CancelMerge => self.confirm_merge = None,
            Action::Merge => {
                let Some(d) = self.selected.as_ref().and_then(|p| self.details.get(&p.id)).and_then(|d| d.data.clone()) else { return };
                let method = self.chosen_merge_method(&d);
                self.merge_action("Pull request merged", move |c, d| c.merge(&d.id, &method, &d.head_ref_oid));
            }
            Action::EnableAutoMerge => {
                let Some(d) = self.selected.as_ref().and_then(|p| self.details.get(&p.id)).and_then(|d| d.data.clone()) else { return };
                let method = self.chosen_merge_method(&d);
                self.merge_action("Auto-merge enabled", move |c, d| c.enable_auto_merge(&d.id, &method));
            }
            Action::DisableAutoMerge => self.merge_action("Auto-merge disabled", |c, d| c.disable_auto_merge(&d.id)),
            Action::ToggleFile(name) => {
                if let Some((key, _, _)) = self.diff_target() {
                    let now = self.is_collapsed(&key, &name);
                    self.collapsed.insert((key.clone(), name), !now);
                    self.drop_layouts(&key);
                }
            }
            Action::QuoteReply(body) => {
                let quoted: String = body.trim().lines().map(|l| format!("> {l}\n")).collect();
                if !self.composer.is_empty() && !self.composer.ends_with("\n\n") {
                    self.composer.push_str(if self.composer.ends_with('\n') { "\n" } else { "\n\n" });
                }
                self.composer.push_str(&quoted);
                self.composer.push('\n');
                self.composer_preview = false;
                self.focus_composer = true;
            }
            Action::StartReply(t) => {
                self.thread_replies.entry(t).or_default();
            }
            Action::CancelReply(t) => {
                self.thread_replies.remove(&t);
            }
            Action::SendReply(t) => {
                let body = self.thread_replies.get(&t).map(|b| b.trim().to_string()).unwrap_or_default();
                if body.is_empty() || !self.busy_threads.insert(t.clone()) {
                    return;
                }
                self.thread_action(t, "Reply", move |c, id| c.reply_to_thread(id, &body));
            }
            Action::ResolveThread(t, resolve) => {
                if !self.busy_threads.insert(t.clone()) {
                    return;
                }
                let what = if resolve { "Resolve" } else { "Unresolve" };
                self.thread_action(t, what, move |c, id| c.set_thread_resolved(id, resolve));
            }
            Action::ToggleThread(t) => {
                if !self.open_threads.remove(&t) {
                    self.open_threads.insert(t);
                }
            }
            Action::RunSearch(q) => {
                // "is:pr is:open author:@me" -> Created, open. Anything else is a search.
                let closed = q.split_whitespace().any(|t| t == "is:closed" || t == "is:merged");
                let rest: Vec<&str> = q.split_whitespace().filter(|t| !matches!(*t, "is:pr" | "is:open" | "is:closed" | "archived:false")).collect();
                let rest = rest.join(" ");
                let view = match SECTIONS.iter().position(|s| s.query == rest) {
                    Some(i) => View::Section(i),
                    None if rest.is_empty() => View::Section(0),
                    None => View::Search(rest),
                };
                self.closed = closed;
                self.apply(Action::SetView(view), ctx);
            }
            Action::LoadMore => self.load_more(),
            Action::ReadyForReview => {
                self.merge_action("Marked ready for review", |c, d| c.ready_for_review(&d.id));
            }
            Action::SetPreview(v) => self.composer_preview = v,
            Action::ClosePr => {
                self.merge_action("Pull request closed", |c, d| c.close_pull_request(&d.id));
            }
            Action::OpenLink(url) => {
                // Same path as typing the link in the go-to box, which also
                // shows why if it isn't a PR link.
                self.goto = Some(GoTo { text: url, ..Default::default() });
                self.apply(Action::GoTo, ctx);
            }
            Action::ToggleGoTo => {
                self.goto = if self.goto.is_some() { None } else { Some(GoTo::default()) };
            }
            Action::GoTo => {
                let default_repo = self.default_repo();
                let Some(g) = self.goto.as_mut() else { return };
                if g.loading {
                    return;
                }
                let Some((repo, number)) = util::parse_pr_ref(&g.text, default_repo.as_deref()) else {
                    g.error = Some("Type a PR number, owner/repo#123, or a link to a pull request.".into());
                    return;
                };
                // Already loaded? Open it without asking GitHub.
                let known = self
                    .lists
                    .values()
                    .filter_map(|l| l.data.as_ref())
                    .flat_map(|l| l.rows.iter())
                    .find(|p| p.number == number && p.repository.name_with_owner.eq_ignore_ascii_case(&repo));
                if let Some(pr) = known.cloned() {
                    self.goto = None;
                    self.actions.push(Action::Select(pr));
                    return;
                }
                self.goto_seq += 1;
                let seq = self.goto_seq;
                g.seq = seq;
                g.loading = true;
                g.error = None;
                let ctx = self.ctx.clone();
                self.spawn(move |c, tx| {
                    // Fetch the PR page and its files alongside the lookup,
                    // not after it: the page query alone takes ~2 seconds.
                    std::thread::scope(|s| {
                        let detail = s.spawn(|| c.detail_by_number(&repo, number));
                        let files = s.spawn(|| c.files(&repo, number));
                        let result = c.pull_request(&repo, number);
                        let id = result.as_ref().ok().map(|pr| pr.id.clone());
                        let _ = tx.send(Msg::Found { seq, result, preloading: id.is_some() });
                        ctx.request_repaint();
                        let Some(id) = id else { return };
                        let failed = || github::Error::Other("Loading failed.".into());
                        let result = detail.join().unwrap_or_else(|_| Err(failed()));
                        if let Ok(d) = &result {
                            cache::store(&format!("detail-{id}"), d);
                        }
                        let _ = tx.send(Msg::Detail { id: id.clone(), result, stale: false });
                        ctx.request_repaint();
                        let result = files.join().unwrap_or_else(|_| Err(failed()));
                        if let Ok(f) = &result {
                            cache::store(&format!("files-{id}"), f);
                        }
                        let _ = tx.send(Msg::Files { id, result, stale: false });
                    });
                });
            }
            Action::ExpandHunk(path, idx, amount) => {
                let Some((id, repo, head)) = self.diff_target() else { return };
                let key = (id.clone(), path.clone());
                *self.expanded.entry(key.clone()).or_default().entry(idx).or_default() += amount;
                self.drop_layouts(&id);
                if let std::collections::hash_map::Entry::Vacant(e) = self.file_texts.entry(key) {
                    if head.is_empty() {
                        return;
                    }
                    e.insert(None);
                    self.spawn(move |c, tx| {
                        let result = c.file_text(&repo, &path, &head);
                        let _ = tx.send(Msg::FileText { pr_id: id, path, result });
                    });
                }
            }
            Action::SetViewed(path, viewed) => {
                let Some(pr) = self.selected.clone() else { return };
                // Like GitHub: marking a file viewed folds it away.
                self.collapsed.insert((pr.id.clone(), path.clone()), viewed);
                self.viewed_pending.insert((pr.id.clone(), path.clone()), viewed);
                self.patch_viewed(&pr.id, &path, viewed);
                self.spawn(move |c, tx| {
                    let result = c.set_viewed(&pr.id, &path, viewed);
                    let _ = tx.send(Msg::Viewed { id: pr.id, path, viewed, result });
                });
            }
            Action::SubmitToken => {
                let t = self.token_input.trim().to_string();
                if t.is_empty() {
                    return;
                }
                if let Err(e) = auth::save_token(&t) {
                    self.show_toast(format!("Couldn't save token: {e}"), true);
                }
                self.token_input.clear();
                self.set_client(t, Source::Pasted);
            }
            Action::TogglePanel(which) => {
                let open = match which {
                    Panel::List if self.narrow => &mut self.list_open_narrow,
                    Panel::List => &mut self.panels.list,
                    Panel::Details => &mut self.panels.details,
                    Panel::Tree if self.tree_narrow => &mut self.tree_open_narrow,
                    Panel::Tree => &mut self.panels.tree,
                };
                *open = !*open;
            }
            Action::FindToken => {
                self.finding_token = true;
                self.token_error = None;
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::Token(auth::find_token()));
                    ctx.request_repaint();
                });
            }
            Action::SignOut => {
                auth::forget_token();
                self.client = None;
                self.token_error = None;
                self.viewer.clear();
            }
        }
    }

    fn move_selection(&mut self, delta: i64) {
        let Some(rows) = self.current_list().and_then(|l| l.data.clone()) else { return };
        if rows.rows.is_empty() {
            return;
        }
        let cur = self.selected.as_ref().and_then(|s| rows.rows.iter().position(|r| r.id == s.id));
        // j on the last row fetches the next page.
        if delta > 0 && cur == Some(rows.rows.len() - 1) && rows.next.is_some() {
            self.actions.push(Action::LoadMore);
            self.advance_on_more = true;
        }
        let next = match cur {
            Some(i) => (i as i64 + delta).clamp(0, rows.rows.len() as i64 - 1) as usize,
            None => 0,
        };
        self.actions.push(Action::Select(rows.rows[next].clone()));
        self.ctx.data_mut(|d| d.insert_temp(egui::Id::new("scroll-to-row"), next));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Only a text box swallows plain keys; a Tab-focused button doesn't.
        let typing_text = ctx.text_edit_focused() || self.goto.is_some();
        // Keys behind the help window shouldn't move the list.
        let typing = typing_text || self.help_open;
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
        let mut acts = Vec::new();
        let mut close = false;
        ctx.input_mut(|i| {
            if i.consume_shortcut(&cmd(Key::R)) {
                acts.push(Action::Refresh);
            }
            // ⌘W and Ctrl+W both close the window, on every OS.
            if i.consume_shortcut(&cmd(Key::W)) || i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::W)) {
                close = true;
            }
            // ⌘B: PR list. ⇧⌘B: the side panel of the current tab.
            if i.consume_shortcut(&cmd_shift(Key::B)) {
                acts.push(Action::TogglePanel(if matches!(self.tab, Tab::Files | Tab::New) || self.commit.is_some() { Panel::Tree } else { Panel::Details }));
            }
            if i.consume_shortcut(&cmd(Key::B)) {
                acts.push(Action::TogglePanel(Panel::List));
            }
            for (k, t) in [(Key::Num1, Tab::Conversation), (Key::Num2, Tab::Commits), (Key::Num3, Tab::Checks), (Key::Num4, Tab::Files), (Key::Num5, Tab::New)]
            {
                if i.consume_shortcut(&cmd(k)) {
                    acts.push(Action::Tab(t));
                }
            }
            if i.consume_shortcut(&cmd(Key::F)) {
                self.focus_search = true;
            }
            if i.consume_shortcut(&cmd(Key::K)) {
                acts.push(Action::ToggleGoTo);
            }
            // ⌘? is ⇧⌘/ on most keyboards; the OS reports either key.
            if i.consume_shortcut(&cmd(Key::Questionmark)) || i.consume_shortcut(&cmd(Key::Slash)) {
                acts.push(Action::ToggleHelp);
            }
            if let Some(pr) = &self.selected {
                if i.consume_shortcut(&cmd(Key::O)) {
                    acts.push(Action::OpenUrl(pr.url.clone()));
                }
                if i.consume_shortcut(&cmd_shift(Key::C)) {
                    acts.push(Action::Copy(pr.url.clone()));
                }
            }
        });
        // Must happen after input_mut returns: egui holds a lock inside it,
        // and sending a command there waits on that same lock forever.
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // A plain ? opens help too, and closes it again.
        if !typing_text && ctx.input(|i| i.key_pressed(Key::Questionmark) || (i.modifiers.shift_only() && i.key_pressed(Key::Slash))) {
            acts.push(Action::ToggleHelp);
        }
        if !typing {
            let (down, up, slash, copy, escape) = ctx.input(|i| {
                let plain = i.modifiers.is_none();
                (
                    i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::J),
                    i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::K),
                    !i.modifiers.shift && i.key_pressed(Key::Slash),
                    plain && i.key_pressed(Key::C),
                    plain && i.key_pressed(Key::Escape),
                )
            });
            // Review keys, while a diff is on screen: n / p next and previous
            // file, u next unviewed, v viewed (on a commit: reviewed, then
            // the next commit), [ and ] previous and next commit.
            let on_diff = self.commit.is_some() || matches!(self.tab, Tab::Files | Tab::New);
            if on_diff && self.selected.is_some() {
                let (next, prev, unviewed, viewed, back, fwd) = ctx.input(|i| {
                    let plain = i.modifiers.is_none();
                    (
                        plain && i.key_pressed(Key::N),
                        plain && i.key_pressed(Key::P),
                        plain && i.key_pressed(Key::U),
                        plain && i.key_pressed(Key::V),
                        i.key_pressed(Key::OpenBracket),
                        i.key_pressed(Key::CloseBracket),
                    )
                });
                let on_commit = self.commit.as_ref().is_some_and(|c| c.base.is_none());
                for (hit, act) in [
                    (next, Action::StepFile(1)),
                    (prev, Action::StepFile(-1)),
                    (unviewed, Action::NextUnviewed),
                    (viewed && on_commit, Action::ReviewedNext),
                    (viewed && !on_commit, Action::ToggleViewedHere),
                    (back, Action::StepCommit(-1)),
                    (fwd, Action::StepCommit(1)),
                ] {
                    if hit {
                        self.actions.push(act);
                    }
                }
            }
            // Esc leaves a commit page, unless it's closing the floating
            // file tree or the ⌘K box first.
            if escape && self.commit.is_some() && self.goto.is_none() && !(self.tree_narrow && self.tree_open_narrow) {
                self.actions.push(Action::CloseCommit);
            }
            if copy && let Some(pr) = &self.selected {
                self.actions.push(Action::Copy(pr.url.clone()));
            }
            if down {
                self.move_selection(1);
            }
            if up {
                self.move_selection(-1);
            }
            if slash {
                self.focus_search = true;
            }
        }
        self.actions.extend(acts);
    }
}

impl App {
    /// Screenshot runs: data has loaded and the picture can be taken.
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub fn shot_ready(&self) -> bool {
        self.shot.as_ref().is_some_and(|s| s.requested)
    }

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let list_ready = self.current_list().is_some_and(|l| l.data.is_some());
        let Some(shot) = &mut self.shot else { return };
        ctx.request_repaint_after(Duration::from_millis(200));
        let rows = self.lists.get(&format!("list-{}-{}", self.closed, self.view.query())).and_then(|l| l.data.clone());
        if !shot.selected {
            // GITHUB_PRS_GOTO opens a PR through the ⌘K box;
            // GITHUB_PRS_GOTO_TYPE just shows the box with that text.
            if let Ok(text) = std::env::var("GITHUB_PRS_GOTO") {
                shot.selected = true;
                self.tab = match std::env::var("GITHUB_PRS_TAB").as_deref() {
                    Ok("commits") => Tab::Commits,
                    Ok("checks") => Tab::Checks,
                    Ok("files") => Tab::Files,
                    Ok("new") => Tab::New,
                    _ => Tab::Conversation,
                };
                self.goto = Some(GoTo { text, ..Default::default() });
                self.actions.push(Action::GoTo);
            } else if let Ok(text) = std::env::var("GITHUB_PRS_GOTO_TYPE")
                && list_ready
            {
                shot.selected = true;
                self.goto = Some(GoTo { text, ..Default::default() });
            }
        }
        if !shot.selected
            && let Some(rows) = rows.filter(|r| !r.rows.is_empty())
        {
            let i: usize = std::env::var("GITHUB_PRS_ROW").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            let pr = rows.rows[i.min(rows.rows.len() - 1)].clone();
            self.tab = match std::env::var("GITHUB_PRS_TAB").as_deref() {
                Ok("commits") => Tab::Commits,
                Ok("checks") => Tab::Checks,
                Ok("files") => Tab::Files,
                Ok("new") => Tab::New,
                _ => Tab::Conversation,
            };
            shot.selected = true;
            self.actions.push(Action::Select(pr));
        }
        let typing_only = self.goto.is_some() && std::env::var("GITHUB_PRS_GOTO_TYPE").is_ok();
        let loaded = typing_only
            || self.selected.as_ref().is_some_and(|s| {
                self.details.get(&s.id).is_some_and(|d| d.data.is_some() && !d.loading)
                    && !self.merge_retries.contains_key(&s.id)
                    && self.files.get(&s.id).is_some_and(|f| f.data.is_some())
            });
        // GITHUB_PRS_STATE=reply,quote,preview,open-resolved sets up those
        // views once the PR loads. Local only: nothing is sent to GitHub.
        if loaded && !shot.staged {
            shot.staged = true;
            let state = std::env::var("GITHUB_PRS_STATE").unwrap_or_default();
            let d = self.selected.as_ref().and_then(|s| self.details.get(&s.id)).and_then(|d| d.data.clone());
            if let Some(d) = d {
                // GITHUB_PRS_COMMIT=first|last|<sha> opens that commit's page.
                if let Ok(which) = std::env::var("GITHUB_PRS_COMMIT") {
                    let nodes = &d.commits.nodes;
                    let oid = match which.as_str() {
                        "first" => nodes.first().map(|n| n.commit.oid.clone()),
                        "last" => nodes.last().map(|n| n.commit.oid.clone()),
                        sha => Some(sha.to_string()),
                    };
                    if let Some(oid) = oid {
                        self.actions.push(Action::OpenCommit(d.repository.name_with_owner.clone(), oid));
                    }
                }
                if state.contains("reply")
                    && let Some(t) = d.review_threads.nodes.iter().find(|t| t.viewer_can_reply)
                {
                    self.thread_replies.insert(t.id.clone(), "Good catch, fixed in the next commit.".into());
                }
                if state.contains("quote") {
                    let body = d.comments.nodes.first().map(|c| c.body.clone()).unwrap_or_else(|| d.body.clone());
                    self.actions.push(Action::QuoteReply(body.lines().take(3).collect::<Vec<_>>().join("\n")));
                }
                if state.contains("preview") {
                    if self.composer.is_empty() && !state.contains("quote") {
                        self.composer = "Looks good :tada: 🚀 thanks @octocat, see #1 and https://github.com".into();
                    }
                    // After any quote reply, which switches back to Write.
                    self.actions.push(Action::SetPreview(true));
                }
                if state.contains("open-resolved") {
                    self.open_threads.extend(d.review_threads.nodes.iter().filter(|t| t.is_resolved).map(|t| t.id.clone()));
                }
            }
        }
        let commit_ready = self.commit.as_ref().is_none_or(|c| self.commits.get(&c.key()).is_some_and(|l| l.data.is_some()));
        let loaded = loaded && commit_ready && (shot.staged || std::env::var("GITHUB_PRS_COMMIT").is_err());
        let settle = self.started.elapsed() > Duration::from_secs(4);
        let timeout = self.started.elapsed() > Duration::from_secs(45);
        if !shot.requested && ((loaded && settle) || timeout) {
            // Give avatars a moment to arrive.
            if self.started.elapsed() > Duration::from_secs(if loaded { 6 } else { 0 }) {
                shot.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }
        let path = shot.path.clone();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let _ = image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Merge methods this repo allows, in GitHub's menu order.
pub fn merge_methods(d: &PrDetail) -> Vec<&'static str> {
    let r = &d.repository;
    let mut m = Vec::new();
    if r.merge_commit_allowed {
        m.push("MERGE");
    }
    if r.squash_merge_allowed {
        m.push("SQUASH");
    }
    if r.rebase_merge_allowed {
        m.push("REBASE");
    }
    if m.is_empty() {
        // Older cached data doesn't have the repo settings yet.
        m = vec!["MERGE", "SQUASH", "REBASE"];
    }
    m
}

pub fn merge_method_label(m: &str) -> &'static str {
    match m {
        "SQUASH" => "Squash and merge",
        "REBASE" => "Rebase and merge",
        _ => "Create a merge commit",
    }
}

impl eframe::App for App {
    // Screenshot runs shouldn't change your saved theme, sizes or layout.
    fn persist_egui_memory(&self) -> bool {
        self.shot.is_none()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Screenshot runs shouldn't change your saved layout.
        if self.shot.is_none() {
            eframe::set_value(storage, "panels", &self.panels);
        }
    }

    fn on_exit(&mut self) {
        if self.shot.is_none() {
            self.end_visit();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        crate::macos::place_traffic_lights(frame);
        let ctx = ui.ctx().clone();
        let frame_start = Instant::now();
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }

        // Refresh every few minutes, and when you come back to the window.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let back = focused && !self.was_focused && self.last_refresh.elapsed() > Duration::from_secs(60);
        self.was_focused = focused;
        if self.client.is_some() && (back || self.last_refresh.elapsed() > AUTO_REFRESH) {
            self.refresh_all();
        }
        ctx.request_repaint_after(AUTO_REFRESH);

        if self.client.is_some() {
            // Links wait in the queue until you've signed in.
            #[cfg(target_os = "macos")]
            self.actions.extend(crate::macos::take_links().into_iter().map(Action::OpenLink));
            self.follow_list();
            self.shortcuts(&ctx);
            views::main(self, ui);
            views::goto_box(self, &ctx);
            views::settings_box(self, &ctx);
            views::help_box(self, &ctx);
        } else {
            views::sign_in(self, ui);
        }
        views::toast(self, &ctx);
        focus_ring(&ctx);

        for a in std::mem::take(&mut self.actions) {
            self.apply(a, &ctx);
        }
        self.screenshot_hook(&ctx);

        // Route link clicks (from markdown and elsewhere) through our own
        // opener, which uses each OS's native API.
        let urls = ctx.output_mut(|o| {
            let mut urls = Vec::new();
            o.commands.retain(|c| match c {
                egui::OutputCommand::OpenUrl(u) => {
                    urls.push(u.url.clone());
                    false
                }
                _ => true,
            });
            urls
        });
        for u in urls {
            util::open_url(&u);
        }
        if self.perf {
            let _ = frame;
            let ms = frame_start.elapsed().as_secs_f64() * 1000.0;
            eprintln!("frame {:>5.1} ms  tab={:?}", ms, self.tab);
        }
    }
}

/// The first file you haven't ticked "Viewed" on, else the first file.
pub fn first_unviewed(files: &[FileDiff]) -> usize {
    files.iter().position(|f| !f.viewed).unwrap_or(0)
}

/// The next file after `at` you haven't ticked "Viewed" on, wrapping around.
pub fn next_unviewed(files: &[FileDiff], at: usize) -> Option<usize> {
    let n = files.len();
    (1..n).map(|k| (at + k) % n).find(|&i| !files[i].viewed)
}

/// Draw Primer's focus ring on the focused control. Our buttons paint
/// their own fills, so egui's focus look never shows; this covers them all.
/// Clicks don't focus buttons in egui, so the ring only follows Tab.
/// Text inputs are skipped: they draw their own focused border.
fn focus_ring(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    if ctx.text_edit_focused() {
        return;
    }
    let Some(resp) = ctx.read_response(id) else { return };
    let p = theme::palette(ctx);
    let rect = resp.interact_rect;
    if !rect.is_positive() {
        return;
    }
    // The widget's own layer, so the ring sits right above it, under
    // anything floating on top.
    ctx.layer_painter(resp.layer_id).rect_stroke(rect, theme::RADIUS, theme::focus_ring(p), egui::StrokeKind::Inside);
}

/// GitHub's search can list a PR that was closed a moment ago under
/// "Open" (and the reverse). Rows that don't fit their list are dropped.
fn state_fits(key: &str, state: &str) -> bool {
    if key.starts_with("list-false-") {
        state == "OPEN"
    } else if key.starts_with("list-true-") {
        state != "OPEN"
    } else {
        true
    }
}
