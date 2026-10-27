//! Screens and shared widgets, styled after github.com.

mod detail;
mod list;

pub(crate) use detail::{fold_header, keep_still};

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
    let files = (app.tab != crate::app::Tab::Conversation || app.commit.is_some()) && app.selected.is_some();
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
        egui::Panel::left("list-rail").resizable(false).exact_size(52.0).frame(Frame::new().fill(p.canvas_subtle).inner_margin(Margin::symmetric(8, 12))),
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
    // No frame stroke: the panel's own separator is the one bottom rule.
    egui::Panel::top("header").exact_size(theme::HEADER_H).frame(Frame::new().fill(p.header).inner_margin(Margin::symmetric(16, 0))).show(ui, |ui| {
        drag_window(ui);
        ui.horizontal_centered(|ui| {
            // Room for the macOS traffic lights, which sit over the header.
            // Full screen hides them, so it needs no gap.
            let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if cfg!(target_os = "macos") && !fullscreen {
                ui.add_space(TRAFFIC_LIGHTS_W);
            }
            // Item spacing is the one gap between header items.
            let list_open = if app.narrow { app.list_open_narrow } else { app.panels.list };
            let (icon, tip) =
                if list_open { (Icon::SidebarLeftClose, "Hide pull request list (⌘B)") } else { (Icon::SidebarLeftOpen, "Show pull request list (⌘B)") };
            if icon_button(ui, icon, tip, p).clicked() {
                app.actions.push(Action::TogglePanel(Panel::List));
            }
            icons::show(ui, Icon::PrOpen, 20.0, p.fg);
            ui.label(RichText::new("Pull requests").font(theme::bold(theme::TEXT_LARGE)).color(p.fg));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !app.viewer.is_empty() {
                    let url = format!("https://github.com/{}.png?size=64", app.viewer);
                    let r = avatar(ui, &url, theme::CONTROL_H);
                    let r = r.interact(Sense::click()).tip(format!("Signed in as {} ({})", app.viewer, app.auth_source.map(|s| s.describe()).unwrap_or("")));
                    r.context_menu(|ui| {
                        if ui.button("Sign out").clicked() {
                            app.actions.push(Action::SignOut);
                        }
                    });
                }
                if icon_button(ui, Icon::Gear, "Settings", p).clicked() {
                    app.actions.push(Action::OpenSettings);
                }
                if icon_button(ui, Icon::Question, "Help and keyboard shortcuts (⌘?)", p).clicked() {
                    app.actions.push(Action::ToggleHelp);
                }
                let loading = app.lists.values().any(|l| l.loading);
                if loading {
                    // Same button frame, with a spinner in it.
                    let (rect, _) = ui.allocate_exact_size(vec2(theme::CONTROL_H, theme::CONTROL_H), Sense::hover());
                    ui.painter().rect(rect, theme::RADIUS, p.btn_bg, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                    egui::Spinner::new()
                        .size(theme::ICON)
                        .color(p.fg_muted)
                        .paint_at(ui, Rect::from_center_size(rect.center(), vec2(theme::ICON, theme::ICON)));
                } else if icon_button(ui, Icon::Sync, "Refresh (⌘R)", p).clicked() {
                    app.actions.push(Action::Refresh);
                }
            });
        });
    });
}

/// Width the macOS close / minimize / zoom buttons take at the window's left.
const TRAFFIC_LIGHTS_W: f32 = 64.0;

/// With no title bar, dragging empty space here moves the window and a
/// double-click zooms it, like a title bar. Call before drawing the
/// controls, so they still get their clicks.
fn drag_window(ui: &mut Ui) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let r = ui.interact(ui.max_rect(), ui.id().with("drag-window"), Sense::click_and_drag());
    if r.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if r.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
    }
}

pub fn sign_in(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    egui::CentralPanel::default().frame(Frame::new().fill(p.canvas_subtle)).show(ui, |ui| {
        // No header here: the top strip stands in for the title bar.
        let strip = Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), 28.0));
        drag_window(&mut ui.new_child(egui::UiBuilder::new().max_rect(strip)));
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 190.0).max(40.0));
            icons::show(ui, Icon::PrOpen, 48.0, p.fg);
            ui.add_space(12.0);
            ui.label(RichText::new("Sign in to GitHub").font(theme::body(theme::TEXT_DISPLAY)).color(p.fg));
            ui.add_space(16.0);
            if app.finding_token {
                ui.add(egui::Spinner::new().size(20.0));
                ui.label(RichText::new("Looking for your GitHub login…").color(p.fg_muted));
                return;
            }
            Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(theme::RADIUS).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(340.0);
                ui.vertical(|ui| {
                    if let Some(e) = &app.token_error {
                        ui.label(RichText::new(e).color(p.closed).size(theme::TEXT_DENSE));
                        ui.add_space(8.0);
                    }
                    ui.label(
                        RichText::new("No gh CLI login or GH_TOKEN was found. Paste a personal access token with the repo and read:org scopes.")
                            .size(theme::TEXT_DENSE)
                            .color(p.fg_muted),
                    );
                    ui.add_space(10.0);
                    ui.label(RichText::new("Personal access token").font(theme::bold(theme::TEXT_BODY)));
                    let r = ui.add(egui::TextEdit::singleline(&mut app.token_input).password(true).desired_width(f32::INFINITY).margin(vec2(8.0, 6.0)));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        app.actions.push(Action::SubmitToken);
                    }
                    ui.add_space(12.0);
                    let w = ui.available_width();
                    if ui.add_sized([w, theme::CONTROL_H], primary_button("Sign in", p)).clicked() {
                        app.actions.push(Action::SubmitToken);
                    }
                });
            });
            ui.add_space(8.0);
            if ui.add(button("Use my gh login", p)).tip("Look for a gh CLI login or GH_TOKEN again").clicked() {
                app.actions.push(Action::FindToken);
            }
            ui.add_space(16.0);
            Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(theme::RADIUS).inner_margin(Margin::same(14)).show(ui, |ui| {
                ui.set_width(340.0);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Need a token?").size(theme::TEXT_DENSE));
                        if link(ui, "Create one on GitHub.", theme::TEXT_DENSE, p).clicked() {
                            app.actions
                                .push(Action::OpenUrl("https://github.com/settings/tokens/new?scopes=repo,read:org&description=GitHub%20PRs%20desktop".into()));
                        }
                    });
                });
            });
        });
    });
}

// ---------- Dialogs ----------

/// Width of every dialog.
pub const MODAL_W: f32 = 560.0;
/// Side padding of every dialog section.
pub const MODAL_PAD: f32 = 16.0;
/// ⌘K rows: height, and how far their icon sits in from the highlight's edge.
const GOTO_ROW_H: f32 = 30.0;
const GOTO_ROW_INSET: f32 = 10.0;
/// The ⌘K text box's inner margin. Its text still starts at the box's
/// left edge, so rows don't add it to their text column.
const GOTO_TEXT_MARGIN: f32 = 4.0;

/// The one dialog frame: overlay fill, 1px border, overlay radius and
/// shadow. No padding; content goes in `modal_section`s.
pub fn modal_frame(p: &Palette) -> Frame {
    Frame::new().fill(p.overlay).stroke(Stroke::new(1.0, p.border)).corner_radius(theme::RADIUS_OVERLAY).shadow(theme::overlay_shadow(p))
}

/// A Primer dialog near the top of the window, over a dimmed page. Build it
/// from `modal_section`s split by `modal_divider`s.
pub fn modal<R>(ctx: &egui::Context, id: &str, add: impl FnOnce(&mut Ui) -> R) -> egui::ModalResponse<R> {
    modal_sized(ctx, id, MODAL_W, add)
}

/// `modal`, at a width other than the usual one.
pub fn modal_sized<R>(ctx: &egui::Context, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> egui::ModalResponse<R> {
    let p = theme::palette(ctx);
    let id = egui::Id::new(id);
    let area = egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, vec2(0.0, 96.0));
    egui::Modal::new(id).area(area).frame(modal_frame(p)).backdrop_color(p.backdrop).show(ctx, |ui| {
        ui.set_width(width);
        // Sections and dividers butt together; each section pads itself.
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui)
    })
}

/// One padded band of a dialog: header, body or footer.
pub fn modal_section<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let spacing = ui.ctx().global_style().spacing.item_spacing;
    Frame::new()
        .inner_margin(Margin::symmetric(MODAL_PAD as i8, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = spacing;
            add(ui)
        })
        .inner
}

/// A dialog's title band: the title on the left, and on the right an X
/// with `Esc`, its key, beside it. Every dialog uses it, so titles and the
/// first divider sit at the same height. True when the X is clicked.
pub fn modal_header(ui: &mut Ui, title: &str, p: &Palette) -> bool {
    modal_section(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(theme::bold(theme::TEXT_TITLE)).color(p.fg));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let closed = icon_button_plain(ui, Icon::X, "Close", p).clicked();
                let size = key_cap_size(ui.painter(), "Esc", p);
                let (r, _) = ui.allocate_exact_size(vec2(size.x, theme::CONTROL_H), Sense::hover());
                key_cap(ui.painter(), r.right_center(), "Esc", p);
                closed
            })
            .inner
        })
        .inner
    })
}

/// The scroll bar of a dialog body: always shown while there's more to
/// scroll, and drawn in the body's right padding, clear of the content.
const MODAL_BAR_W: f32 = 6.0;
const MODAL_BAR_MARGIN: f32 = 4.0;

/// A dialog's body that scrolls when the window is too short for it. Pads
/// itself like `modal_section`; the header and footer stay put.
pub fn modal_scroll<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    let spacing = ui.ctx().global_style().spacing.item_spacing;
    // Room for the dialog's top offset, a header, a footer and a margin below.
    let max_h = (ui.ctx().content_rect().height() - 260.0).max(160.0);
    Frame::new()
        .inner_margin(Margin { left: MODAL_PAD as i8, right: 0, top: 12, bottom: 12 })
        .show(ui, |ui| {
            // A modal's space is last frame's size; with nothing below the
            // body to push it, it would never grow taller. Claim the room.
            ui.set_max_height(max_h);
            let widgets = ui.visuals().widgets.clone();
            let mut scroll = egui::style::ScrollStyle::thin();
            scroll.bar_width = MODAL_BAR_W;
            scroll.floating_width = MODAL_BAR_W;
            // The body's right padding is the gutter; the bar takes no more.
            scroll.floating_allocated_width = 0.0;
            scroll.bar_outer_margin = MODAL_BAR_MARGIN;
            // The handle is there at rest, with no track behind it.
            scroll.dormant_background_opacity = 0.0;
            scroll.active_background_opacity = 0.0;
            scroll.interact_background_opacity = 0.0;
            scroll.interact_handle_opacity = 1.0;
            ui.spacing_mut().scroll = scroll;
            let v = &mut ui.visuals_mut().widgets;
            v.inactive.bg_fill = p.border_emphasis;
            v.hovered.bg_fill = p.fg_muted;
            v.active.bg_fill = p.fg_muted;
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .auto_shrink([false, true])
                .content_margin(Margin { right: MODAL_PAD as i8, ..Margin::ZERO })
                .show(ui, |ui| {
                    ui.visuals_mut().widgets = widgets;
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = spacing;
                    add(ui)
                })
                .inner
        })
        .inner
}

/// A full-width rule between dialog sections.
pub fn modal_divider(ui: &mut Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border_muted));
}

/// The ⌘K highlight: hover fill plus an accent bar, inset so the bar
/// stays clear of the fill's rounded corner.
fn goto_highlight(painter: &egui::Painter, r: Rect, p: &Palette) {
    painter.rect_filled(r, theme::RADIUS, p.hover_row);
    painter.rect_filled(Rect::from_min_size(r.min + vec2(0.0, 5.0), vec2(2.0, r.height() - 10.0)), 1.0, p.accent_emphasis);
}

/// Width of the drawn ↵ arrow, and its gap from text before it.
const ENTER_W: f32 = 8.0;
const ENTER_GAP: f32 = 2.0;

/// A key cap's text, with a trailing `↵` split off (it's drawn, not typed).
fn key_cap_parts(painter: &egui::Painter, key: &str, p: &Palette) -> (std::sync::Arc<egui::Galley>, bool, f32) {
    let (text, enter) = key.strip_suffix('↵').map_or((key, false), |t| (t, true));
    let g = painter.layout_no_wrap(text.to_string(), theme::body(theme::TEXT_SMALL), p.fg_muted);
    let inner = g.size().x + if enter { ENTER_W + if text.is_empty() { 0.0 } else { ENTER_GAP } } else { 0.0 };
    (g, enter, inner)
}

/// The size `key_cap` draws `key` at.
pub fn key_cap_size(painter: &egui::Painter, key: &str, p: &Palette) -> egui::Vec2 {
    vec2((key_cap_parts(painter, key, p).2 + 10.0).max(22.0), 20.0)
}

/// A small key cap, like `Esc`, `⌘K` or `⌘↵`, with its right edge at
/// `right_center`. A trailing `↵` is drawn as an arrow, since system fonts
/// lack the glyph. Returns where it was drawn.
pub fn key_cap(painter: &egui::Painter, right_center: egui::Pos2, key: &str, p: &Palette) -> Rect {
    let size = key_cap_size(painter, key, p);
    let (g, enter, inner) = key_cap_parts(painter, key, p);
    let r = Rect::from_min_size(pos2(right_center.x - size.x, right_center.y - size.y / 2.0), size);
    painter.rect(r, theme::RADIUS_SMALL, p.canvas_subtle, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let left = r.center().x - inner / 2.0;
    let text_w = g.size().x;
    if text_w > 0.0 {
        painter.galley(pos2(left, r.center().y - g.size().y / 2.0), g, p.fg_muted);
    }
    if enter {
        // Down from the top right, then left, with an arrowhead.
        let c = pos2(left + inner - ENTER_W / 2.0, r.center().y);
        let s = Stroke::new(1.25, p.fg_muted);
        let (top, corner, tip) = (pos2(c.x + 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 2.0), pos2(c.x - 4.0, c.y + 2.0));
        painter.add(egui::Shape::line(vec![top, corner, tip], s));
        painter.add(egui::Shape::line(vec![tip + vec2(3.0, -3.0), tip, tip + vec2(3.0, 3.0)], s));
    }
    r
}

/// A path with the home folder shown as `~`.
fn tilde(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(|rest| rest.to_path_buf())) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
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
/// Settings: which owners and repos to leave out of every list. Offers your
/// organizations, every owner and repo in the lists loaded so far, and
/// whatever is already ignored (its PRs are gone from the lists).
pub fn settings_box(app: &mut App, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }
    // Owner -> its repos, one entry per owner however GitHub cased it.
    let mut owners: Vec<(String, Vec<String>)> = Vec::new();
    let mut add = |entry: &str| {
        let owner = entry.split_once('/').map_or(entry, |(o, _)| o);
        if owner.is_empty() {
            return;
        }
        let i = owners.iter().position(|(o, _)| o.eq_ignore_ascii_case(owner)).unwrap_or_else(|| {
            owners.push((owner.to_string(), Vec::new()));
            owners.len() - 1
        });
        let repos = &mut owners[i].1;
        if entry.contains('/') && !repos.iter().any(|r| r.eq_ignore_ascii_case(entry)) {
            repos.push(entry.to_string());
        }
    };
    add(&app.viewer);
    for org in app.orgs.iter().flatten() {
        add(org);
    }
    for list in app.lists.values().filter_map(|l| l.data.as_ref()) {
        for row in &list.rows {
            add(&row.repository.name_with_owner);
        }
    }
    for entry in &app.settings.ignored_repositories {
        add(entry.trim());
    }
    owners.sort_by_key(|(o, _)| o.to_lowercase());
    for (_, repos) in &mut owners {
        repos.sort_by_key(|r| r.to_lowercase());
    }

    let p = theme::palette(ctx);
    let mut changes = Vec::new();
    let mut done = false;
    let resp = modal(ctx, "settings", |ui| {
        done = modal_header(ui, "Settings", p);
        modal_divider(ui, p);
        modal_scroll(ui, |ui| {
            ui.label(RichText::new("Hidden owners and repos").font(theme::bold(theme::TEXT_BODY)).color(p.fg));
            ui.label(RichText::new("Pull requests from checked owners and repos are left out of every list.").size(theme::TEXT_DENSE).color(p.fg_muted));
            // Rows are control height already; no gap between them.
            ui.spacing_mut().item_spacing.y = 0.0;
            for (owner, repos) in &owners {
                let mut on = app.settings.is_ignored(owner);
                if checkbox(ui, &mut on, RichText::new(owner).font(theme::bold(theme::TEXT_DENSE)).color(p.fg), p).changed() {
                    changes.push((owner.clone(), on));
                }
                let owner_on = on;
                for repo in repos {
                    ui.horizontal(|ui| {
                        // Repo boxes line up under the owner's name.
                        ui.add_space(theme::CHECK_BOX);
                        let name = repo.split_once('/').map_or(repo.as_str(), |(_, n)| n);
                        // A whole owner ignored covers its repos.
                        let mut on = owner_on || app.settings.is_ignored(repo);
                        let resp = ui.add_enabled_ui(!owner_on, |ui| checkbox(ui, &mut on, RichText::new(name).size(theme::TEXT_DENSE).color(p.fg), p)).inner;
                        if resp.changed() {
                            changes.push((repo.clone(), on));
                        }
                    });
                }
            }
        });
        modal_divider(ui, p);
        modal_section(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(path) = crate::settings::path() {
                    ui.label(RichText::new(format!("Saved in {}", tilde(&path))).size(theme::TEXT_SMALL).color(p.fg_muted));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button("Done", p)).clicked() {
                        done = true;
                    }
                });
            });
        });
    });
    for (entry, on) in changes {
        app.actions.push(Action::SetIgnored(entry, on));
    }
    if done || resp.should_close() {
        app.settings_open = false;
    }
}

/// The help window is wide enough for its two columns.
const HELP_W: f32 = 800.0;

/// The help window's left column: how the app works, in short.
const HELP_TOPICS: &[(&str, &str)] = &[
    (
        "Your pull requests",
        "The list on the left has five views: Created, Assigned, Mentioned, Review requests and Involved. Switch between open and closed ones at its top.",
    ),
    ("Search", "The search box takes any GitHub search, like repo:owner/name label:bug. Press Enter to run it."),
    ("Open one", "Click a row to open it here. Double-click opens it on github.com. Right-click to copy its link."),
    ("Review and merge", "Comment, approve, request changes and merge from the Conversation tab. Files changed shows the diff."),
    ("Hide noise", "Settings (the gear, top right) leaves owners and repos out of every list."),
    ("Staying current", "Lists refresh every five minutes and when you come back to the window."),
    ("Sign out", "Right-click your avatar, top right."),
];

/// What a shortcut does, and its alternatives. Keys in one alternative
/// are pressed together.
type Shortcut = (&'static str, &'static [&'static [&'static str]]);

/// The help window's shortcuts, by group.
const HELP_KEYS: &[(&str, &[Shortcut])] = &[
    (
        "Move around",
        &[
            ("Next / previous pull request", &[&["j", "k"], &["↓", "↑"]]),
            ("Go to a pull request by number or link", &[&["⌘K"]]),
            ("Search", &[&["/"], &["⌘F"]]),
            ("Switch tabs, Conversation to New", &[&["⌘1", "⌘2", "⌘3", "⌘4", "⌘5"]]),
            ("Back from a commit", &[&["Esc"]]),
        ],
    ),
    (
        "Pull request",
        &[("Open on GitHub", &[&["⌘O"]]), ("Copy link", &[&["c"], &["⇧⌘C"]]), ("Send a comment or reply", &[&["⌘↵"]]), ("Cancel a reply", &[&["Esc"]])],
    ),
    (
        "Reviewing a diff",
        &[
            ("Next / previous file", &[&["n", "p"]]),
            ("Next file you haven't viewed", &[&["u"]]),
            ("Mark the file viewed", &[&["v"]]),
            ("Previous / next commit, on a commit's page", &[&["[", "]"]]),
        ],
    ),
    (
        "Window",
        &[
            ("Show or hide the pull request list", &[&["⌘B"]]),
            ("Show or hide details or the file list", &[&["⇧⌘B"]]),
            ("Refresh", &[&["⌘R"]]),
            ("This help", &[&["⌘?"], &["?"]]),
            ("Close the window", &[&["⌘W"]]),
        ],
    ),
];

/// ⌘? or ?: what the app does, and every keyboard shortcut.
pub fn help_box(app: &mut App, ctx: &egui::Context) {
    if !app.help_open {
        return;
    }
    let p = theme::palette(ctx);
    let mut closed = false;
    let resp = modal_sized(ctx, "help", HELP_W, |ui| {
        closed = modal_header(ui, "Help", p);
        modal_divider(ui, p);
        modal_scroll(ui, |ui| {
            // A wide gutter between the columns, normal spacing in them.
            let gap = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 32.0;
            ui.columns(2, |cols| {
                for col in cols.iter_mut() {
                    col.spacing_mut().item_spacing.x = gap;
                }
                // Columns justify by default, which spreads wrapped words out.
                cols[0].with_layout(egui::Layout::top_down(egui::Align::Min), |left| {
                    help_heading(left, "Getting started", p);
                    for (title, text) in HELP_TOPICS {
                        left.add_space(6.0);
                        left.label(RichText::new(*title).font(theme::bold(theme::TEXT_DENSE)).color(p.fg));
                        left.label(RichText::new(*text).size(theme::TEXT_DENSE).color(p.fg_muted));
                    }
                });
                let right = &mut cols[1];
                help_heading(right, "Keyboard shortcuts", p);
                for (i, (group, rows)) in HELP_KEYS.iter().enumerate() {
                    right.add_space(if i == 0 { 6.0 } else { 12.0 });
                    right.label(RichText::new(*group).font(theme::bold(theme::TEXT_DENSE)).color(p.fg));
                    for (what, keys) in *rows {
                        shortcut_row(right, what, keys, p);
                    }
                }
                if !cfg!(target_os = "macos") {
                    right.add_space(12.0);
                    right.label(RichText::new("On Windows and Linux, use Ctrl instead of ⌘.").size(theme::TEXT_SMALL).color(p.fg_muted));
                }
            });
        });
    });
    if closed || resp.should_close() {
        app.help_open = false;
    }
}

fn help_heading(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text).font(theme::bold(theme::TEXT_BODY)).color(p.fg));
}

/// One help row: what it does on the left, its key caps on the right, with
/// "or" between alternatives.
fn shortcut_row(ui: &mut Ui, what: &str, keys: &[&[&str]], p: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let painter = ui.painter();
    let mut x = r.right();
    for (i, alt) in keys.iter().rev().enumerate() {
        if i > 0 {
            let g = painter.layout_no_wrap("or".into(), theme::body(theme::TEXT_SMALL), p.fg_muted);
            x -= g.size().x + 6.0;
            painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, p.fg_muted);
            x -= 6.0;
        }
        for key in alt.iter().rev() {
            x = key_cap(painter, pos2(x, r.center().y), key, p).left() - 4.0;
        }
    }
    let g = painter.layout_no_wrap(what.into(), theme::body(theme::TEXT_DENSE), p.fg);
    painter.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, p.fg);
}

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
    let resp = modal(ctx, "goto", |ui| {
        modal_section(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::PrOpen, theme::ICON, p.fg_muted);
                let before = g.text.clone();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut g.text)
                        .frame(Frame::NONE)
                        .font(theme::body(theme::TEXT_LARGE))
                        .hint_text("Go to pull request: 123, owner/repo#123, or a link")
                        .margin(vec2(GOTO_TEXT_MARGIN, 2.0))
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
        modal_divider(ui, p);
        // Rows sit GOTO_ROW_INSET outside the dialog's padding, so their icon
        // and text land on the input's icon and text columns.
        Frame::new().inner_margin(Margin::same((MODAL_PAD - GOTO_ROW_INSET) as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let text_x = GOTO_ROW_INSET + theme::ICON + ui.spacing().item_spacing.x;
            let icon_at = |r: Rect| Rect::from_min_size(pos2(r.left() + GOTO_ROW_INSET, r.center().y - theme::ICON / 2.0), vec2(theme::ICON, theme::ICON));
            // Loading, error and hint lines use the same row and columns.
            let note = |ui: &mut Ui, spinner: bool, icon: Option<(Icon, Color32)>, text: &str, color: Color32| {
                let w = ui.available_width();
                let g = ui.painter().layout(text.to_string(), theme::body(theme::TEXT_DENSE), color, w - text_x - GOTO_ROW_INSET);
                // A long error wraps; the row grows to fit it.
                let h = GOTO_ROW_H.max(g.size().y + GOTO_ROW_H - theme::ICON);
                let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                if spinner {
                    egui::Spinner::new().size(theme::ICON).color(p.fg_muted).paint_at(ui, icon_at(r));
                }
                if let Some((icon, c)) = icon {
                    icons::paint(ui.painter(), icon_at(r), icon, c);
                }
                ui.painter().galley(pos2(r.left() + text_x, r.center().y - g.size().y / 2.0), g, color);
            };
            if g.loading {
                note(ui, true, None, "Opening…", p.fg_muted);
            } else if let Some(e) = &g.error {
                note(ui, false, Some((Icon::X, p.closed)), e, p.closed);
            } else if !results.is_empty() {
                for (i, res) in results.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GOTO_ROW_H), Sense::click());
                    // Hover moves the highlight, but only when the mouse
                    // moves, so a resting pointer doesn't fight the keys.
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) && i != sel {
                        g.sel = Some(res.key());
                        ui.ctx().request_repaint();
                    }
                    let on = i == sel;
                    if on {
                        goto_highlight(ui.painter(), r, p);
                    }
                    let mut job = egui::text::LayoutJob::default();
                    let muted = |c| egui::TextFormat { font_id: theme::body(theme::TEXT_DENSE), color: c, ..Default::default() };
                    let (icon, color) = match res {
                        GoResult::Pr(pr) => {
                            job.append(&format!("{}#{} ", pr.repository.name_with_owner, pr.number), 0.0, muted(p.fg_muted));
                            append_title(&mut job, &pr.title, theme::TEXT_DENSE, true, 0.0, p);
                            let (icon, color, _) = pr_icon(&pr.state, pr.is_draft, p);
                            (icon, color)
                        }
                        GoResult::Ref(repo, n) => {
                            job.append(
                                &format!("{repo}#{n}"),
                                0.0,
                                egui::TextFormat { font_id: theme::bold(theme::TEXT_DENSE), color: p.fg, ..Default::default() },
                            );
                            job.append("Load pull request", 8.0, muted(p.fg_muted));
                            (Icon::PrOpen, p.fg_muted)
                        }
                    };
                    icons::paint(ui.painter(), icon_at(r), icon, color);
                    // The highlighted row gets the Enter key cap.
                    let hint_w = if on { key_cap(ui.painter(), pos2(r.right() - GOTO_ROW_INSET, r.center().y), "↵", p).width() + 16.0 } else { 0.0 };
                    let (tg, _) = cut_job(ui.painter(), job, r.width() - text_x - GOTO_ROW_INSET - hint_w);
                    ui.painter().galley(pos2(r.left() + text_x, r.center().y - tg.size().y / 2.0), tg, p.fg);
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
            } else if !g.text.trim().is_empty() {
                // Nothing loaded matches: offer GitHub's search for it.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GOTO_ROW_H), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Search pull requests"));
                goto_highlight(ui.painter(), r, p);
                icons::paint(ui.painter(), icon_at(r), Icon::Search, p.fg_muted);
                let hint_w = key_cap(ui.painter(), pos2(r.right() - GOTO_ROW_INSET, r.center().y), "↵", p).width() + 16.0;
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    "Search pull requests for ",
                    0.0,
                    egui::TextFormat { font_id: theme::body(theme::TEXT_DENSE), color: p.fg_muted, ..Default::default() },
                );
                job.append(
                    &format!("\"{}\"", g.text.trim()),
                    0.0,
                    egui::TextFormat { font_id: theme::bold(theme::TEXT_DENSE), color: p.fg, ..Default::default() },
                );
                let (tg, _) = cut_job(ui.painter(), job, r.width() - text_x - GOTO_ROW_INSET - hint_w);
                ui.painter().galley(pos2(r.left() + text_x, r.center().y - tg.size().y / 2.0), tg, p.fg);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    search_for = Some(g.text.trim().to_string());
                }
            } else {
                let hint = match &default_repo {
                    Some(r) => format!("A bare number opens a PR in {r}. Esc to close."),
                    None => "Include the repo, like owner/repo#123. Esc to close.".to_string(),
                };
                note(ui, false, None, &hint, p.fg_muted);
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
    Frame::new().fill(p.danger_subtle).stroke(Stroke::new(1.0, p.danger_border)).corner_radius(theme::RADIUS).inner_margin(Margin::symmetric(16, 12)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icons::show(ui, Icon::X, 16.0, p.closed);
                // The text wraps in what's left beside Retry.
                let text_w = (ui.available_width() - 80.0).max(80.0);
                ui.allocate_ui_with_layout(vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(title).font(theme::bold(theme::TEXT_BODY)).color(p.fg));
                    ui.label(RichText::new(friendly_error(raw)).size(theme::TEXT_DENSE).color(p.fg_muted)).tip(raw);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    retry = ui.add(button("Retry", p)).clicked();
                });
            });
        },
    );
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
            Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(theme::RADIUS).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Sync, 14.0, p.attention);
                    ui.label(RichText::new("Couldn't refresh · showing saved data").size(theme::TEXT_SMALL).color(p.fg_muted))
                        .tip(format!("{}\n{raw}", friendly_error(raw)));
                    retry = link(ui, "Retry", theme::TEXT_SMALL, p).clicked();
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
    egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0)).show(ctx, |ui| {
        // Primer toast: dark in both themes, colored icon, white text.
        Frame::new()
            .fill(p.tooltip)
            .corner_radius(theme::RADIUS)
            .inner_margin(Margin::symmetric(14, 10))
            .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(60) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon, color) = if t.error { (Icon::X, p.tooltip_danger) } else { (Icon::Check, p.tooltip_success) };
                    icons::show(ui, icon, theme::ICON, color);
                    ui.label(RichText::new(&t.text).color(p.tooltip_fg).size(theme::TEXT_BODY));
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
    let shape =
        |rect: egui::Rect, fill: Color32, stroke: Stroke| egui::Shape::Rect(egui::epaint::RectShape::new(rect, radius, fill, stroke, egui::StrokeKind::Inside));
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
    pill(ui, name, theme::bold(theme::TEXT_SMALL), bg, fg, border);
}

pub fn primary_button(text: &str, p: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).font(theme::bold(theme::TEXT_BODY)).color(p.fg_on_emphasis))
        .fill(p.btn_primary)
        .stroke(Stroke::new(1.0, p.btn_primary.gamma_multiply(0.85)))
        .corner_radius(theme::RADIUS)
        .min_size(vec2(0.0, theme::CONTROL_H))
}

/// A primary button that may be disabled. Use this, not
/// `ui.add_enabled(.., primary_button(..))`: egui's disabled fade turns
/// green pale on white and barely changes it on black. This draws one
/// muted look, the same in both themes.
pub fn primary_action(ui: &mut Ui, text: &str, enabled: bool, p: &Palette) -> egui::Response {
    if enabled {
        return ui.add(primary_button(text, p));
    }
    let b = egui::Button::new(RichText::new(text).font(theme::bold(theme::TEXT_BODY)).color(p.fg_disabled))
        .fill(p.btn_disabled)
        .stroke(Stroke::new(1.0, p.border_muted))
        .corner_radius(theme::RADIUS)
        .min_size(vec2(0.0, theme::CONTROL_H));
    // Still disabled for clicks and screen readers, just not faded.
    ui.add_enabled_ui(false, |ui| {
        ui.set_opacity(1.0);
        ui.add(b)
    })
    .inner
}

pub fn button(text: &str, p: &Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).font(theme::bold(theme::TEXT_BODY)).color(p.fg))
        .fill(p.btn_bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .min_size(vec2(0.0, theme::CONTROL_H))
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

/// Widest a tooltip gets before it wraps.
const TIP_MAX_W: f32 = 320.0;

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
    const GAP: f32 = 6.0;
    // Below a header button means below the header's rule, not across it.
    let header_bottom = screen.top() + theme::HEADER_H;
    let below = if r.bottom() <= header_bottom { header_bottom + GAP } else { r.bottom() + GAP };
    // Its real width, so a centered tip near the right edge is still centered.
    let half_w = resp.ctx.debug_painter().layout(text.to_string(), theme::body(theme::TEXT_SMALL), p.tooltip_fg, TIP_MAX_W).size().x / 2.0 + 8.0;
    // To the right if there's room, else below; near the right edge, below
    // and right-aligned so it stays inside the window.
    let (pos, pivot) = if side == Side::Above && r.top() > screen.top() + 40.0 {
        (egui::pos2(r.left(), r.top() - GAP), egui::Align2::LEFT_BOTTOM)
    } else if side == Side::Below {
        (egui::pos2(r.left(), below), egui::Align2::LEFT_TOP)
    } else if r.right() + TIP_MAX_W + 20.0 < screen.right() {
        (egui::pos2(r.right() + GAP, r.center().y), egui::Align2::LEFT_CENTER)
    } else if r.center().x + half_w + GAP < screen.right() {
        (egui::pos2(r.center().x, below), egui::Align2::CENTER_TOP)
    } else {
        (egui::pos2(r.right(), below), egui::Align2::RIGHT_TOP)
    };
    egui::Area::new(resp.id.with("tip")).order(egui::Order::Tooltip).fixed_pos(pos).pivot(pivot).interactable(false).show(&resp.ctx, |ui| {
        Frame::new().fill(p.tooltip).corner_radius(theme::RADIUS).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.set_max_width(TIP_MAX_W);
            ui.label(RichText::new(text).size(theme::TEXT_SMALL).color(p.tooltip_fg));
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
    let (rect, resp) = ui.allocate_exact_size(vec2(theme::CONTROL_H, theme::CONTROL_H), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    tip_at(&resp, label, if above { Side::Above } else { Side::Auto });
    ui.painter().rect(rect, theme::RADIUS, button_fill(&resp, p.btn_bg, p), Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), icon_rect(rect), icon, p.fg_muted);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An icon button with no frame until hovered, like GitHub's "⋯".
pub fn icon_button_plain(ui: &mut Ui, icon: Icon, label: &str, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(theme::CONTROL_H, theme::CONTROL_H), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    tip(&resp, label);
    ui.painter().rect_filled(rect, theme::RADIUS, button_fill(&resp, Color32::TRANSPARENT, p));
    icons::paint(ui.painter(), icon_rect(rect), icon, p.fg_muted);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A button's fill: `rest`, then the shared hover and pressed fills.
pub fn button_fill(resp: &egui::Response, rest: Color32, p: &Palette) -> Color32 {
    if resp.is_pointer_button_down_on() {
        p.btn_active
    } else if resp.hovered() {
        p.btn_hover
    } else {
        rest
    }
}

/// The icon square centered in a control.
pub fn icon_rect(control: Rect) -> Rect {
    Rect::from_center_size(control.center(), vec2(theme::ICON, theme::ICON))
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
pub fn boxed<R>(ui: &mut Ui, header_fill: Color32, border: Color32, header: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = theme::palette(ui.ctx()).canvas;
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(theme::RADIUS)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new()
                .fill(header_fill)
                .corner_radius(CornerRadius { nw: theme::RADIUS, ne: theme::RADIUS, sw: 0, se: 0 })
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
        .corner_radius(theme::RADIUS)
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

/// Primer's checkbox: a small square with a 1px border, accent fill and a
/// white check when on. The whole row, at control height, is the target.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: RichText, p: &Palette) -> egui::Response {
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width(), egui::TextStyle::Body);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(ui.available_width(), theme::CONTROL_H), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let label = galley.text().to_string();
    let checked = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), checked, &label));
    let b = Rect::from_min_size(pos2(rect.left(), rect.center().y - theme::CHECK_BOX / 2.0), vec2(theme::CHECK_BOX, theme::CHECK_BOX));
    paint_checkbox(ui.painter(), b, *on, resp.hovered(), p);
    let x = b.right() + ui.spacing().item_spacing.x;
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, p.fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Just the checkbox square, for widgets that paint their own row (the
/// diff's "Viewed" button). `b` is the box, `theme::CHECK_BOX` square.
pub fn paint_checkbox(painter: &egui::Painter, b: Rect, on: bool, hovered: bool, p: &Palette) {
    if on {
        painter.rect_filled(b, theme::RADIUS_SMALL, p.accent_emphasis);
        icons::paint(painter, b.shrink(1.5), Icon::Check, p.fg_on_emphasis);
    } else {
        let border = if hovered { p.fg_muted } else { p.border_emphasis };
        painter.rect(b, theme::RADIUS_SMALL, p.canvas, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
}

/// A one-line search or filter box at control height: search icon, text,
/// and a × when `clearable`. Returns the text box's response, and whether
/// the × was clicked (the caller decides what clearing means).
pub fn search_input(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, clearable: bool, clear_label: &str, p: &Palette) -> (egui::Response, bool) {
    let focused = ui.memory(|m| m.has_focus(id));
    // Clicks only: the text field inside is the one Tab stop.
    let (rect, frame) = ui.allocate_exact_size(vec2(ui.available_width(), theme::CONTROL_H), Sense::CLICK);
    let stroke = if focused {
        Stroke::new(2.0, p.accent_emphasis)
    } else if frame.hovered() || ui.rect_contains_pointer(rect) {
        Stroke::new(1.0, p.border_emphasis)
    } else {
        Stroke::new(1.0, p.border)
    };
    ui.painter().rect(rect, theme::RADIUS, p.canvas, stroke, egui::StrokeKind::Inside);
    if frame.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let inner = rect.shrink2(vec2(8.0, 0.0));
    let mut cleared = false;
    let resp = ui
        .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            icons::show(ui, Icon::Search, theme::ICON, p.fg_muted);
            // The × is a square a bit smaller than the box, easy to hit.
            let x_size = theme::CONTROL_H - 6.0;
            let x_w = if clearable { x_size + ui.spacing().item_spacing.x } else { 0.0 };
            let resp = ui.add(
                egui::TextEdit::singleline(text)
                    .id(id)
                    .hint_text(RichText::new(hint).color(p.fg_muted))
                    .frame(Frame::NONE)
                    .font(theme::body(theme::TEXT_BODY))
                    .margin(vec2(4.0, 0.0))
                    // Exactly what's left, so the box never grows to fit.
                    .desired_width((ui.available_width() - x_w).max(20.0)),
            );
            if clearable {
                let (r, x) = ui.allocate_exact_size(vec2(x_size, x_size), Sense::click());
                x.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, clear_label));
                ui.painter().rect_filled(r, theme::RADIUS_SMALL, button_fill(&x, Color32::TRANSPARENT, p));
                icons::paint(ui.painter(), r.shrink(5.0), Icon::X, if x.hovered() { p.fg } else { p.fg_muted });
                cleared = x.tip(clear_label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            }
            resp
        })
        .inner;
    (resp, cleared)
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

/// A status icon that explains itself on hover, like "2 non-required
/// checks failed". Same look as `icons::show` with `status_icon`.
pub fn status_icon_tip(ui: &mut Ui, state: &str, size: f32, tip_text: &str, p: &Palette) -> egui::Response {
    let (icon, color) = status_icon(state, p);
    let resp = icons::show(ui, icon, size, color).interact(Sense::hover());
    tip(&resp, tip_text);
    resp
}

/// Tooltip for a CI icon from the full check list: what failed, and
/// whether it's required, e.g. "2 non-required checks failed · 12 passed".
pub fn checks_tip(checks: &[crate::github::Check]) -> String {
    let count = |required: Option<bool>, rank: u8| checks.iter().filter(|c| check_rank(&c.result) == rank && required.is_none_or(|r| c.required == r)).count();
    let plural = |n: usize, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let mut parts = Vec::new();
    let (req_failed, other_failed) = (count(Some(true), 0), count(Some(false), 0));
    if req_failed > 0 {
        parts.push(plural(req_failed, "required check failed", "required checks failed"));
    }
    if other_failed > 0 {
        parts.push(plural(other_failed, "non-required check failed", "non-required checks failed"));
    }
    let (running, passed, skipped) = (count(None, 1), count(None, 2), count(None, 3));
    if running > 0 {
        parts.push(format!("{running} running"));
    }
    if passed > 0 {
        parts.push(format!("{passed} passed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if parts.is_empty() { "No checks".into() } else { parts.join(" · ") }
}

/// Tooltip for a CI icon when only the overall result is known (list
/// rows). With the merge state it can tell failing checks that block
/// merging from ones that don't.
pub fn ci_tip(state: &str, merge_state: Option<&str>) -> &'static str {
    match (check_rank(state), merge_state) {
        (0, Some("CLEAN" | "HAS_HOOKS" | "UNSTABLE")) => "Checks that aren't required failed. Merging isn't blocked.",
        (0, _) => "Some checks failed",
        (1, _) => "Checks are running",
        (2, _) => "All checks passed",
        _ => "Checks were skipped",
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
    let s = |icon, color, short: &'static str, title: &str, detail: String| Some(MergeStatus { icon, color, short, title: title.to_string(), detail });
    if draft || merge_state == Some("DRAFT") {
        return s(Icon::PrDraft, p.neutral, "Draft", "Draft — not ready to merge", "Mark it ready for review to merge.".into());
    }
    if auto_merge_on || auto_merge.is_some() {
        let method = match auto_merge.map(|a| a.merge_method.as_str()) {
            Some("SQUASH") => "squash and merge",
            Some("REBASE") => "rebase and merge",
            _ => "merge",
        };
        let by = auto_merge.and_then(|a| a.enabled_by.as_ref()).map(|a| format!(" · enabled by {}", a.login)).unwrap_or_default();
        let waiting = if why.is_empty() {
            String::new()
        } else {
            let mut w = why.chars();
            let first = w.next().map(|c| c.to_lowercase().to_string()).unwrap_or_default();
            format!(" Waiting on: {first}{}", w.as_str())
        };
        return s(
            Icon::AutoMerge,
            p.merged,
            "Auto-merge on",
            "Auto-merge enabled",
            format!("GitHub will {method} when all requirements are met{by}.{waiting}"),
        );
    }
    match merge_state.unwrap_or("UNKNOWN") {
        "CLEAN" | "HAS_HOOKS" => s(Icon::Check, p.open, "Ready to merge", "Ready to merge", "All requirements are met.".into()),
        "UNSTABLE" => s(
            Icon::Check,
            p.open,
            "Ready to merge",
            "Ready to merge",
            if why.is_empty() { "Some checks that aren't required are failing.".into() } else { why },
        ),
        "BEHIND" => s(Icon::Sync, p.attention, "Out of date", "Branch is out of date", "Update it with the base branch before merging.".into()),
        "DIRTY" => s(Icon::X, p.closed, "Conflicts", "Merge conflicts", "Resolve conflicts with the base branch before merging.".into()),
        "BLOCKED" => s(
            Icon::X,
            p.closed,
            "Blocked",
            "Merging is blocked",
            if why.is_empty() { "GitHub reports a branch rule is blocking this merge.".into() } else { why },
        ),
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
