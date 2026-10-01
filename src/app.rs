//! App state and the main loop. Network calls run on background threads and
//! send a `Msg` back over a channel; the UI never waits on the network.

use crate::auth::{self, Source};
use crate::github::{self, Client, FileDiff, ListResult, PrDetail, PrSummary, SECTIONS};
use crate::{cache, diff, theme, util, views};
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
    List { key: String, result: github::Result<ListResult>, stale: bool },
    /// Fast first pass of a list: rows without labels, avatars or CI yet.
    ListRows { key: String, result: github::Result<Vec<PrSummary>> },
    /// Merge status for list rows: (id, mergeStateStatus, auto-merge on).
    ListMerge { key: String, result: github::Result<Vec<(String, Option<String>, bool)>> },
    Detail { id: String, result: github::Result<PrDetail>, stale: bool },
    Files { id: String, result: github::Result<Vec<FileDiff>>, stale: bool },
    Posted { id: String, result: github::Result<()>, what: &'static str },
    /// A PR looked up from the ⌘K box. `seq` drops replies to old requests.
    Found { seq: u64, result: github::Result<PrSummary> },
    /// A file's full text, for showing more diff context.
    FileText { pr_id: String, path: String, result: github::Result<String> },
    /// GitHub's emoji list (name -> picture URL).
    Emojis(github::Result<HashMap<String, String>>),
    /// Reply to answering or resolving a review thread.
    Thread { pr_id: String, thread: String, result: github::Result<()>, what: &'static str },
    /// Reply to ticking a file's "Viewed" box.
    Viewed { id: String, path: String, viewed: bool, result: github::Result<()> },
}

#[derive(Default)]
pub struct GoTo {
    pub text: String,
    pub loading: bool,
    pub error: Option<String>,
    /// The highlighted result, moved with the arrow keys or the mouse.
    pub sel: usize,
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
}

impl Default for Panels {
    fn default() -> Self {
        Panels { list: true, details: true, tree: true }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Panel {
    List,
    Details,
    Tree,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Conversation,
    Commits,
    Checks,
    Files,
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
    SetPreview(bool),
    /// ⌘K: open or close the "Go to pull request" box.
    ToggleGoTo,
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
    pub diff_layouts: HashMap<String, Arc<diff::Layout>>,
    /// Files you opened or closed by hand, keyed by (PR id, path). Other
    /// files start closed if they're marked viewed, like on GitHub.
    pub collapsed: HashMap<(String, String), bool>,
    /// "Viewed" changes sent to GitHub but not confirmed yet. Reapplied if
    /// a refresh lands first, so the checkbox doesn't flicker back.
    viewed_pending: HashMap<(String, String), bool>,
    /// The ⌘K "Go to pull request" box, when open.
    pub goto: Option<GoTo>,
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
/// (e.g. `list,details,tree`).
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
            diff_layouts: HashMap::new(),
            collapsed: HashMap::new(),
            viewed_pending: HashMap::new(),
            goto: None,
            goto_seq: 0,
            follow_list: false,
            narrow: false,
            list_open_narrow: false,
            details_open_narrow: false,
            details_folded: false,
            details_wide: true,
            detail_scrolled: false,
            tree_narrow: false,
            tree_open_narrow: false,
            composer: String::new(),
            composer_preview: false,
            focus_composer: false,
            thread_replies: HashMap::new(),
            busy_threads: HashSet::new(),
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
            shot_scroll: std::env::var("GITHUB_PRS_SCREENSHOT").ok().and_then(|_| {
                std::env::var("GITHUB_PRS_SCROLL").ok().map(|v| if v == "bottom" { f32::MAX } else { v.parse().unwrap_or(0.0) })
            }),
            shot: std::env::var("GITHUB_PRS_SCREENSHOT")
                .ok()
                .map(|path| Screenshot { path, requested: false, selected: false, staged: false }),
        };
        let mut app = app;
        if app.shot.is_some() {
            let collapse = std::env::var("GITHUB_PRS_COLLAPSE").unwrap_or_default();
            app.panels = Panels {
                list: !collapse.contains("list"),
                details: !collapse.contains("details"),
                tree: !collapse.contains("tree"),
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

    pub fn list_key(&self, view: &View) -> String {
        format!("list-{}-{}", self.closed, view.query())
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
        let closed = self.closed;
        let ctx = self.ctx.clone();
        self.spawn(move |c, tx| {
            if first_time {
                if let Some(cached) = cache::load::<ListResult>(&key) {
                    let _ = tx.send(Msg::List { key: key.clone(), result: Ok(cached), stale: true });
                    ctx.request_repaint();
                }
            }
            let rows = match c.search(view.query(), closed) {
                Ok(rows) => rows,
                Err(e) => {
                    let _ = tx.send(Msg::List { key, result: Err(e), stale: false });
                    return;
                }
            };
            let _ = tx.send(Msg::ListRows { key: key.clone(), result: Ok(rows.clone()) });
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
                let result = c.enrich(view.query(), rows);
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
                if first_time {
                    if let Some(cached) = cache::load::<PrDetail>(&key) {
                        let _ = tx.send(Msg::Detail { id: id.clone(), result: Ok(cached), stale: true });
                        ctx.request_repaint();
                    }
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
                if first_time {
                    if let Some(cached) = cache::load::<Vec<FileDiff>>(&key) {
                        let _ = tx.send(Msg::Files { id: id.clone(), result: Ok(cached), stale: true });
                        ctx.request_repaint();
                    }
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
        let todo: std::collections::VecDeque<PrSummary> = rows
            .iter()
            .take(PREFETCH)
            .filter(|p| !self.details.contains_key(&p.id) && self.prefetched.insert(p.id.clone()))
            .cloned()
            .collect();
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
                Msg::List { stale, .. } => ("list", *stale),
                Msg::ListRows { .. } => ("list rows", false),
                Msg::ListMerge { .. } => ("list merge", false),
                Msg::Detail { stale, .. } => ("detail", *stale),
                Msg::Files { stale, .. } => ("files", *stale),
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
                entry.apply(result, stale);
                if let (Some(old), Some(new)) = (old, entry.data.as_mut()) {
                    // The merge status arrives separately; don't lose it.
                    let new = Arc::make_mut(new);
                    for r in &mut new.rows {
                        if r.merge_state_status.is_none() {
                            if let Some(o) = old.rows.iter().find(|o| o.id == r.id) {
                                r.merge_state_status = o.merge_state_status.clone();
                                r.auto_merge = o.auto_merge;
                            }
                        }
                    }
                }
                if !stale {
                    if let Some(d) = &entry.data {
                        let (key, d) = (key.clone(), d.clone());
                        std::thread::spawn(move || cache::store(&key, &*d));
                    }
                }
                let rows = entry.data.clone();
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
                let Ok(mut rows) = result else { return };
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
                        }
                    }
                }
                entry.data = Some(Arc::new(ListResult { open, closed, rows: rows.clone() }));
                if key == self.list_key(&self.view) {
                    self.prefetch(&rows);
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
                let unknown = result.as_ref().ok().filter(|d| d.state == "OPEN").is_some_and(|d| {
                    matches!(d.merge_state_status.as_deref(), None | Some("UNKNOWN")) || d.mergeable == "UNKNOWN"
                });
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
                self.diff_layouts.remove(&id);
                self.files.entry(id.clone()).or_default().apply(result, stale);
                let pending: Vec<(String, bool)> =
                    self.viewed_pending.iter().filter(|((i, _), _)| *i == id).map(|((_, p), v)| (p.clone(), *v)).collect();
                for (path, v) in pending {
                    self.patch_viewed(&id, &path, v);
                }
            }
            Msg::FileText { pr_id, path, result } => match result {
                Ok(text) => {
                    let lines: Vec<String> = text.lines().map(str::to_string).collect();
                    self.file_texts.insert((pr_id.clone(), path), Some(Arc::new(lines)));
                    self.diff_layouts.remove(&pr_id);
                }
                Err(e) => {
                    self.file_texts.remove(&(pr_id.clone(), path.clone()));
                    self.expanded.remove(&(pr_id.clone(), path));
                    self.diff_layouts.remove(&pr_id);
                    self.show_toast(format!("Couldn't load the file: {e}"), true);
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
            Msg::Found { seq, result } => {
                let Some(g) = self.goto.as_mut().filter(|g| g.seq == seq) else { return };
                match result {
                    Ok(pr) => {
                        self.goto = None;
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
        }
        self.diff_layouts.remove(pr_id);
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
        self.md_text
            .entry(h.finish())
            .or_insert_with(|| Arc::new(crate::gfm::to_markdown(&util::clean_markdown(body), repo, &emojis)))
            .clone()
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
                self.load_list(self.view.clone());
            }
            Action::Select(pr) => {
                self.load_detail(&pr, false);
                if self.selected.as_ref().map(|p| &p.id) != Some(&pr.id) {
                    self.composer.clear();
                }
                self.selected = Some(pr);
            }
            Action::Tab(t) => self.tab = t,
            Action::Refresh => self.refresh_all(),
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
                if let Some(pr) = &self.selected {
                    let now = self.is_collapsed(&pr.id, &name);
                    self.collapsed.insert((pr.id.clone(), name), !now);
                    self.diff_layouts.remove(&pr.id);
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
                let rest: Vec<&str> =
                    q.split_whitespace().filter(|t| !matches!(*t, "is:pr" | "is:open" | "is:closed" | "archived:false")).collect();
                let rest = rest.join(" ");
                let view = match SECTIONS.iter().position(|s| s.query == rest) {
                    Some(i) => View::Section(i),
                    None if rest.is_empty() => View::Section(0),
                    None => View::Search(rest),
                };
                self.closed = closed;
                self.apply(Action::SetView(view), ctx);
            }
            Action::ReadyForReview => {
                self.merge_action("Marked ready for review", |c, d| c.ready_for_review(&d.id));
            }
            Action::SetPreview(v) => self.composer_preview = v,
            Action::ClosePr => {
                self.merge_action("Pull request closed", |c, d| c.close_pull_request(&d.id));
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
                let known = self.lists.values().filter_map(|l| l.data.as_ref()).flat_map(|l| l.rows.iter()).find(|p| {
                    p.number == number && p.repository.name_with_owner.eq_ignore_ascii_case(&repo)
                });
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
                self.spawn(move |c, tx| {
                    let result = c.pull_request(&repo, number);
                    let _ = tx.send(Msg::Found { seq, result });
                });
            }
            Action::ExpandHunk(path, idx, amount) => {
                let Some(pr) = self.selected.clone() else { return };
                let key = (pr.id.clone(), path.clone());
                *self.expanded.entry(key.clone()).or_default().entry(idx).or_default() += amount;
                self.diff_layouts.remove(&pr.id);
                if !self.file_texts.contains_key(&key) {
                    let head = self.details.get(&pr.id).and_then(|d| d.data.as_ref()).map(|d| d.head_ref_oid.clone()).unwrap_or_default();
                    if head.is_empty() {
                        return;
                    }
                    self.file_texts.insert(key, None);
                    let repo = pr.repository.name_with_owner.clone();
                    self.spawn(move |c, tx| {
                        let result = c.file_text(&repo, &path, &head);
                        let _ = tx.send(Msg::FileText { pr_id: pr.id, path, result });
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
        let next = match cur {
            Some(i) => (i as i64 + delta).clamp(0, rows.rows.len() as i64 - 1) as usize,
            None => 0,
        };
        self.actions.push(Action::Select(rows.rows[next].clone()));
        self.ctx.data_mut(|d| d.insert_temp(egui::Id::new("scroll-to-row"), next));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input() || self.goto.is_some();
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
                acts.push(Action::TogglePanel(if self.tab == Tab::Files { Panel::Tree } else { Panel::Details }));
            }
            if i.consume_shortcut(&cmd(Key::B)) {
                acts.push(Action::TogglePanel(Panel::List));
            }
            for (k, t) in [(Key::Num1, Tab::Conversation), (Key::Num2, Tab::Commits), (Key::Num3, Tab::Checks), (Key::Num4, Tab::Files)] {
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
        if !typing {
            let (down, up, slash, copy) = ctx.input(|i| {
                let plain = i.modifiers.is_none();
                (
                    i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::J),
                    i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::K),
                    i.key_pressed(Key::Slash),
                    plain && i.key_pressed(Key::C),
                )
            });
            if copy {
                if let Some(pr) = &self.selected {
                    self.actions.push(Action::Copy(pr.url.clone()));
                }
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
                    _ => Tab::Conversation,
                };
                self.goto = Some(GoTo { text, ..Default::default() });
                self.actions.push(Action::GoTo);
            } else if let Ok(text) = std::env::var("GITHUB_PRS_GOTO_TYPE") {
                if list_ready {
                    shot.selected = true;
                    self.goto = Some(GoTo { text, ..Default::default() });
                }
            }
        }
        if !shot.selected {
            if let Some(rows) = rows.filter(|r| !r.rows.is_empty()) {
                let i: usize = std::env::var("GITHUB_PRS_ROW").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
                let pr = rows.rows[i.min(rows.rows.len() - 1)].clone();
                self.tab = match std::env::var("GITHUB_PRS_TAB").as_deref() {
                    Ok("commits") => Tab::Commits,
                    Ok("checks") => Tab::Checks,
                    Ok("files") => Tab::Files,
                    _ => Tab::Conversation,
                };
                shot.selected = true;
                self.actions.push(Action::Select(pr));
            }
        }
        let typing_only = self.goto.is_some() && std::env::var("GITHUB_PRS_GOTO_TYPE").is_ok();
        let loaded = typing_only || self.selected.as_ref().is_some_and(|s| {
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
                if state.contains("reply") {
                    if let Some(t) = d.review_threads.nodes.iter().find(|t| t.viewer_can_reply) {
                        self.thread_replies.insert(t.id.clone(), "Good catch, fixed in the next commit.".into());
                    }
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

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
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
            self.follow_list();
            self.shortcuts(&ctx);
            views::main(self, ui);
            views::goto_box(self, &ctx);
        } else {
            views::sign_in(self, ui);
        }
        views::toast(self, &ctx);

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
