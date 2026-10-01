//! Screens and shared widgets, styled after github.com.

mod detail;
mod list;

use crate::app::{Action, App, Panel};
use crate::github::PrDetail;
use crate::icons::{self, Icon};
use crate::theme::{self, Palette};
use egui::{Color32, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

/// Below this window width the PR list starts folded.
const NARROW: f32 = 1100.0;

pub fn main(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    header(app, ui, p);
    // Narrow windows: the list folds to its rail on its own (⌘B still opens
    // it), and it never takes more than ~40% of the width.
    let width = ui.available_width();
    // On Files, the diff keeps at least 900px (below that the tree hides),
    // so the list folds sooner there and never grows past what's left.
    let files = app.tab != crate::app::Tab::Conversation && app.selected.is_some();
    let room = width - 900.0 - 40.0;
    app.narrow = width < NARROW || (files && room < 300.0);
    let mut open = if app.narrow { app.list_open_narrow } else { app.panels.list };
    let max = if app.narrow { 340.0 } else { (width * 0.42).clamp(300.0, 760.0) };
    let max = if files && !app.narrow { max.min(room.max(300.0)) } else { max };
    // The PR list slides closed to a thin strip of PR icons. Drag its edge
    // past the minimum width to close it too.
    egui::Panel::show_switched(
        ui,
        &mut open,
        egui::Panel::left("list-rail")
            .resizable(false)
            .exact_size(52.0)
            .frame(Frame::new().fill(p.canvas_subtle).inner_margin(Margin::symmetric(8, 12))),
        egui::Panel::left("list")
            .resizable(true)
            .default_size(480.0f32.min(max))
            .size_range(300.0f32.min(max)..=max)
            .frame(Frame::new().fill(p.canvas).inner_margin(Margin { left: 16, right: 12, top: 16, bottom: 0 })),
        |ui, expanded| if expanded { list::show(app, ui) } else { list::rail(app, ui) },
    );
    if app.narrow {
        app.list_open_narrow = open;
    } else {
        app.panels.list = open;
    }
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.canvas).inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 0 }))
        .show(ui, |ui| detail::show(app, ui));
}

fn header(app: &mut App, ui: &mut Ui, p: &Palette) {
    egui::Panel::top("header")
        .exact_size(52.0)
        .frame(
            Frame::new()
                .fill(p.header)
                .inner_margin(Margin::symmetric(16, 0))
                .stroke(Stroke::new(1.0, p.border)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let list_open = if app.narrow { app.list_open_narrow } else { app.panels.list };
                let (icon, tip) = if list_open {
                    (Icon::SidebarLeftClose, "Hide pull request list (⌘B)")
                } else {
                    (Icon::SidebarLeftOpen, "Show pull request list (⌘B)")
                };
                if icon_button(ui, icon, tip, p).clicked() {
                    app.actions.push(Action::TogglePanel(Panel::List));
                }
                ui.add_space(6.0);
                icons::show(ui, Icon::PrOpen, 20.0, p.fg);
                ui.add_space(2.0);
                ui.label(RichText::new("Pull requests").font(theme::bold(15.0)).color(p.fg));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !app.viewer.is_empty() {
                        let url = format!("https://github.com/{}.png?size=64", app.viewer);
                        let r = avatar(ui, &url, 28.0);
                        let r = r.interact(Sense::click()).tip(format!(
                            "Signed in as {} ({})",
                            app.viewer,
                            app.auth_source.map(|s| s.describe()).unwrap_or("")
                        ));
                        r.context_menu(|ui| {
                            if ui.button("Sign out").clicked() {
                                app.actions.push(Action::SignOut);
                            }
                        });
                    }
                    ui.add_space(4.0);
                    let loading = app.lists.values().any(|l| l.loading);
                    if loading {
                        // Same button frame, with a spinner in it.
                        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                        ui.painter().rect(rect, 6.0, p.btn_bg, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                        egui::Spinner::new().size(16.0).color(p.fg_muted).paint_at(ui, rect.shrink(8.0));
                    } else if icon_button(ui, Icon::Sync, "Refresh (⌘R)", p).clicked() {
                        app.actions.push(Action::Refresh);
                    }
                });
            });
        });
}

pub fn sign_in(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    egui::CentralPanel::default().frame(Frame::new().fill(p.canvas_subtle)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 190.0).max(40.0));
            icons::show(ui, Icon::PrOpen, 48.0, p.fg);
            ui.add_space(12.0);
            ui.label(RichText::new("Sign in to GitHub").font(theme::body(24.0)).color(p.fg));
            ui.add_space(16.0);
            if app.finding_token {
                ui.add(egui::Spinner::new().size(20.0));
                ui.label(RichText::new("Looking for your GitHub login…").color(p.fg_muted));
                return;
            }
            Frame::new()
                .fill(p.canvas)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(6)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.vertical(|ui| {
                        if let Some(e) = &app.token_error {
                            ui.label(RichText::new(e).color(p.closed).size(13.0));
                            ui.add_space(8.0);
                        }
                        ui.label(
                            RichText::new(
                                "No gh CLI login or GH_TOKEN was found. Paste a personal access token with the repo and read:org scopes.",
                            )
                            .size(13.0)
                            .color(p.fg_muted),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("Personal access token").font(theme::bold(14.0)));
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut app.token_input)
                                .password(true)
                                .desired_width(f32::INFINITY)
                                .margin(vec2(8.0, 6.0)),
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            app.actions.push(Action::SubmitToken);
                        }
                        ui.add_space(12.0);
                        let w = ui.available_width();
                        if ui.add_sized([w, 32.0], primary_button("Sign in", p)).clicked() {
                            app.actions.push(Action::SubmitToken);
                        }
                    });
                });
            ui.add_space(8.0);
            if ui.add(button("Use my gh login", p)).tip("Look for a gh CLI login or GH_TOKEN again").clicked() {
                app.actions.push(Action::FindToken);
            }
            ui.add_space(16.0);
            Frame::new()
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(6)
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Need a token?").size(13.0));
                            if link(ui, "Create one on GitHub.", 13.0, p).clicked() {
                                app.actions.push(Action::OpenUrl(
                                    "https://github.com/settings/tokens/new?scopes=repo,read:org&description=GitHub%20PRs%20desktop".into(),
                                ));
                            }
                        });
                    });
                });
        });
    });
}

/// One ⌘K result: a PR already loaded, or a ref to fetch from GitHub.
enum GoResult {
    Pr(crate::github::PrSummary),
    Ref(String, u64),
}

impl GoResult {
    fn key(&self) -> String {
        match self {
            GoResult::Pr(pr) => pr.id.clone(),
            GoResult::Ref(repo, n) => format!("{repo}#{n}"),
        }
    }
}

/// Every PR in the loaded lists, in a fixed order (the list on screen
/// first), so ⌘K results don't reshuffle as other lists finish loading.
fn loaded_prs(app: &App) -> Vec<&crate::github::PrSummary> {
    let current = app.list_key(&app.view);
    let mut keys: Vec<&String> = app.lists.keys().collect();
    keys.sort_by_key(|k| (**k != current, (*k).clone()));
    let mut out: Vec<&crate::github::PrSummary> = Vec::new();
    for k in keys {
        for pr in app.lists[k].data.iter().flat_map(|l| l.rows.iter()) {
            if !out.iter().any(|o| o.id == pr.id) {
                out.push(pr);
            }
        }
    }
    out
}

/// ⌘K: type a PR number, `owner/repo#123`, or paste a link to open it.
pub fn goto_box(app: &mut App, ctx: &egui::Context) {
    let default_repo = app.default_repo();
    let raw = app.goto.as_ref().map(|g| g.text.trim().to_string()).unwrap_or_default();
    let typed = raw.to_lowercase();
    let loaded = loaded_prs(app);
    let mut results: Vec<GoResult> = Vec::new();
    let has = |results: &[GoResult], id: &str| results.iter().any(|r| matches!(r, GoResult::Pr(p) if p.id == id));
    let digits = typed.trim_start_matches('#');
    let bare = !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit());
    let parsed = crate::util::parse_pr_ref(&raw, default_repo.as_deref());
    if bare {
        // Loaded PRs with that number first (the default repo is only a
        // guess), then numbers that start with it, then the guess.
        let n: u64 = digits.parse().unwrap_or(0);
        for pr in loaded.iter().filter(|p| p.number == n) {
            results.push(GoResult::Pr((*pr).clone()));
        }
        for pr in loaded.iter().filter(|p| p.number != n && p.number.to_string().starts_with(digits)).take(5) {
            results.push(GoResult::Pr((*pr).clone()));
        }
        if let Some((repo, n)) = parsed.clone() {
            let known = results.iter().any(|r| matches!(r, GoResult::Pr(p) if p.number == n && p.repository.name_with_owner.eq_ignore_ascii_case(&repo)));
            if !known {
                results.push(GoResult::Ref(repo, n));
            }
        }
    } else if let Some((repo, n)) = parsed {
        // A ref or link: the loaded PR if we have it, else fetch it.
        match loaded.iter().find(|p| p.number == n && p.repository.name_with_owner.eq_ignore_ascii_case(&repo)) {
            Some(pr) => results.push(GoResult::Pr((*pr).clone())),
            None => results.push(GoResult::Ref(repo, n)),
        }
    } else if typed.len() >= 2 {
        // Words: match titles of PRs already loaded.
        for pr in &loaded {
            let hay = format!("{} {}", pr.title, pr.repository.name_with_owner);
            if crate::util::words_match(&hay, &typed) && !has(&results, &pr.id) {
                results.push(GoResult::Pr((*pr).clone()));
            }
        }
    }
    results.truncate(6);
    drop(loaded);

    let mut picked: Option<crate::github::PrSummary> = None;
    let mut submit = false;
    let mut search_for: Option<String> = None;
    let Some(g) = app.goto.as_mut() else { return };
    // The highlight follows a result, not a position, so it can't jump to
    // another PR when the results change.
    let mut sel = g.sel.as_ref().and_then(|k| results.iter().position(|r| r.key() == *k)).unwrap_or(0);
    // Arrow keys (and Ctrl-N/P) move the highlight. Taken before the text
    // box sees them, so they don't move its cursor.
    if !results.is_empty() {
        let (down, up) = ctx.input_mut(|i| {
            let down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) || i.consume_key(egui::Modifiers::CTRL, egui::Key::N);
            let up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) || i.consume_key(egui::Modifiers::CTRL, egui::Key::P);
            (down, up)
        });
        if down {
            sel = (sel + 1) % results.len();
        }
        if up {
            sel = (sel + results.len() - 1) % results.len();
        }
    }
    g.sel = results.get(sel).map(GoResult::key);
    let p = theme::palette(ctx);
    let id = egui::Id::new("goto");
    let area = egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, vec2(0.0, 96.0));
    let frame = Frame::new()
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(12)
        .inner_margin(Margin::same(0))
        .shadow(egui::Shadow { offset: [0, 12], blur: 48, spread: 0, color: Color32::from_black_alpha(if p.dark { 180 } else { 70 }) });
    let resp = egui::Modal::new(id).area(area).frame(frame).backdrop_color(p.backdrop).show(ctx, |ui| {
        ui.set_width(560.0);
        Frame::new().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::PrOpen, 16.0, p.fg_muted);
                let before = g.text.clone();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut g.text)
                        .frame(Frame::NONE)
                        .font(theme::body(16.0))
                        .hint_text("Go to pull request: 123, owner/repo#123, or a link")
                        .margin(vec2(4.0, 2.0))
                        .desired_width(f32::INFINITY),
                );
                if !edit.has_focus() && !g.loading {
                    edit.request_focus();
                }
                if edit.changed() {
                    g.error = None;
                    g.sel = None;
                }
                // A pasted link is unambiguous, so open it straight away.
                let pasted = g.text.len() > before.len() + 1 && g.text.contains("github.com/");
                if pasted && crate::util::parse_pr_ref(&g.text, None).is_some() {
                    submit = true;
                } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    match results.get(sel) {
                        Some(GoResult::Pr(pr)) => picked = Some(pr.clone()),
                        Some(GoResult::Ref(..)) => submit = true,
                        None => {}
                    }
                }
            });
        });
        ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, p.border_muted));
        Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if g.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Opening…").color(p.fg_muted).size(13.0));
                });
            } else if let Some(e) = &g.error {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::X, 14.0, p.closed);
                    ui.label(RichText::new(e).color(p.closed).size(13.0));
                });
            } else if !results.is_empty() {
                ui.vertical(|ui| {
                    for (i, res) in results.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                        // Hover moves the highlight, but only when the mouse
                        // moves, so a resting pointer doesn't fight the keys.
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) && i != sel {
                            g.sel = Some(res.key());
                            ui.ctx().request_repaint();
                        }
                        let on = i == sel;
                        if on {
                            ui.painter().rect_filled(r, 6.0, p.hover_row);
                            ui.painter().rect_filled(Rect::from_min_size(r.min + vec2(0.0, 5.0), vec2(2.0, 20.0)), 1.0, p.accent_emphasis);
                        }
                        let mut job = egui::text::LayoutJob::default();
                        let muted = |c| egui::TextFormat { font_id: theme::body(13.0), color: c, ..Default::default() };
                        let (icon, color) = match res {
                            GoResult::Pr(pr) => {
                                job.append(&format!("{}#{} ", pr.repository.name_with_owner, pr.number), 0.0, muted(p.fg_muted));
                                append_title(&mut job, &pr.title, 13.0, true, 0.0, p);
                                let (icon, color, _) = pr_icon(&pr.state, pr.is_draft, p);
                                (icon, color)
                            }
                            GoResult::Ref(repo, n) => {
                                job.append(&format!("{repo}#{n}"), 0.0, egui::TextFormat { font_id: theme::bold(13.0), color: p.fg, ..Default::default() });
                                job.append("Open from GitHub", 8.0, muted(p.fg_muted));
                                (Icon::PrOpen, p.fg_muted)
                            }
                        };
                        icons::paint(ui.painter(), Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 7.0), vec2(14.0, 14.0)), icon, color);
                        // The highlighted row gets the Enter hint.
                        let hint_w = if on { ui.painter().layout_no_wrap("Enter to open".into(), theme::body(12.0), p.fg_muted).size().x + 16.0 } else { 0.0 };
                        let (tg, _) = cut_job(ui.painter(), job, r.width() - 44.0 - hint_w);
                        ui.painter().galley(pos2(r.left() + 32.0, r.center().y - tg.size().y / 2.0), tg, p.fg);
                        if on {
                            ui.painter().text(pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, "Enter to open", theme::body(12.0), p.fg_muted);
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            match res {
                                GoResult::Pr(pr) => picked = Some(pr.clone()),
                                GoResult::Ref(..) => {
                                    g.sel = Some(res.key());
                                    submit = true;
                                }
                            }
                        }
                    }
                });
            } else if !g.text.trim().is_empty() {
                // Nothing loaded matches: offer GitHub's search for it.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Search pull requests"));
                ui.painter().rect_filled(r, 6.0, p.hover_row);
                ui.painter().rect_filled(Rect::from_min_size(r.min + vec2(0.0, 5.0), vec2(2.0, 20.0)), 1.0, p.accent_emphasis);
                icons::paint(ui.painter(), Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 7.0), vec2(14.0, 14.0)), Icon::Search, p.fg_muted);
                let mut job = egui::text::LayoutJob::default();
                job.append("Search pull requests for ", 0.0, egui::TextFormat { font_id: theme::body(13.0), color: p.fg_muted, ..Default::default() });
                job.append(&format!("\"{}\"", g.text.trim()), 0.0, egui::TextFormat { font_id: theme::bold(13.0), color: p.fg, ..Default::default() });
                job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 44.0 - 110.0);
                let tg = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 32.0, r.center().y - tg.size().y / 2.0), tg, p.fg);
                ui.painter().text(pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, "Enter to search", theme::body(12.0), p.fg_muted);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    search_for = Some(g.text.trim().to_string());
                }
            } else {
                let hint = match &default_repo {
                    Some(r) => format!("A bare number opens a PR in {r}. Esc to close."),
                    None => "Include the repo, like owner/repo#123. Esc to close.".to_string(),
                };
                ui.label(RichText::new(hint).color(p.fg_muted).size(13.0));
            }
        });
    });
    if let Some(pr) = picked {
        app.goto = None;
        app.actions.push(Action::Select(pr));
    } else if let Some(q) = search_for {
        app.goto = None;
        app.actions.push(Action::RunSearch(q));
    } else if submit {
        app.actions.push(Action::GoTo);
    } else if resp.should_close() {
        app.goto = None;
    }
}

/// Lays `job` out on one line cut to `width`, with "…" right after the
/// last visible character (egui's own cut can leave a space before it).
/// Also says whether anything was cut.
pub fn cut_job(painter: &egui::Painter, mut job: egui::text::LayoutJob, width: f32) -> (std::sync::Arc<egui::Galley>, bool) {
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    job.wrap.break_anywhere = true;
    let g = painter.layout_job(job.clone());
    if !g.elided {
        return (g, false);
    }
    // The glyphs that fit, minus the "…" and any spaces before it.
    let fit = g.rows.first().map(|r| r.glyphs.len()).unwrap_or(0).saturating_sub(1);
    let chars: Vec<char> = job.text.chars().collect();
    let mut keep = fit.min(chars.len());
    while keep > 0 && chars[keep - 1].is_whitespace() {
        keep -= 1;
    }
    let end: usize = chars[..keep].iter().map(|c| c.len_utf8()).sum();
    let mut cut = egui::text::LayoutJob::default();
    let mut last = None;
    for s in &job.sections {
        let (start, stop) = (s.byte_range.start.0, s.byte_range.end.0.min(end));
        if start >= end {
            break;
        }
        cut.append(&job.text[start..stop], s.leading_space, s.format.clone());
        last = Some(s.format.clone());
    }
    if let Some(f) = last {
        cut.append("…", 0.0, f);
    }
    (painter.layout_job(cut), true)
}

/// Adds a PR title to `job`, drawing `code spans` in monospace on a tinted
/// background like github.com.
pub fn append_title(job: &mut egui::text::LayoutJob, title: &str, size: f32, bold: bool, leading: f32, p: &Palette) {
    let font = if bold { theme::bold(size) } else { theme::body(size) };
    let code_bg = if p.dark { Color32::from_rgba_unmultiplied(101, 108, 118, 51) } else { Color32::from_rgba_unmultiplied(129, 139, 152, 31) };
    let parts: Vec<&str> = title.split('`').collect();
    // An odd number of backticks means an unclosed span: show it as typed.
    let balanced = parts.len() % 2 == 1;
    let mut leading = leading;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let code = balanced && i % 2 == 1;
        let text = if !balanced && i > 0 { format!("`{part}") } else { part.to_string() };
        let format = if code {
            egui::TextFormat { font_id: theme::mono(size * 0.85), color: p.fg, background: code_bg, valign: egui::Align::Center, ..Default::default() }
        } else {
            egui::TextFormat { font_id: font.clone(), color: p.fg, ..Default::default() }
        };
        job.append(&text, std::mem::take(&mut leading), format);
    }
}

/// Plain words for an error, plus the raw message for the details tooltip.
pub fn friendly_error(raw: &str) -> &'static str {
    let r = raw.to_ascii_lowercase();
    if r.contains("rate limit") {
        "GitHub's rate limit was hit. Try again in a few minutes."
    } else if ["network", "connection", "timed out", "timeout", "dns", "io:", "tls"].iter().any(|k| r.contains(k)) {
        "Can't reach GitHub. Check your connection."
    } else if r.contains("saml") || r.contains("sso") {
        "Your organization needs single sign-on for this token."
    } else if r.contains("invalid search") || r.contains("query") || r.contains("parse") {
        "GitHub couldn't read that search. Check the query syntax."
    } else {
        "GitHub returned an error."
    }
}

/// Primer's red flash banner with a Retry button. Returns true on Retry.
pub fn error_banner(ui: &mut Ui, p: &Palette, title: &str, raw: &str) -> bool {
    let mut retry = false;
    Frame::new()
        .fill(p.danger_subtle)
        .stroke(Stroke::new(1.0, p.danger_border))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icons::show(ui, Icon::X, 16.0, p.closed);
                // The text wraps in what's left beside Retry.
                let text_w = (ui.available_width() - 80.0).max(80.0);
                ui.allocate_ui_with_layout(vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(title).font(theme::bold(14.0)).color(p.fg));
                    ui.label(RichText::new(friendly_error(raw)).size(13.0).color(p.fg_muted)).tip(raw);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    retry = ui.add(button("Retry", p)).clicked();
                });
            });
        });
    retry
}

/// "Couldn't refresh" while older data stays on screen: a small note in the
/// corner, so nothing on the page moves. Returns true on Retry.
pub fn stale_note(ctx: &egui::Context, id: &str, raw: &str) -> bool {
    let p = theme::palette(ctx);
    let mut retry = false;
    egui::Area::new(egui::Id::new(("stale", id)))
        // The list's note sits bottom left, the PR page's bottom right.
        .anchor(if id == "list" { egui::Align2::LEFT_BOTTOM } else { egui::Align2::RIGHT_BOTTOM }, vec2(if id == "list" { 16.0 } else { -16.0 }, -16.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(6).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Sync, 14.0, p.attention);
                    ui.label(RichText::new("Couldn't refresh · showing saved data").size(12.0).color(p.fg_muted)).tip(format!("{}\n{raw}", friendly_error(raw)));
                    retry = link(ui, "Retry", 12.0, p).clicked();
                });
            });
        });
    retry
}

pub fn toast(app: &mut App, ctx: &egui::Context) {
    let Some(t) = &app.toast else { return };
    if std::time::Instant::now() > t.until {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    let p = theme::palette(ctx);
    egui::Area::new(egui::Id::new("toast"))
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
        .show(ctx, |ui| {
            // Primer toast: dark in both themes, colored icon, white text.
            Frame::new()
                .fill(p.tooltip)
                .corner_radius(6)
                .inner_margin(Margin::symmetric(14, 10))
                .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(60) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (icon, color) = if t.error { (Icon::X, Color32::from_rgb(0xff, 0x81, 0x82)) } else { (Icon::Check, Color32::from_rgb(0x4a, 0xc2, 0x6b)) };
                        icons::show(ui, icon, 16.0, color);
                        ui.label(RichText::new(&t.text).color(Color32::WHITE).size(14.0));
                    });
                });
        });
}

// ---------- Shared widgets ----------

/// Round avatar. Shows a gray circle until the image loads.
pub fn avatar(ui: &mut Ui, url: &str, size: f32) -> egui::Response {
    avatar_rounded(ui, url, size, size / 2.0)
}

/// A bot or app: GitHub draws those as rounded squares, people as circles.
pub fn avatar_of(ui: &mut Ui, login: &str, url: &str, size: f32) -> egui::Response {
    let bot = login.ends_with("[bot]") || login.ends_with("-bot") || login.ends_with("-robot") || login == "github-actions" || url.contains("/in/");
    avatar_rounded(ui, url, size, if bot { size * 0.25 } else { size / 2.0 })
}

fn avatar_rounded(ui: &mut Ui, url: &str, size: f32, radius: f32) -> egui::Response {
    let p = theme::palette(ui.ctx());
    let shape = |rect: egui::Rect, fill: Color32, stroke: Stroke| egui::Shape::Rect(egui::epaint::RectShape::new(rect, radius, fill, stroke, egui::StrokeKind::Inside));
    if url.is_empty() {
        // No GitHub account: a plain person, like GitHub's default avatar.
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        ui.painter().add(shape(rect, p.canvas_subtle, Stroke::new(1.0, p.border_muted)));
        icons::paint(ui.painter(), rect.shrink(size * 0.2), Icon::Person, p.fg_muted);
        return resp;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().add(shape(rect, p.border_muted, Stroke::NONE));
    let image = egui::Image::new(url).corner_radius(radius).show_loading_spinner(false);
    // A failed download would draw egui's red error glyph; show an identicon instead.
    if image.load_for_size(ui.ctx(), rect.size()).is_err() {
        identicon(ui.painter(), rect, url, p);
    } else {
        image.paint_at(ui, rect);
    }
    ui.painter().add(shape(rect, Color32::TRANSPARENT, Stroke::new(1.0, p.border_muted)));
    resp
}

/// GitHub-style 5×5 mirrored pattern, picked from a hash of `seed`.
fn identicon(painter: &egui::Painter, rect: egui::Rect, seed: &str, p: &Palette) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    let bits = h.finish();
    let color = Color32::from_rgb(90 + (bits >> 40) as u8 % 120, 90 + (bits >> 48) as u8 % 120, 90 + (bits >> 56) as u8 % 120);
    painter.circle_filled(rect.center(), rect.width() / 2.0, if p.dark { p.canvas_subtle } else { Color32::from_gray(240) });
    let inner = rect.shrink(rect.width() * 0.2);
    let cell = inner.width() / 5.0;
    for y in 0..5 {
        for x in 0..3 {
            if bits >> (y * 3 + x) & 1 == 1 {
                for cx in [x, 4 - x] {
                    let r = egui::Rect::from_min_size(inner.min + vec2(cx as f32 * cell, y as f32 * cell), vec2(cell, cell));
                    painter.rect_filled(r, 0.0, color);
                }
            }
        }
    }
}

/// Rounded pill, used for labels, counters and branch names.
pub fn pill(ui: &mut Ui, text: &str, font: egui::FontId, bg: Color32, fg: Color32, border: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let pad = vec2(7.0, 2.0);
    let size = galley.size() + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(255), bg, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    ui.painter().galley(rect.min + pad, galley, fg);
    resp
}

pub fn label_pill(ui: &mut Ui, name: &str, color: &str) {
    let p = theme::palette(ui.ctx());
    let (bg, fg, border) = theme::label_colors(color, p);
    pill(ui, name, theme::bold(12.0), bg, fg, border);
}

pub fn primary_button(text: &str, p: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).font(theme::bold(14.0)).color(Color32::WHITE))
        .fill(p.btn_primary)
        .stroke(Stroke::new(1.0, p.btn_primary.gamma_multiply(0.85)))
        .corner_radius(6)
}

pub fn button(text: &str, p: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).font(theme::bold(14.0)).color(p.fg))
        .fill(p.btn_bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6)
}

/// `.tip("…")` on any response: a Primer tooltip instead of egui's.
pub trait Tip {
    fn tip(self, text: impl AsRef<str>) -> Self;
    /// Above the widget, for spots where the text beside it matters.
    fn tip_above(self, text: impl AsRef<str>) -> Self;
    /// Below it, when what's above matters too.
    fn tip_below(self, text: impl AsRef<str>) -> Self;
}

impl Tip for egui::Response {
    fn tip(self, text: impl AsRef<str>) -> Self {
        tip(&self, text.as_ref());
        self
    }

    fn tip_above(self, text: impl AsRef<str>) -> Self {
        tip_at(&self, text.as_ref(), Side::Above);
        self
    }

    fn tip_below(self, text: impl AsRef<str>) -> Self {
        tip_at(&self, text.as_ref(), Side::Below);
        self
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Side {
    Auto,
    Above,
    Below,
}

/// Primer's tooltip: dark, small white text, beside the widget instead of
/// under the pointer.
pub fn tip(resp: &egui::Response, text: &str) {
    tip_at(resp, text, Side::Auto);
}

fn tip_at(resp: &egui::Response, text: &str, side: Side) {
    if !resp.hovered() || resp.ctx.dragged_id().is_some() || egui::Popup::is_any_open(&resp.ctx) {
        return;
    }
    // After a click, nothing shows until the pointer moves: a button that
    // swaps for another under a resting pointer shouldn't pop its tooltip.
    let click_id = egui::Id::new("tip-click-pos");
    let pointer = resp.ctx.input(|i| i.pointer.latest_pos());
    if resp.ctx.input(|i| i.pointer.any_click()) {
        resp.ctx.data_mut(|d| d.insert_temp(click_id, pointer));
        return;
    }
    if resp.ctx.data(|d| d.get_temp::<Option<egui::Pos2>>(click_id)).flatten().is_some_and(|at| Some(at) == pointer) {
        return;
    }
    let p = theme::palette(&resp.ctx);
    let r = resp.rect;
    let screen = resp.ctx.content_rect();
    // To the right if there's room, else below; near the right edge, below
    // and right-aligned so it stays inside the window.
    let (pos, pivot) = if side == Side::Above && r.top() > screen.top() + 40.0 {
        (egui::pos2(r.left(), r.top() - 6.0), egui::Align2::LEFT_BOTTOM)
    } else if side == Side::Below {
        (egui::pos2(r.left(), r.bottom() + 6.0), egui::Align2::LEFT_TOP)
    } else if r.right() + 340.0 < screen.right() {
        (egui::pos2(r.right() + 6.0, r.center().y), egui::Align2::LEFT_CENTER)
    } else if r.center().x + 170.0 < screen.right() {
        (egui::pos2(r.center().x, r.bottom() + 6.0), egui::Align2::CENTER_TOP)
    } else {
        (egui::pos2(r.right(), r.bottom() + 6.0), egui::Align2::RIGHT_TOP)
    };
    egui::Area::new(resp.id.with("tip"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos)
        .pivot(pivot)
        .interactable(false)
        .show(&resp.ctx, |ui| {
            Frame::new().fill(p.tooltip).corner_radius(4).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.set_max_width(320.0);
                ui.label(RichText::new(text).size(12.0).color(Color32::WHITE));
            });
        });
}

/// A square button with just an icon. `label` is its tooltip and the name
/// screen readers (and scripted UI tests) use.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, p: &Palette) -> egui::Response {
    icon_button_at(ui, icon, label, p, false)
}

/// `icon_button` with its tooltip above, clear of what's below it.
pub fn icon_button_tip_above(ui: &mut Ui, icon: Icon, label: &str, p: &Palette) -> egui::Response {
    icon_button_at(ui, icon, label, p, true)
}

fn icon_button_at(ui: &mut Ui, icon: Icon, label: &str, p: &Palette, above: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    tip_at(&resp, label, if above { Side::Above } else { Side::Auto });
    let bg = if resp.hovered() { ui.visuals().widgets.hovered.weak_bg_fill } else { p.btn_bg };
    ui.painter().rect(rect, 6.0, bg, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), rect.shrink(8.0), icon, p.fg_muted);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An icon button with no frame until hovered, like GitHub's "⋯".
pub fn icon_button_plain(ui: &mut Ui, icon: Icon, label: &str, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    tip(&resp, label);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.hover_row);
    }
    icons::paint(ui.painter(), rect.shrink(8.0), icon, p.fg_muted);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Blue text that underlines on hover, like links on github.com.
pub fn link(ui: &mut Ui, text: &str, size: f32, p: &Palette) -> egui::Response {
    link_styled(ui, text, theme::body(size), p.accent)
}

pub fn link_styled(ui: &mut Ui, text: &str, font: egui::FontId, color: Color32) -> egui::Response {
    let p = theme::palette(ui.ctx());
    let hover_id = ui.next_auto_id();
    let hovered = ui.ctx().data(|d| d.get_temp::<bool>(hover_id).unwrap_or(false));
    let mut rt = RichText::new(text).font(font).color(if hovered { p.accent } else { color });
    if hovered {
        rt = rt.underline();
    }
    let r = ui.add(egui::Label::new(rt).sense(Sense::click()).selectable(false));
    ui.ctx().data_mut(|d| d.insert_temp(hover_id, r.hovered()));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A bordered box with a gray header strip, like GitHub comment boxes.
pub fn boxed<R>(
    ui: &mut Ui,
    header_fill: Color32,
    border: Color32,
    header: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let fill = theme::palette(ui.ctx()).canvas;
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(header_fill)
                .corner_radius(CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                .inner_margin(Margin::symmetric(16, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    header(ui);
                });
            let r = ui.available_rect_before_wrap();
            ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, border));
            Frame::new()
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    body(ui)
                })
                .inner
        })
        .inner
}

/// A plain bordered box with rows separated by lines.
pub fn plain_box<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    Frame::new()
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui)
        })
        .inner
}

pub fn row_separator(ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, p.border_muted));
}

/// Icon and color for a CI or review result.
pub fn status_icon(state: &str, p: &Palette) -> (Icon, Color32) {
    match state {
        "SUCCESS" => (Icon::Check, p.open),
        "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => (Icon::X, p.closed),
        "CANCELLED" | "SKIPPED" | "NEUTRAL" | "STALE" => (Icon::Skip, p.fg_muted),
        _ => (Icon::Dot, p.attention),
    }
}

/// One-line merge readiness, from GitHub's `mergeStateStatus` plus
/// auto-merge, review and check details when we have them.
pub struct MergeStatus {
    pub icon: Icon,
    pub color: Color32,
    /// Short, for list rows: "Ready to merge", "Blocked", ...
    pub short: &'static str,
    pub title: String,
    pub detail: String,
}

pub fn merge_status(
    p: &Palette,
    state: &str,
    draft: bool,
    merge_state: Option<&str>,
    auto_merge: Option<&crate::github::AutoMerge>,
    auto_merge_on: bool,
    why: String,
) -> Option<MergeStatus> {
    if state != "OPEN" {
        return None;
    }
    let s = |icon, color, short: &'static str, title: &str, detail: String| {
        Some(MergeStatus { icon, color, short, title: title.to_string(), detail })
    };
    if draft || merge_state == Some("DRAFT") {
        return s(Icon::PrDraft, p.neutral, "Draft", "Draft — not ready to merge", "Mark it ready for review to merge.".into());
    }
    if auto_merge_on || auto_merge.is_some() {
        let method = match auto_merge.map(|a| a.merge_method.as_str()) {
            Some("SQUASH") => "squash and merge",
            Some("REBASE") => "rebase and merge",
            _ => "merge",
        };
        let by = auto_merge
            .and_then(|a| a.enabled_by.as_ref())
            .map(|a| format!(" · enabled by {}", a.login))
            .unwrap_or_default();
        let waiting = if why.is_empty() {
            String::new()
        } else {
            let mut w = why.chars();
            let first = w.next().map(|c| c.to_lowercase().to_string()).unwrap_or_default();
            format!(" Waiting on: {first}{}", w.as_str())
        };
        return s(Icon::AutoMerge, p.merged, "Auto-merge on", "Auto-merge enabled", format!("GitHub will {method} when all requirements are met{by}.{waiting}"));
    }
    match merge_state.unwrap_or("UNKNOWN") {
        "CLEAN" | "HAS_HOOKS" => s(Icon::Check, p.open, "Ready to merge", "Ready to merge", "All requirements are met.".into()),
        "UNSTABLE" => s(Icon::Check, p.open, "Ready to merge", "Ready to merge", if why.is_empty() { "Some checks that aren't required are failing.".into() } else { why }),
        "BEHIND" => s(Icon::Sync, p.attention, "Out of date", "Branch is out of date", "Update it with the base branch before merging.".into()),
        "DIRTY" => s(Icon::X, p.closed, "Conflicts", "Merge conflicts", "Resolve conflicts with the base branch before merging.".into()),
        "BLOCKED" => s(Icon::X, p.closed, "Blocked", "Merging is blocked", if why.is_empty() { "GitHub reports a branch rule is blocking this merge.".into() } else { why }),
        _ => s(Icon::Dot, p.fg_muted, "Checking", "Checking merge status…", "GitHub hasn't finished computing this yet.".into()),
    }
}

/// Merge status for a PR page. GitHub's own `mergeStateStatus` sometimes
/// says BLOCKED when every rule it lets us see is met, and its review field
/// ignores repository rulesets. So we check the requirements ourselves:
/// approval, required checks (policy-bot counts as passing), resolved
/// conversations and no conflicts. `list_state` is the list's value, used
/// while GitHub is still computing the page's.
pub fn detail_merge_status(p: &Palette, d: &PrDetail, list_state: Option<&str>) -> Option<MergeStatus> {
    let gh_state = match d.merge_state_status.as_deref() {
        None | Some("UNKNOWN") => list_state.or(d.merge_state_status.as_deref()),
        known => known,
    };
    let author = d.author.as_ref().map(|a| a.login.as_str()).unwrap_or("");
    let by_others = d.latest_opinionated_reviews.nodes.iter().filter(|r| r.author.as_ref().is_none_or(|a| a.login != author));
    let approved = by_others.clone().any(|r| r.state == "APPROVED");
    let changes = by_others.clone().any(|r| r.state == "CHANGES_REQUESTED");
    let unresolved = d.review_threads.nodes.iter().filter(|t| !t.is_resolved).count();
    let required_failing = d.checks.iter().filter(|c| c.required && check_rank(&c.result) == 0).count();
    let required_running = d.checks.iter().filter(|c| c.required && check_rank(&c.result) == 1).count();

    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let mut why = Vec::new();
    if changes {
        why.push("changes requested".to_string());
    } else if !approved {
        why.push("needs an approving review".to_string());
    }
    if required_failing > 0 {
        why.push(plural(required_failing, "required check failing", "required checks failing"));
    }
    if required_running > 0 {
        why.push(plural(required_running, "required check running", "required checks running"));
    }
    if unresolved > 0 {
        why.push(plural(unresolved, "unresolved conversation", "unresolved conversations"));
    }
    let ready = why.is_empty() && d.mergeable == "MERGEABLE";
    let state = match gh_state {
        Some("BLOCKED") | Some("UNKNOWN") | None if ready => Some("CLEAN"),
        other => other,
    };
    let mut detail = why.join(" · ");
    if let Some(first) = detail.get(..1) {
        detail = first.to_uppercase() + &detail[1..] + ".";
    }
    merge_status(p, &d.state, d.is_draft, state, d.auto_merge_request.as_ref(), false, detail)
}

/// Counts of checks by outcome.
pub struct CheckSummary {
    pub failed: usize,
    pub pending: usize,
    pub passed: usize,
    pub skipped: usize,
}

impl CheckSummary {
    pub fn of(checks: &[crate::github::Check]) -> Self {
        let mut s = CheckSummary { failed: 0, pending: 0, passed: 0, skipped: 0 };
        for c in checks {
            match check_rank(&c.result) {
                0 => s.failed += 1,
                1 => s.pending += 1,
                2 => s.passed += 1,
                _ => s.skipped += 1,
            }
        }
        s
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (n, word) in [(self.failed, "failing"), (self.pending, "in progress"), (self.skipped, "skipped"), (self.passed, "successful")] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        let total = self.failed + self.pending + self.passed + self.skipped;
        format!("{} check{}", parts.join(", "), if total == 1 { "" } else { "s" })
    }

    /// Overall CI state, like GitHub's rollup: FAILURE, PENDING or SUCCESS.
    pub fn state(&self) -> &'static str {
        if self.failed > 0 {
            "FAILURE"
        } else if self.pending > 0 {
            "PENDING"
        } else {
            "SUCCESS"
        }
    }
}

/// 0 failed, 1 running, 2 passed, 3 skipped or neutral.
pub fn check_rank(result: &str) -> u8 {
    match result {
        "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => 0,
        "PENDING" | "QUEUED" | "IN_PROGRESS" | "EXPECTED" | "WAITING" => 1,
        "SUCCESS" => 2,
        _ => 3,
    }
}

/// PR state icon and color: open, draft, merged, closed.
pub fn pr_icon(state: &str, draft: bool, p: &Palette) -> (Icon, Color32, &'static str) {
    match state {
        "MERGED" => (Icon::PrMerged, p.merged, "Merged"),
        "CLOSED" => (Icon::PrClosed, p.closed, "Closed"),
        _ if draft => (Icon::PrDraft, p.neutral, "Draft"),
        _ => (Icon::PrOpen, p.open, "Open"),
    }
}

/// `text`, cut from the front with "…" until it fits in `max_w`. Keeps the
/// end of a file path, where the file name is, readable.
pub fn elide_front(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32) -> String {
    let fits = |t: &str| painter.layout_no_wrap(t.into(), font.clone(), egui::Color32::WHITE).size().x <= max_w;
    if fits(text) {
        return text.to_string();
    }
    let starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    // Shortest cut that fits, found by bisection.
    let (mut lo, mut hi) = (1, starts.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if fits(&format!("…{}", &text[starts[mid]..])) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    match starts.get(lo) {
        Some(&i) => format!("…{}", &text[i..]),
        None => "…".into(),
    }
}
