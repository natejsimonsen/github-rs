//! Right side: the pull request page. Header, tabs, and each tab's content.

use super::{
    CheckSummary, check_rank, detail_merge_status, merge_status, avatar, boxed, button, icon_button, label_pill, link, link_styled, pill, plain_box, pr_icon,
    primary_button, row_separator, status_icon,
};
use super::Tip;
use crate::app::{Action, App, Panel, Tab};
use crate::github::{Check, Comment, Event, EventCommit, PrDetail, PrSummary, Review, Reviewer, Thread};
use crate::icons::{self, Icon};
use crate::theme::{self, Palette};
use crate::{diff, util};
use egui::{Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::sync::Arc;

const AVATAR: f32 = 40.0;
const GUTTER: f32 = AVATAR + 16.0;

pub fn show(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let Some(sel) = app.selected.clone() else {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 2.0 - 60.0);
            icons::show(ui, Icon::PrOpen, 32.0, p.fg_muted);
            ui.add_space(8.0);
            let data = app.current_list().and_then(|l| l.data.as_ref());
            let empty = data.is_some_and(|d| d.rows.is_empty());
            if empty {
                // The list already says why it's empty; just point the way out.
                ui.label(RichText::new("No pull request selected").font(theme::bold(20.0)));
                ui.label(RichText::new(EMPTY_HINT).color(p.fg_muted).size(13.0));
            } else {
                ui.label(RichText::new("No pull request selected").font(theme::bold(20.0)));
                ui.label(RichText::new(EMPTY_HINT).color(p.fg_muted).size(13.0));
            }
        });
        return;
    };
    let entry = app.details.get(&sel.id);
    let detail = entry.and_then(|d| d.data.clone());
    let error = entry.and_then(|d| d.error.clone());
    let loading = entry.is_some_and(|d| d.loading);
    let files = app.files.get(&sel.id).and_then(|f| f.data.clone());

    // Same right edge as the scrolling content below (which leaves room
    // for its scrollbar).
    let full = ui.available_width();
    ui.scope(|ui| {
        ui.set_max_width(full - 14.0);
        header(app, ui, p, &sel, detail.as_deref(), files.as_deref().map(|f| f.len()));
    });
    ui.add_space(8.0);

    if let Some(e) = &error {
        if detail.is_some() {
            // Old data is still good to read: say so quietly, don't push the page.
            if super::stale_note(ui.ctx(), "detail", e) {
                app.actions.push(Action::Select(sel.clone()));
            }
        } else {
            ui.scope(|ui| {
                ui.set_max_width(full - 14.0);
                if super::error_banner(ui, p, "Couldn't load this pull request.", e) {
                    app.actions.push(Action::Select(sel.clone()));
                }
            });
            ui.add_space(8.0);
        }
    }
    let Some(d) = detail else {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            if loading {
                ui.add(egui::Spinner::new().size(24.0));
            }
        });
        return;
    };

    // Only Conversation has the sidebar, so only it gets the toggle.
    if app.tab != Tab::Conversation {
        app.details_folded = false;
    }
    if app.tab == Tab::Files {
        app.detail_scrolled = false;
    }
    match app.tab {
        Tab::Files => match files {
            Some(f) => {
                let src = diff::Source { repo: &d.repository.name_with_owner, head: &d.head_ref_oid };
                diff::show(app, ui, &sel.id, f, src)
            }
            None => {
                let err = app.files.get(&sel.id).and_then(|f| f.error.clone());
                ui.add_space(40.0);
                match err {
                    Some(e) => {
                        if super::error_banner(ui, p, "Couldn't load the changed files.", &e) {
                            app.actions.push(Action::Refresh);
                        }
                    }
                    None => {
                        ui.vertical_centered(|ui| ui.add(egui::Spinner::new().size(24.0).color(p.fg_muted)));
                    }
                }
            }
        },
        tab => {
            if tab == Tab::Conversation {
                details_panel(app, ui, p, &d);
            }
            // A vertical scroll area otherwise grows to fit its widest child,
            // which would slide under the Details sidebar. Pin it to the space
            // that's left, and clip to it.
            let width = ui.available_width();
            let frame_rect = ui.available_rect_before_wrap();
            let mut area = egui::ScrollArea::vertical().id_salt(("detail", &sel.id, tab as u8)).auto_shrink([false, false]).max_width(width);
            let to_bottom = app.shot_scroll == Some(f32::MAX);
            if let Some(y) = app.shot_scroll.filter(|_| !to_bottom) {
                area = area.vertical_scroll_offset(y);
            }
            let mut pinned = ui.new_child(egui::UiBuilder::new().max_rect(frame_rect));
            pinned.set_clip_rect(frame_rect.intersect(ui.clip_rect()));
            let ui = &mut pinned;
            let out = area.show(ui, |ui| {
                    // Room for the scrollbar, so it never covers card borders.
                    ui.set_max_width(width - 14.0);
                    ui.add_space(8.0);
                    match tab {
                        Tab::Conversation => ui.vertical(|ui| timeline(app, ui, p, &d)).inner,
                        Tab::Commits => commits(app, ui, p, &d),
                        Tab::Checks => checks(app, ui, p, &d.checks),
                        Tab::Files => {}
                    }
                    ui.add_space(40.0);
                    if to_bottom {
                        ui.scroll_to_cursor(Some(egui::Align::Max));
                    }
                });
            // Scrolled into the page: the header slims down (next frame), like
            // GitHub's sticky bar. Two thresholds so it doesn't flicker.
            let y = out.state.offset.y;
            app.detail_scrolled = if app.detail_scrolled { y > 10.0 } else { y > 120.0 };
        }
    }
}

fn header(app: &mut App, ui: &mut Ui, p: &Palette, sel: &PrSummary, d: Option<&PrDetail>, file_count: Option<usize>) {
    if app.detail_scrolled {
        slim_header(ui, p, sel, d);
    } else {
        full_header(app, ui, p, sel, d);
    }
    tab_row(app, ui, p, d, file_count);
}

/// Once you scroll into a PR: one line with its state and title, so the
/// page keeps most of the window (GitHub's sticky header).
/// Under "No pull request selected", with or without PRs in the list.
const EMPTY_HINT: &str = "⌘K go to any pull request · j / k move through the list";

fn slim_header(ui: &mut Ui, p: &Palette, sel: &PrSummary, d: Option<&PrDetail>) {
    let state = d.map(|d| d.state.as_str()).unwrap_or(&sel.state);
    let draft = d.map(|d| d.is_draft).unwrap_or(sel.is_draft);
    let (icon, _, word) = pr_icon(state, draft, p);
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        // The state stays in view, like GitHub's sticky header.
        state_badge(ui, icon, badge_color(p, state, draft), word, 24.0);
        ui.add_space(4.0);
        // The number always shows; only the title text gets cut.
        let num = ui.painter().layout_no_wrap(format!("#{}", sel.number), theme::body(15.0), p.fg_muted);
        let mut job = egui::text::LayoutJob::default();
        super::append_title(&mut job, &sel.title, 15.0, true, 0.0, p);
        job.wrap = egui::text::TextWrapping::truncate_at_width((ui.available_width() - num.size().x - 8.0).max(60.0));
        let g = ui.painter().layout_job(job);
        let cut = g.elided;
        let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
        ui.painter().galley(r.min, g, p.fg);
        if cut {
            resp.tip(format!("{} #{}", sel.title, sel.number));
        }
        ui.add_space(4.0);
        let (r, _) = ui.allocate_exact_size(num.size(), Sense::hover());
        ui.painter().galley(r.min, num, p.fg_muted);
    });
    ui.add_space(4.0);
}

fn full_header(app: &mut App, ui: &mut Ui, p: &Palette, sel: &PrSummary, d: Option<&PrDetail>) {
    // Title and buttons.
    let buttons_w = 190.0;
    ui.horizontal_top(|ui| {
        let title_w = (ui.available_width() - buttons_w).max(200.0);
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = title_w;
        super::append_title(&mut job, &sel.title, 26.0, false, 0.0, p);
        // A real space, so the number can wrap on its own.
        job.append(
            &format!(" #{}", sel.number),
            4.0,
            egui::TextFormat { font_id: theme::body(26.0), color: p.fg_muted, ..Default::default() },
        );
        let g = ui.painter().layout_job(job);
        let (rect, _) = ui.allocate_exact_size(vec2(title_w, g.size().y), Sense::hover());
        ui.painter().galley(rect.min, g, p.fg);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if icon_button(ui, Icon::Link, "Copy link (⇧⌘C)", p).clicked() {
                app.actions.push(Action::Copy(sel.url.clone()));
            }
            if ui.add(button("Open on GitHub", p)).tip("Open on GitHub (⌘O)").clicked() {
                app.actions.push(Action::OpenUrl(sel.url.clone()));
            }
        });
    });
    ui.add_space(8.0);

    // State badge and "who wants to merge what into where".
    ui.horizontal_wrapped(|ui| {
        let state = d.map(|d| d.state.as_str()).unwrap_or(&sel.state);
        let draft = d.map(|d| d.is_draft).unwrap_or(sel.is_draft);
        let (icon, _, word) = pr_icon(state, draft, p);
        state_badge(ui, icon, badge_color(p, state, draft), word, 32.0);
        ui.add_space(4.0);
        // The sentence wraps in its own column, never under the badge.
        ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().interact_size.y = 32.0;
        let author = sel.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(author).font(theme::bold(14.0)).color(p.fg));
        if let Some(d) = d {
            let n = d.commits.total_count;
            let commits = format!("{n} commit{}", if n == 1 { "" } else { "s" });
            let verb = if d.state == "MERGED" { "merged" } else { "wants to merge" };
            ui.label(RichText::new(format!("{verb} {commits} into")).color(p.fg_muted));
            branch(ui, p, &d.base_ref_name);
            // "from" wraps together with its branch, never left at a line end.
            let from_w = ui.painter().layout_no_wrap("from".into(), theme::body(14.0), p.fg).size().x;
            let branch_w = ui.painter().layout_no_wrap(d.head_ref_name.clone(), theme::mono(12.0), p.fg).size().x + 20.0;
            if from_w + 4.0 + branch_w > ui.available_size_before_wrap().x {
                ui.end_row();
            }
            ui.label(RichText::new("from").color(p.fg_muted));
            branch(ui, p, &d.head_ref_name);
            if let Some(m) = &d.merged_at {
                ui.label(RichText::new(util::ago(m).replace(' ', "\u{a0}")).color(p.fg_muted));
            }
        } else {
            ui.label(RichText::new(format!("opened {}", util::ago(&sel.created_at))).color(p.fg_muted));
        }
        });
    });
    ui.add_space(12.0);

    // Merge readiness, always visible. Uses list data until details arrive.
    let ms = match d {
        Some(d) => detail_merge_status(p, d, sel.merge_state_status.as_deref()),
        None => merge_status(p, &sel.state, sel.is_draft, sel.merge_state_status.as_deref(), None, sel.auto_merge, String::new()),
    };
    if let Some(ms) = ms {
        merge_bar(app, ui, p, &ms, d);
        ui.add_space(4.0);
    }

}

/// Conversation / Commits / Checks / Files, with the diffstat on the right.
fn tab_row(app: &mut App, ui: &mut Ui, p: &Palette, d: Option<&PrDetail>, file_count: Option<usize>) {
    let tabs = [
        (Tab::Conversation, Icon::Comment, "Conversation", d.map(|d| (d.comments.nodes.len() + d.reviews.nodes.iter().filter(|r| !r.body.trim().is_empty()).count()) as u64)),
        (Tab::Commits, Icon::Commit, "Commits", d.map(|d| d.commits.total_count)),
        (Tab::Checks, Icon::Checklist, "Checks", d.map(|d| d.checks.len() as u64)),
        (Tab::Files, Icon::FileDiff, "Files changed", file_count.map(|n| n as u64).or(d.map(|d| d.changed_files))),
    ];
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // Room on the right: the Details toggle (when the sidebar is hidden)
        // and the diffstat. When space runs out the tabs drop their icons,
        // then the diffstat goes, and only then do the tabs scroll.
        let avail = ui.available_width();
        let toggle_w = if app.details_folded { 36.0 } else { 0.0 };
        let stat = d.map(|d| (format!("+{}", d.additions), format!("−{}", d.deletions)));
        let stat_w = stat.as_ref().map_or(0.0, |(a, r)| {
            let w = |t: &str| ui.painter().layout_no_wrap(t.to_string(), theme::bold(13.0), p.fg).size().x;
            w(a) + w(r) + 50.0 + 12.0 + 16.0
        });
        let natural = |compact: bool| tabs.iter().map(|(tab, _, text, count)| tab_width(ui, p, text, *count, app.tab == *tab, compact)).sum::<f32>();
        // The diffstat goes first; icons only when the tabs alone won't fit
        // (GitHub keeps them at normal widths).
        let compact = natural(false) + toggle_w > avail;
        // Still too tight: "Files changed" becomes "Files", like GitHub's narrow view.
        let short = compact && natural(true) + toggle_w + 8.0 > avail;
        let show_stat = stat.is_some() && !compact && natural(false) + toggle_w + stat_w <= avail;
        // A gap before the toggle so the scroll fade doesn't butt into it.
        let tabs_w = avail - toggle_w - if show_stat { stat_w } else { 0.0 } - if app.details_folded { 8.0 } else { 0.0 };
        let out = egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .max_width(tabs_w)
            .auto_shrink([true, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (tab, icon, text, count) in tabs {
                        let text = if short && tab == Tab::Files { "Files" } else { text };
                        let resp = tab_button(ui, p, icon, text, count, app.tab == tab, compact);
                        // Keep the open tab in view when the row scrolls.
                        // Scroll a bit past it so the fade lands on a neighbor.
                        if app.tab == tab && !ui.clip_rect().contains_rect(resp.rect) {
                            ui.scroll_to_rect(resp.rect.expand2(vec2(40.0, 0.0)), None);
                        }
                        if resp.clicked() {
                            app.actions.push(Action::Tab(tab));
                        }
                    }
                });
            });
        // Tabs cut off at either end fade out, like a scrolling row.
        let r = out.inner_rect;
        let hidden_right = out.content_size.x - out.state.offset.x - r.width();
        for (hidden, right) in [(out.state.offset.x, false), (hidden_right, true)] {
            if hidden <= 1.0 {
                continue;
            }
            let steps = 8;
            for i in 0..steps {
                let t = (i + 1) as f32 / steps as f32;
                let w = 32.0 / steps as f32;
                let x0 = if right { r.right() - 32.0 + w * i as f32 } else { r.left() + 32.0 - w * (i + 1) as f32 };
                let band = Rect::from_min_max(pos2(x0, r.top()), pos2(x0 + w + 0.5, r.bottom() - 2.0));
                ui.painter().rect_filled(band, 0.0, p.canvas.gamma_multiply(t));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // A hidden sidebar's toggle sits at the far right of this row.
            if app.details_folded && icon_button(ui, Icon::SidebarRightOpen, "Show details (⇧⌘B)", p).clicked() {
                if app.details_wide {
                    app.actions.push(Action::TogglePanel(Panel::Details));
                } else {
                    app.details_open_narrow = true;
                }
            }
            if let (Some(d), Some((adds, dels)), true) = (d, stat, show_stat) {
                if app.details_folded {
                    ui.add_space(12.0);
                }
                let (rect, _) = ui.allocate_exact_size(vec2(50.0, 16.0), Sense::hover());
                diff::diffstat_blocks(ui.painter(), p, pos2(rect.left(), rect.center().y), d.additions, d.deletions);
                ui.add_space(6.0);
                ui.label(RichText::new(dels).font(theme::bold(13.0)).color(p.closed));
                ui.add_space(6.0);
                ui.label(RichText::new(adds).font(theme::bold(13.0)).color(p.open));
            }
        });
    });
    let r = row.response.rect;
    ui.painter().line_segment(
        [pos2(ui.max_rect().left(), r.bottom()), pos2(ui.max_rect().right(), r.bottom())],
        Stroke::new(1.0, p.border),
    );
}

/// A colored bar under the title: ready, blocked (and why), conflicts, ...
/// with merge and auto-merge controls on the right.
fn merge_bar(app: &mut App, ui: &mut Ui, p: &Palette, ms: &super::MergeStatus, d: Option<&PrDetail>) {
    egui::Frame::new()
        .fill(ms.color.gamma_multiply(if p.dark { 0.14 } else { 0.08 }))
        .stroke(Stroke::new(1.0, ms.color.gamma_multiply(0.6)))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Same height whatever the state, so the tabs don't jump between PRs.
            ui.set_min_height(32.0);
            // One estimate of the buttons' width for both decisions below:
            // stack the buttons under the status, and show the reason whole.
            let controls_w = match d {
                Some(d) if d.is_draft => 200.0,
                Some(d) if d.checks.is_empty() => 200.0,
                Some(_) => 250.0,
                None => 0.0,
            };
            let measure = |t: &str, f: egui::FontId| ui.painter().layout_no_wrap(t.to_string(), f, p.fg).size().x;
            let title_w = measure(&ms.title, theme::bold(14.0));
            let avail = ui.available_width();
            // The reason matters more than one line: if it won't fit beside the
            // buttons, the buttons move down (GitHub always says why).
            let reason_w = 36.0 + title_w + 8.0 + measure(&ms.detail, theme::body(13.0));
            let stacked = avail < 36.0 + title_w + 80.0 + controls_w || (!ms.detail.is_empty() && reason_w + 16.0 + controls_w > avail);
            // The buttons fit beside the title alone: the reason goes under
            // the title instead of the buttons under everything.
            let reason_below = stacked && !ms.detail.is_empty() && 36.0 + title_w + 16.0 + controls_w <= avail;
            let reason_fits = !reason_below && reason_w + if stacked { 0.0 } else { 16.0 + controls_w } <= avail;
            let status = |ui: &mut Ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 14.0, ms.color);
                icons::paint(ui.painter(), rect.shrink(7.0), ms.icon, Color32::WHITE);
                ui.add_space(4.0);
                let title = ui.label(RichText::new(&ms.title).font(theme::bold(14.0)).color(p.fg));
                // The reason shows whole or not at all (then as the title's tooltip).
                if reason_fits {
                    ui.label(RichText::new(&ms.detail).size(13.0).color(p.fg_muted));
                } else if !ms.detail.is_empty() {
                    title.tip(&ms.detail);
                }
            };
            let mut controls = |ui: &mut Ui| {
                if let Some(d) = d {
                    merge_controls(app, ui, p, ms, d);
                    if !d.checks.is_empty() && app.tab != Tab::Checks && link(ui, "View checks", 13.0, p).clicked() {
                        app.actions.push(Action::Tab(Tab::Checks));
                    }
                }
            };
            if reason_below {
                ui.spacing_mut().item_spacing.y = 2.0;
                egui::Sides::new().shrink_left().show(ui, status, controls);
                ui.horizontal(|ui| {
                    // 28px icon + the two gaps before the title.
                    ui.add_space(28.0 + 4.0 + ui.spacing().item_spacing.x);
                    ui.label(RichText::new(&ms.detail).size(13.0).color(p.fg_muted));
                });
            } else if stacked {
                ui.horizontal(status);
                ui.add_space(6.0);
                // Only as tall as the buttons (a bare layout here would fill
                // the rest of the window).
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| controls(ui));
            } else {
                egui::Sides::new().shrink_left().show(ui, status, controls);
            }
        });
}

/// Right side of the merge bar. Laid out right to left.
fn merge_controls(app: &mut App, ui: &mut Ui, p: &Palette, _ms: &super::MergeStatus, d: &PrDetail) {
    // Laid out right to left here, so the main button ends up on the right.
    merge_actions(app, ui, p, d);
}

/// The merge actions, the same at the top and in the merge box: GitHub's
/// split "Squash and merge ▾" (or "Ready for review" on a draft), the
/// confirm step, and auto-merge.
fn merge_actions(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    if app.posting {
        ui.add(egui::Spinner::new().size(16.0));
        return;
    }
    if d.is_draft {
        if ui.add(button("Ready for review", p)).tip("Take this pull request out of draft").clicked() {
            app.actions.push(Action::ReadyForReview);
        }
        return;
    }
    if d.auto_merge_request.is_some() {
        if d.viewer_can_disable_auto_merge && ui.add(button("Disable auto-merge", p)).clicked() {
            app.actions.push(Action::DisableAutoMerge);
        }
        return;
    }
    let ms = detail_merge_status(p, d, None);
    let ready = ms.as_ref().is_some_and(|m| m.short == "Ready to merge");
    let method = app.chosen_merge_method(d);
    let label = crate::app::merge_method_label(&method);
    if app.confirm_merge.as_deref() == Some(d.id.as_str()) {
        let confirm = |ui: &mut Ui, app: &mut App| {
            if ui.add(primary_button(&format!("Confirm {}", label.to_lowercase()), p)).clicked() {
                app.actions.push(Action::Merge);
            }
        };
        let cancel = |ui: &mut Ui, app: &mut App| {
            if ui.add(button("Cancel", p)).clicked() {
                app.actions.push(Action::CancelMerge);
            }
        };
        // Same order either way: in a right-to-left row Confirm lands on the right.
        confirm(ui, app);
        cancel(ui, app);
        return;
    }
    // Blocked but auto-merge is allowed: GitHub's one green "Enable
    // auto-merge" split button (its menu still picks the method).
    let auto = !ready && d.viewer_can_enable_auto_merge && d.repository.auto_merge_allowed;
    let why = ms.as_ref().map(|m| format!("Can't merge yet: {}", m.detail)).unwrap_or_default();
    if auto {
        let tip = format!("{label} automatically once requirements are met");
        split_button(app, ui, p, d, "Enable auto-merge", &method, true, &tip, Action::EnableAutoMerge);
    } else {
        split_button(app, ui, p, d, label, &method, ready, &why, Action::AskMerge);
    }
}

/// GitHub's split button: "Squash and merge" on the left (asks to confirm),
/// "▾" on the right for the method menu. One widget, so layout order can't
/// pull its halves apart.
#[allow(clippy::too_many_arguments)]
fn split_button(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail, label: &str, method: &str, ready: bool, why: &str, on_click: Action) {
    let font = theme::bold(14.0);
    let text_color = if ready { Color32::WHITE } else { p.fg };
    let g = ui.painter().layout_no_wrap(label.to_string(), font, text_color);
    let main_w = g.size().x + 24.0;
    let (rect, _) = ui.allocate_exact_size(vec2(main_w + 30.0, 32.0), Sense::hover());
    let main_rect = Rect::from_min_size(rect.min, vec2(main_w, rect.height()));
    let caret_rect = Rect::from_min_max(pos2(main_rect.right(), rect.top()), rect.max);
    // Scoped to the parent: the same button appears at the top and in the merge box.
    let main = ui.interact(main_rect, ui.id().with(("merge-main", &d.id)), if ready { Sense::click() } else { Sense::hover() });
    let caret = ui.interact(caret_rect, ui.id().with(("merge-caret", &d.id)), Sense::click());
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, label));
    caret.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Choose how to merge"));
    let (fill, hover_fill, border) = if ready {
        (p.btn_primary, p.btn_primary.gamma_multiply(0.9), p.btn_primary.gamma_multiply(0.85))
    } else {
        (p.btn_bg, ui.visuals().widgets.hovered.weak_bg_fill, p.border)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, fill);
    if main.hovered() && ready {
        painter.rect_filled(main_rect, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, hover_fill);
    }
    if caret.hovered() {
        painter.rect_filled(caret_rect, CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 }, hover_fill);
    }
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let divider = if ready { Color32::from_black_alpha(60) } else { p.border };
    painter.vline(main_rect.right(), rect.y_range(), Stroke::new(1.0, divider));
    // Can't merge yet: GitHub greys the label out.
    let label_color = if ready { Color32::WHITE } else { p.fg_muted.gamma_multiply(0.75) };
    painter.text(main_rect.center(), egui::Align2::CENTER_CENTER, label, theme::bold(14.0), label_color);
    let c = caret_rect.center();
    let caret_color = if ready { Color32::WHITE } else { p.fg_muted.gamma_multiply(0.75) };
    painter.add(egui::Shape::convex_polygon(vec![pos2(c.x - 4.0, c.y - 2.0), pos2(c.x + 4.0, c.y - 2.0), pos2(c.x, c.y + 2.5)], caret_color, Stroke::NONE));
    if ready {
        let main = main.on_hover_cursor(egui::CursorIcon::PointingHand);
        if !matches!(on_click, Action::AskMerge) {
            super::tip(&main, why);
        }
        if main.clicked() {
            app.actions.push(on_click);
        }
    } else {
        super::tip(&main, why);
    }
    let caret = caret.on_hover_cursor(egui::CursorIcon::PointingHand);
    super::tip(&caret, "Choose how to merge");
    method_popup(app, p, d, method, &caret, Some(rect));
}

/// The menu of merge methods, opened by clicking `resp`.
fn method_popup(app: &mut App, p: &Palette, d: &PrDetail, method: &str, resp: &egui::Response, under: Option<Rect>) {
    let methods = crate::app::merge_methods(d);
    let n = d.commits.total_count;
    let commits = if n == 1 { "The 1 commit".to_string() } else { format!("The {n} commits") };
    let mut popup = egui::Popup::menu(resp).align(if under.is_some() { egui::RectAlign::BOTTOM_START } else { egui::RectAlign::BOTTOM_END });
    if let Some(r) = under {
        popup = popup.anchor(r);
    }
    popup.show(|ui| {
        ui.set_width(340.0);
        for m in &methods {
            let about = match *m {
                "SQUASH" => format!("{commits} from this branch will be combined into one commit in the base branch."),
                "REBASE" => format!("{commits} from this branch will be rebased and added to the base branch."),
                _ => "All commits from this branch will be added to the base branch via a merge commit.".to_string(),
            };
            let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.hover_row);
            }
            if *m == method {
                icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 10.0), vec2(16.0, 16.0)), Icon::Check, p.fg);
            }
            let x = rect.left() + 32.0;
            ui.painter().text(pos2(x, rect.top() + 18.0), egui::Align2::LEFT_CENTER, crate::app::merge_method_label(m), theme::bold(14.0), p.fg);
            let g = ui.painter().layout(about, theme::body(12.0), p.fg_muted, rect.right() - x - 8.0);
            ui.painter().galley(pos2(x, rect.top() + 28.0), g, p.fg_muted);
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.actions.push(Action::SetMergeMethod(m.to_string()));
                ui.close();
            }
        }
    });
}

/// Badges use Primer's stronger "emphasis" fills.
fn badge_color(p: &Palette, state: &str, draft: bool) -> Color32 {
    match (state, draft) {
        ("MERGED", _) => p.merged_emphasis,
        ("CLOSED", _) => p.closed_emphasis,
        (_, true) => p.neutral_emphasis,
        _ => p.open_emphasis,
    }
}

/// "Open", "Draft", ... in a filled pill, 32 tall (or smaller for the
/// slim header).
fn state_badge(ui: &mut Ui, icon: Icon, color: Color32, word: &str, height: f32) {
    let k = height / 32.0;
    let g = ui.painter().layout_no_wrap(word.to_string(), theme::bold(14.0 * k.max(0.85)), Color32::WHITE);
    let icon_s = 16.0 * k.max(0.85);
    let pad = 12.0 * k;
    let size = vec2(pad + icon_s + 6.0 * k + g.size().x + pad, height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), color);
    icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + pad, rect.center().y - icon_s / 2.0), vec2(icon_s, icon_s)), icon, Color32::WHITE);
    ui.painter().galley(pos2(rect.left() + pad + icon_s + 6.0 * k, rect.center().y - g.size().y / 2.0), g, Color32::WHITE);
}

fn branch(ui: &mut Ui, p: &Palette, name: &str) {
    let fg = p.accent;
    pill(ui, name, theme::mono(12.0), p.accent_subtle, fg, Color32::TRANSPARENT);
}

/// A tab's width. `compact` drops the icon and tightens the padding.
fn tab_width(ui: &Ui, p: &Palette, text: &str, count: Option<u64>, active: bool, compact: bool) -> f32 {
    let font = if active { theme::bold(14.0) } else { theme::body(14.0) };
    let text_w = ui.painter().layout_no_wrap(text.to_string(), font, p.fg).size().x;
    let count_w = count.map(|n| ui.painter().layout_no_wrap(n.to_string(), theme::bold(12.0), p.fg).size().x + 14.0 + 8.0).unwrap_or(0.0);
    let pad = if compact { 10.0 } else { 16.0 };
    let icon = if compact { 0.0 } else { 24.0 };
    pad + icon + text_w + count_w + pad
}

fn tab_button(ui: &mut Ui, p: &Palette, icon: Icon, text: &str, count: Option<u64>, active: bool, compact: bool) -> egui::Response {
    let font = if active { theme::bold(14.0) } else { theme::body(14.0) };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, p.fg);
    let w = tab_width(ui, p, text, count, active, compact);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, text));
    let painter = ui.painter();
    if resp.hovered() && !active {
        painter.rect_filled(rect.shrink2(vec2(4.0, 6.0)), 6.0, p.hover_row);
    }
    let mut x = rect.left() + if compact { 10.0 } else { 16.0 };
    if !compact {
        icons::paint(painter, Rect::from_min_size(pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0)), icon, p.fg_muted);
        x += 24.0;
    }
    let gh = g.size().y;
    let gw = g.size().x;
    painter.galley(pos2(x, rect.center().y - gh / 2.0), g, p.fg);
    x += gw + 8.0;
    if let Some(n) = count {
        let bg = if p.dark { Color32::from_rgb(0x2f, 0x35, 0x3d) } else { Color32::from_rgb(0xe6, 0xea, 0xef) };
        let cg = painter.layout_no_wrap(n.to_string(), theme::bold(12.0), p.fg);
        let cr = Rect::from_min_size(pos2(x, rect.center().y - 10.0), vec2(cg.size().x + 14.0, 20.0));
        painter.rect_filled(cr, CornerRadius::same(255), bg);
        painter.galley(pos2(cr.left() + 7.0, cr.center().y - cg.size().y / 2.0), cg, p.fg);
    }
    if active {
        // GitHub's orange underline for the selected tab.
        let line = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 2.0), pos2(rect.right() - 8.0, rect.bottom()));
        painter.rect_filled(line, 2.0, Color32::from_rgb(0xfd, 0x8c, 0x73));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------- Conversation ----------

enum Item<'a> {
    Comment(&'a Comment),
    Review(&'a Review),
    /// A thread whose review wasn't loaded (PRs with 100+ reviews).
    Thread(&'a Thread),
    /// Merged, closed, labeled, ... Consecutive label changes by one
    /// person are shown together, like on GitHub.
    Events(Vec<&'a Event>),
    /// Commits pushed in a row.
    Commits(Vec<&'a EventCommit>),
}

impl Item<'_> {
    fn time(&self) -> &str {
        match self {
            Item::Comment(c) => &c.created_at,
            Item::Review(r) => r.submitted_at.as_deref().unwrap_or(""),
            Item::Thread(t) => t.comments.nodes.first().map(|c| c.created_at.as_str()).unwrap_or(""),
            Item::Events(e) => e.first().map(|e| e.created_at.as_str()).unwrap_or(""),
            Item::Commits(c) => c.first().map(|c| c.committed_date.as_str()).unwrap_or(""),
        }
    }
}

/// The review a thread was started in.
fn thread_review(t: &Thread) -> Option<&str> {
    t.comments.nodes.first()?.pull_request_review.as_ref().map(|r| r.id.as_str())
}

/// Reviewers, assignees and labels, on the right. Slides closed to a strip.
fn details_panel(app: &mut App, ui: &mut Ui, p: &Palette, d: &Arc<PrDetail>) {
    // Like GitHub's layout breakpoint: on a narrow page the sidebar folds
    // away so the conversation keeps room. Opening it then is temporary.
    let wide = ui.available_width() >= 820.0;
    let mut open = if wide { app.panels.details } else { app.details_open_narrow };
    app.details_wide = wide;
    app.details_folded = !open;
    if app.details_folded {
        return;
    }
    let mut toggle = false;
    egui::Panel::show_switched(
        ui,
        &mut open,
        egui::Panel::right("details-rail")
            .resizable(false)
            .exact_size(44.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin { left: 12, right: 0, top: 8, bottom: 0 })),
        egui::Panel::right("details")
            .resizable(true)
            .default_size(256.0)
            .size_range(200.0..=420.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin { left: 24, right: 0, top: 8, bottom: 0 })),
        |ui, expanded| {
            if expanded {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Details").font(theme::bold(16.0)).color(p.fg));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        toggle |= icon_button(ui, Icon::SidebarRightClose, "Hide details (⇧⌘B)", p).clicked();
                    });
                });
                ui.add_space(8.0);
                row_separator(ui);
                ui.add_space(12.0);
                egui::ScrollArea::vertical().id_salt("details-scroll").auto_shrink([false, false]).show(ui, |ui| sidebar(ui, p, d));
            } else {
                toggle |= icon_button(ui, Icon::SidebarRightOpen, "Show details (⇧⌘B)", p).clicked();
            }
        },
    );
    if wide {
        app.panels.details = open;
    } else {
        app.details_open_narrow = open;
    }
    if toggle {
        if wide {
            app.actions.push(Action::TogglePanel(Panel::Details));
        } else {
            app.details_open_narrow = !app.details_open_narrow;
        }
    }
}

fn timeline(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    let pr_author = d.author.as_ref().map(|a| a.login.as_str()).unwrap_or("");
    // The PR description is the first comment.
    comment_box(
        app,
        ui,
        p,
        d.author.as_ref().map(|a| (a.login.as_str(), a.avatar_url.as_str())),
        "opened this pull request",
        &d.created_at,
        &d.body,
        pr_author,
        &d.url,
        &d.repository.name_with_owner,
    );

    // One continuous line behind every item, drawn first so the boxes and
    // badges sit on top of it.
    let line_slot = ui.painter().add(egui::Shape::Noop);
    let line_top = ui.cursor().top();
    let repo = d.repository.name_with_owner.as_str();
    let review_ids: std::collections::HashSet<&str> = d.reviews.nodes.iter().map(|r| r.id.as_str()).collect();
    let mut items: Vec<Item> = d.comments.nodes.iter().map(Item::Comment).collect();
    items.extend(d.reviews.nodes.iter().map(Item::Review));
    items.extend(
        d.review_threads.nodes.iter().filter(|t| !thread_review(t).is_some_and(|r| review_ids.contains(r))).map(Item::Thread),
    );
    let merged_at: Vec<&str> = d.timeline_items.nodes.iter().filter(|e| e.kind == "MergedEvent").map(|e| e.created_at.as_str()).collect();
    for e in &d.timeline_items.nodes {
        // GitHub hides the "closed" that comes with every merge.
        if e.kind == "ClosedEvent" && merged_at.contains(&e.created_at.as_str()) {
            continue;
        }
        match (&e.kind[..], &e.commit) {
            ("PullRequestCommit", Some(c)) => items.push(Item::Commits(vec![c])),
            ("PullRequestCommit", None) => {}
            _ => items.push(Item::Events(vec![e])),
        }
    }
    items.sort_by(|a, b| a.time().cmp(b.time()));
    // Group runs: commits pushed together, and label changes by one person.
    let mut grouped: Vec<Item> = Vec::new();
    for item in items {
        match (grouped.last_mut(), item) {
            (Some(Item::Commits(run)), Item::Commits(more)) => run.extend(more),
            (Some(Item::Events(run)), Item::Events(more))
                if (is_label(run[0]) && is_label(more[0]) || is_request(run[0]) && is_request(more[0])) && same_actor(run[0], more[0]) =>
            {
                run.extend(more)
            }
            (_, item) => grouped.push(item),
        }
    }
    let items = grouped;

    for item in &items {
        fenced(ui, |ui| match item {
            Item::Comment(c) => {
                timeline_gap(ui, p);
                comment_box(
                    app,
                    ui,
                    p,
                    c.author.as_ref().map(|a| (a.login.as_str(), a.avatar_url.as_str())),
                    "commented",
                    &c.created_at,
                    &c.body,
                    pr_author,
                    &c.url,
                    repo,
                );
            }
            Item::Review(r) => {
                let threads: Vec<&Thread> = d.review_threads.nodes.iter().filter(|t| thread_review(t) == Some(r.id.as_str())).collect();
                // Reviews with no text and no new threads are replies inside
                // an existing thread; GitHub folds those away too.
                if r.state == "COMMENTED" && r.body.trim().is_empty() && threads.is_empty() {
                    return;
                }
                // One block, like GitHub: the review's line, then its text
                // and threads right under it, sharing that line's avatar.
                timeline_gap(ui, p);
                review_event(ui, p, r);
                if !r.body.trim().is_empty() {
                    ui.add_space(8.0);
                    comment_card(
                        app,
                        ui,
                        p,
                        r.author.as_ref().map(|a| (a.login.as_str(), a.avatar_url.as_str())),
                        "left a comment",
                        r.submitted_at.as_deref().unwrap_or(""),
                        &r.body,
                        pr_author,
                        &r.url,
                        repo,
                        false,
                    );
                }
                for t in threads {
                    ui.add_space(8.0);
                    thread_box(app, ui, p, t, repo);
                }
            }
            Item::Thread(t) => {
                timeline_gap(ui, p);
                thread_box(app, ui, p, t, repo);
            }
            Item::Events(events) => {
                timeline_gap(ui, p);
                event_row(app, ui, p, events, repo, &d.head_ref_name);
            }
            Item::Commits(commits) => {
                timeline_gap(ui, p);
                commits_row(app, ui, p, commits, repo);
            }
        });
    }

    timeline_gap(ui, p);
    let x = ui.min_rect().left() + GUTTER + 20.0;
    ui.painter().set(line_slot, egui::Shape::line_segment([pos2(x, line_top), pos2(x, ui.cursor().top())], Stroke::new(2.0, p.border_muted)));
    merge_box(app, ui, p, d);
    // GitHub keeps the comment box on merged and closed PRs too.
    ui.add_space(16.0);
    composer(app, ui, p, d);
}

/// Draws `f` in a box exactly as wide as the space left. Whatever is inside
/// can't widen the page (egui would otherwise grow the whole column to fit
/// the widest thing in it); anything too wide is clipped instead.
fn fenced(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    let top_left = ui.cursor().min;
    let w = ui.available_width();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(top_left, vec2(w, f32::INFINITY))).layout(egui::Layout::top_down(egui::Align::Min)));
    let clip = ui.clip_rect();
    child.set_clip_rect(Rect::from_x_y_ranges(top_left.x..=top_left.x + w, clip.y_range()).intersect(clip));
    f(&mut child);
    let h = child.min_rect().height();
    ui.allocate_rect(Rect::from_min_size(top_left, vec2(w, h)), Sense::hover());
}

/// GitHub Apps (bots) have avatars under /in/.
fn is_bot(login: &str, avatar: &str) -> bool {
    login.ends_with("[bot]") || avatar.contains("githubusercontent.com/in/")
}

fn is_label(e: &Event) -> bool {
    matches!(e.kind.as_str(), "LabeledEvent" | "UnlabeledEvent")
}

fn is_request(e: &Event) -> bool {
    e.kind == "ReviewRequestedEvent"
}

fn same_actor(a: &Event, b: &Event) -> bool {
    a.actor.as_ref().map(|x| &x.login) == b.actor.as_ref().map(|x| &x.login)
}

/// The round badge on the timeline line, with the person and a sentence.
fn event_line(ui: &mut Ui, p: &Palette, icon: Icon, badge: Option<Color32>, who: Option<&crate::github::Actor>, body: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(GUTTER + 4.0);
        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
        match badge {
            Some(fill) => {
                ui.painter().circle(rect.center(), 16.0, fill, Stroke::new(2.0, p.canvas));
                icons::paint(ui.painter(), rect.shrink(8.0), icon, Color32::WHITE);
            }
            None => {
                ui.painter().circle(rect.center(), 16.0, p.canvas_subtle, Stroke::new(2.0, p.canvas));
                icons::paint(ui.painter(), rect.shrink(8.0), icon, p.fg_muted);
            }
        }
        ui.add_space(4.0);
        // Wraps, since one event can carry many labels.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.spacing_mut().interact_size.y = 20.0;
            if let Some(a) = who {
                avatar(ui, &a.avatar_url, 20.0);
                ui.label(RichText::new(&a.login).font(theme::bold(14.0)).color(p.fg));
            }
            body(ui);
        });
    });
}

fn sha_link(app: &mut App, ui: &mut Ui, p: &Palette, repo: &str, c: &EventCommit) {
    if link_styled(ui, &c.abbreviated_oid, theme::mono(13.0), p.fg).tip(&c.oid).clicked() {
        app.actions.push(Action::OpenUrl(format!("https://github.com/{repo}/commit/{}", c.oid)));
    }
}

/// Merged, closed, reopened, branch deleted, force-pushed, labeled, ...
fn event_row(app: &mut App, ui: &mut Ui, p: &Palette, events: &[&Event], repo: &str, head_ref: &str) {
    let e = events[0];
    let muted = |ui: &mut Ui, t: &str| {
        ui.label(RichText::new(t).color(p.fg_muted));
    };
    // "on May 19" stays together when the line wraps.
    let ago = util::ago(&e.created_at).replace(' ', "\u{a0}");
    let who = e.actor.as_ref();
    match e.kind.as_str() {
        "MergedEvent" => event_line(ui, p, Icon::PrMerged, Some(p.merged_emphasis), who, |ui| {
            muted(ui, "merged commit");
            if let Some(c) = &e.commit {
                sha_link(app, ui, p, repo, c);
            }
            muted(ui, "into");
            branch(ui, p, e.merge_ref_name.as_deref().unwrap_or(""));
            muted(ui, &ago);
        }),
        "ClosedEvent" => event_line(ui, p, Icon::PrClosed, Some(p.closed_emphasis), who, |ui| muted(ui, &format!("closed this {ago}"))),
        "ReopenedEvent" => event_line(ui, p, Icon::PrOpen, Some(p.open_emphasis), who, |ui| muted(ui, &format!("reopened this {ago}"))),
        "HeadRefDeletedEvent" => event_line(ui, p, Icon::Branch, None, who, |ui| {
            muted(ui, "deleted the");
            branch(ui, p, e.head_ref_name.as_deref().unwrap_or("head"));
            muted(ui, &format!("branch {ago}"));
        }),
        "HeadRefForcePushedEvent" => event_line(ui, p, Icon::Commit, None, who, |ui| {
            muted(ui, "force-pushed the");
            branch(ui, p, head_ref);
            muted(ui, "branch from");
            if let Some(c) = &e.before_commit {
                sha_link(app, ui, p, repo, c);
            }
            muted(ui, "to");
            if let Some(c) = &e.after_commit {
                sha_link(app, ui, p, repo, c);
            }
            muted(ui, &ago);
        }),
        "ReadyForReviewEvent" => event_line(ui, p, Icon::Eye, None, who, |ui| muted(ui, &format!("marked this pull request as ready for review {ago}"))),
        "ConvertToDraftEvent" => event_line(ui, p, Icon::PrDraft, None, who, |ui| muted(ui, &format!("marked this pull request as draft {ago}"))),
        // Requests made together read as one line: "requested review from
        // a, b and c".
        "ReviewRequestedEvent" => event_line(ui, p, Icon::Eye, None, who, |ui| {
            let mut names: Vec<String> = Vec::new();
            for e in events {
                let name = match &e.requested_reviewer {
                    Some(Reviewer::User { login, .. }) => login.clone(),
                    Some(Reviewer::Team { name }) => name.clone(),
                    _ => "someone".into(),
                };
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            muted(ui, if names.len() == 1 { "requested a review from" } else { "requested review from" });
            let n = names.len();
            for (i, name) in names.iter().enumerate() {
                let sep = if i + 2 < n { "," } else { "" };
                ui.label(RichText::new(format!("{name}{sep}")).font(theme::bold(14.0)).color(p.fg));
                if i + 2 == n {
                    muted(ui, "and");
                }
            }
            muted(ui, &ago);
        }),
        "AssignedEvent" => event_line(ui, p, Icon::Person, None, who, |ui| {
            let name = match &e.assignee {
                Some(Reviewer::User { login, .. }) => login.clone(),
                _ => "someone".into(),
            };
            if who.is_some_and(|a| a.login == name) {
                muted(ui, &format!("self-assigned this {ago}"));
            } else {
                muted(ui, "assigned");
                ui.label(RichText::new(name).font(theme::bold(14.0)).color(p.fg));
                muted(ui, &ago);
            }
        }),
        "AutoMergeEnabledEvent" => event_line(ui, p, Icon::AutoMerge, None, who, |ui| muted(ui, &format!("enabled auto-merge {ago}"))),
        "AutoMergeDisabledEvent" => event_line(ui, p, Icon::AutoMerge, None, who, |ui| muted(ui, &format!("disabled auto-merge {ago}"))),
        "LabeledEvent" | "UnlabeledEvent" => event_line(ui, p, Icon::Tag, None, who, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let added: Vec<_> = events.iter().filter(|e| e.kind == "LabeledEvent").filter_map(|e| e.label.as_ref()).collect();
            let removed: Vec<_> = events.iter().filter(|e| e.kind == "UnlabeledEvent").filter_map(|e| e.label.as_ref()).collect();
            // "added a b and removed c labels", like GitHub.
            let one = added.len() + removed.len() == 1;
            for (i, (word, labels)) in [("added", &added), ("removed", &removed)].into_iter().filter(|(_, l)| !l.is_empty()).enumerate() {
                muted(ui, if i == 0 { word } else if word == "removed" { "and removed" } else { word });
                if one {
                    muted(ui, "the");
                }
                for l in labels.iter() {
                    label_pill(ui, &l.name, &l.color);
                }
            }
            muted(ui, if added.len() + removed.len() == 1 { "label" } else { "labels" });
            muted(ui, &ago);
        }),
        _ => {}
    }
}

/// "octocat added 3 commits 2 days ago" and the commits under it.
fn commits_row(app: &mut App, ui: &mut Ui, p: &Palette, commits: &[&EventCommit], repo: &str) {
    let first = commits[0];
    let who = first.author.as_ref().and_then(|a| a.user.clone());
    let n = commits.len();
    event_line(ui, p, Icon::Commit, None, who.as_ref(), |ui| {
        if who.is_none() {
            avatar(ui, "", 20.0);
            let name = first.author.as_ref().and_then(|a| a.name.clone()).unwrap_or_else(|| "Someone".into());
            ui.label(RichText::new(name).font(theme::bold(14.0)).color(p.fg));
        }
        ui.label(RichText::new(format!("added {n} commit{} {}", if n == 1 { "" } else { "s" }, util::ago(&first.committed_date))).color(p.fg_muted));
    });
    for c in commits {
        ui.horizontal(|ui| {
            ui.add_space(GUTTER + 4.0 + 32.0 + 8.0);
            let a = c.author.as_ref().and_then(|a| a.user.as_ref());
            avatar(ui, a.map(|u| u.avatar_url.as_str()).unwrap_or(""), 16.0);
            let msg_w = (ui.available_width() - 90.0).max(80.0);
            let mut job = egui::text::LayoutJob::default();
            super::append_title(&mut job, &c.message_headline, 13.0, false, 0.0, p);
            job.wrap = egui::text::TextWrapping::truncate_at_width(msg_w);
            let g = ui.painter().layout_job(job);
            let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
            ui.painter().galley(r.min, g, p.fg);
            if resp.hovered() {
                ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, p.accent));
            }
            if resp.tip(&c.message_headline).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.actions.push(Action::OpenUrl(format!("https://github.com/{repo}/commit/{}", c.oid)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                sha_link(app, ui, p, repo, c);
                if let Some(r) = &c.status_check_rollup {
                    let (icon, color) = status_icon(&r.state, p);
                    icons::show(ui, icon, 14.0, color).tip(match r.state.as_str() {
                        "SUCCESS" => "All checks passed",
                        "FAILURE" | "ERROR" => "Some checks failed",
                        "PENDING" | "EXPECTED" => "Checks are running",
                        _ => "Checks",
                    });
                }
            });
        });
    }
}

/// The vertical gray line that connects timeline items.
fn timeline_gap(ui: &mut Ui, _p: &Palette) {
    ui.add_space(16.0);
}

#[allow(clippy::too_many_arguments)]
fn comment_box(
    app: &mut App,
    ui: &mut Ui,
    p: &Palette,
    author: Option<(&str, &str)>,
    verb: &str,
    time: &str,
    body: &str,
    pr_author: &str,
    url: &str,
    repo: &str,
) {
    comment_card(app, ui, p, author, verb, time, body, pr_author, url, repo, true);
}

/// A comment box. Without `show_avatar` the gutter stays empty: a review's
/// body sits under the review's own line, which already has the avatar.
#[allow(clippy::too_many_arguments)]
fn comment_card(
    app: &mut App,
    ui: &mut Ui,
    p: &Palette,
    author: Option<(&str, &str)>,
    verb: &str,
    time: &str,
    body: &str,
    pr_author: &str,
    url: &str,
    repo: &str,
    show_avatar: bool,
) {
    let (login, avatar_url) = author.unwrap_or(("ghost", ""));
    let mine = !app.viewer.is_empty() && login == app.viewer;
    let (head_fill, border) = if mine { (p.accent_subtle, p.accent.gamma_multiply(0.5)) } else { (p.canvas_subtle, p.border) };
    let mut open_link = false;
    let mut menu_acts = Vec::new();
    ui.horizontal_top(|ui| {
        if show_avatar {
            avatar(ui, avatar_url, AVATAR);
            ui.add_space(16.0 - ui.spacing().item_spacing.x);
        } else {
            // Same left edge as an avatar plus its gap.
            ui.add_space(GUTTER);
        }
        ui.vertical(|ui| {
            boxed(
                ui,
                head_fill,
                border,
                |ui| {
                    // Badges and menu first (right), then the text in what's
                    // left, cut with "…" so it can never run under them.
                    ui.horizontal(|ui| {
                        let author_w = if login == pr_author { 62.0 } else { 0.0 };
                        let bot = is_bot(login, avatar_url);
                        let text_w = (ui.available_width() - 40.0 - author_w - if bot { 40.0 } else { 0.0 }).max(60.0);
                        let mut job = egui::text::LayoutJob::default();
                        job.append(login, 0.0, egui::TextFormat { font_id: theme::bold(14.0), color: p.fg, ..Default::default() });
                        // "github-actions [bot] commented …": the badge goes
                        // right after the name, so the text is drawn in two parts.
                        if bot {
                            let g = ui.painter().layout_job(std::mem::take(&mut job));
                            let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                            ui.painter().galley(r.min, g, p.fg);
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                open_link = true;
                            }
                            pill(ui, "bot", theme::body(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
                        }
                        let text_w = if bot { (ui.available_width() - 40.0 - author_w).max(60.0) } else { text_w };
                        job.append(verb, if bot { 0.0 } else { 4.0 }, egui::TextFormat { font_id: theme::body(14.0), color: p.fg_muted, ..Default::default() });
                        job.append(&util::ago(time), 4.0, egui::TextFormat { font_id: theme::body(14.0), color: p.fg_muted, ..Default::default() });
                        job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
                        let g = ui.painter().layout_job(job);
                        let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                        ui.painter().galley(r.min, g, p.fg);
                        if resp.hovered() {
                            ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, p.fg_muted.gamma_multiply(0.5)));
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).tip(format!("{login} {verb} {} · open on GitHub", util::ago(time))).clicked() {
                            open_link = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            comment_menu(ui, p, url, body, &mut menu_acts);
                            if login == pr_author {
                                pill(ui, "Author", theme::bold(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
                            }
                        });
                    });
                },
                |ui| markdown(app, ui, p, body, url, repo),
            );
        });
    });
    app.actions.extend(menu_acts);
    if open_link {
        app.actions.push(Action::OpenUrl(url.to_string()));
    }
}

/// "⋯" menu on a comment, like github.com's.
fn comment_menu(ui: &mut Ui, p: &Palette, url: &str, body: &str, acts: &mut Vec<Action>) {
    let resp = super::icon_button_plain(ui, Icon::Kebab, "More actions", p);
    egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_END).show(|ui| {
        ui.set_min_width(170.0);
        if ui.button("Copy link").clicked() {
            acts.push(Action::Copy(url.to_string()));
        }
        if ui.button("Quote reply").clicked() {
            acts.push(Action::QuoteReply(body.to_string()));
        }
        ui.separator();
        if ui.button("Open on GitHub").clicked() {
            acts.push(Action::OpenUrl(url.to_string()));
        }
    });
}

/// `id` keeps each comment's widget state apart; the URL is unique.
fn markdown(app: &mut App, ui: &mut Ui, p: &Palette, body: &str, id: &str, repo: &str) {
    if body.trim().is_empty() {
        ui.label(RichText::new("No description provided.").italics().color(p.fg_muted));
        return;
    }
    let text = app.markdown_text(body, repo);
    if let Some(url) = crate::markdown::show(ui, ui.id().with(id), &text) {
        app.actions.push(Action::OpenUrl(url));
    }
}

fn review_event(ui: &mut Ui, p: &Palette, r: &Review) {
    let (icon, badge_bg, badge_fg, verb) = match r.state.as_str() {
        "APPROVED" => (Icon::Check, p.open_emphasis, Color32::WHITE, "approved these changes"),
        "CHANGES_REQUESTED" => (Icon::FileDiff, p.closed_emphasis, Color32::WHITE, "requested changes"),
        "DISMISSED" => (Icon::X, p.canvas_subtle, p.fg_muted, "had their review dismissed"),
        "PENDING" => (Icon::Eye, p.canvas_subtle, p.attention, "has a pending review (only you can see it)"),
        _ => (Icon::Eye, p.canvas_subtle, p.fg_muted, "reviewed"),
    };
    let login = r.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
    ui.horizontal(|ui| {
        ui.add_space(GUTTER + 4.0);
        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
        ui.painter().circle(rect.center(), 16.0, badge_bg, Stroke::new(2.0, p.canvas));
        icons::paint(ui.painter(), rect.shrink(8.0), icon, badge_fg);
        ui.add_space(4.0);
        let av = r.author.as_ref().map(|a| a.avatar_url.as_str()).unwrap_or("");
        avatar(ui, av, 20.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(login).font(theme::bold(14.0)).color(p.fg));
        if is_bot(login, av) {
            pill(ui, "bot", theme::body(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
        }
        ui.label(RichText::new(verb).color(p.fg_muted));
        ui.label(RichText::new(util::ago(r.submitted_at.as_deref().unwrap_or(""))).color(p.fg_muted));
    });
}

/// A conversation on a line of code, like GitHub: a header with the file
/// (click to fold), the diff lines it points at edge to edge, the comments,
/// and a gray footer with the reply box and Resolve. Resolved ones start folded.
fn thread_box(app: &mut App, ui: &mut Ui, p: &Palette, t: &Thread, repo: &str) {
    let Some(first) = t.comments.nodes.first() else { return };
    // `open_threads` holds threads you toggled away from their default.
    let folded = t.is_resolved != app.open_threads.contains(&t.id);
    let busy = app.busy_threads.contains(&t.id);
    let pr_author = app.selected.as_ref().and_then(|s| s.author.as_ref()).map(|a| a.login.clone()).unwrap_or_default();
    let mut acts = Vec::new();
    ui.horizontal_top(|ui| {
        ui.add_space(GUTTER);
        // A frame inherits its parent's layout; this one must stack vertically.
        ui.vertical(|ui| egui::Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(6).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            // Header.
            let radius = if folded { CornerRadius::same(6) } else { CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 } };
            let head = egui::Frame::new().fill(p.canvas_subtle).corner_radius(radius).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    icons::show(ui, if folded { Icon::ChevronRight } else { Icon::ChevronDown }, 16.0, p.fg_muted);
                    let line = t.line.or(t.original_line).map(|l| format!(":{l}")).unwrap_or_default();
                    let path_color = if t.is_resolved { p.fg_muted } else { p.fg };
                    // Long paths lose their start, so the file name stays.
                    let badges = if t.is_outdated { 80.0 } else { 0.0 } + if t.is_resolved { 170.0 } else if folded { 90.0 } else { 0.0 };
                    let room = (ui.available_width() - badges - 12.0).max(80.0);
                    let shown = super::elide_front(ui.painter(), &format!("{}{line}", t.path), theme::mono(12.0), room);
                    let g = ui.painter().layout_no_wrap(shown, theme::mono(12.0), path_color);
                    let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                    let hovered = resp.hovered();
                    ui.painter().galley(r.min, g, path_color);
                    if hovered {
                        ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, p.accent));
                    }
                    if resp.tip(format!("{}{line} · Open on GitHub", t.path)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        acts.push(Action::OpenUrl(first.url.clone()));
                    }
                    if t.is_outdated {
                        pill(ui, "Outdated", theme::bold(12.0), Color32::TRANSPARENT, p.attention, p.attention.gamma_multiply(0.5));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if busy {
                            ui.add(egui::Spinner::new().size(16.0));
                        }
                        if t.is_resolved {
                            let who = t.resolved_by.as_ref().map(|a| format!(" by {}", a.login)).unwrap_or_default();
                            ui.label(RichText::new(format!("Resolved{who}")).size(12.0).color(p.fg_muted));
                            icons::show(ui, Icon::Check, 14.0, p.fg_muted);
                        } else if folded {
                            let n = t.comments.nodes.len();
                            ui.label(RichText::new(format!("{n} comment{}", if n == 1 { "" } else { "s" })).size(12.0).color(p.fg_muted));
                        }
                    });
                });
            });
            let toggle = ui.interact(head.response.rect, egui::Id::new(("thread-head", &t.id)), Sense::click());
            if toggle.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                acts.push(Action::ToggleThread(t.id.clone()));
            }
            if folded {
                return;
            }
            row_separator(ui);
            diff_tail(ui, p, &first.diff_hunk, &t.path);
            // Comments.
            egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 4 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, c) in t.comments.nodes.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(8.0);
                        row_separator(ui);
                        ui.add_space(12.0);
                    }
                    let login = c.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let av = c.author.as_ref().map(|a| a.avatar_url.as_str()).unwrap_or("");
                        avatar(ui, av, 24.0);
                        ui.label(RichText::new(login).font(theme::bold(14.0)).color(p.fg));
                        if is_bot(login, av) {
                            pill(ui, "bot", theme::body(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
                        }
                        if link_styled(ui, &util::ago(&c.created_at), theme::body(13.0), p.fg_muted).clicked() {
                            acts.push(Action::OpenUrl(c.url.clone()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            comment_menu(ui, p, &c.url, &c.body, &mut acts);
                            if !pr_author.is_empty() && login == pr_author {
                                pill(ui, "Author", theme::bold(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    // Indent under the avatar, in a box of fixed width.
                    let mut body = ui.available_rect_before_wrap();
                    body.min.x += 30.0;
                    ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                        markdown(app, ui, p, &c.body, &c.url, repo)
                    });
                }
                ui.add_space(8.0);
            });
            // Footer.
            row_separator(ui);
            egui::Frame::new()
                .fill(p.canvas_subtle)
                .corner_radius(CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 })
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    thread_footer(app, ui, p, t, busy, &mut acts);
                });
        }));
    });
    app.actions.extend(acts);
}

/// The reply box (or the "Reply…" field that opens it), then Resolve.
fn thread_footer(app: &mut App, ui: &mut Ui, p: &Palette, t: &Thread, busy: bool, acts: &mut Vec<Action>) {
    let resolve_button = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if t.is_resolved && t.viewer_can_unresolve {
            if ui.add_enabled(!busy, button("Unresolve conversation", p)).clicked() {
                acts.push(Action::ResolveThread(t.id.clone(), false));
            }
        } else if !t.is_resolved && t.viewer_can_resolve {
            if ui.add_enabled(!busy, button("Resolve conversation", p)).clicked() {
                acts.push(Action::ResolveThread(t.id.clone(), true));
            }
        }
    };
    let Some(draft) = app.thread_replies.get_mut(&t.id) else {
        if t.viewer_can_reply {
            // Your avatar beside the field, like GitHub.
            let me = (!app.viewer.is_empty()).then(|| format!("https://github.com/{}.png?size=64", app.viewer));
            let (rect, resp) = ui
                .horizontal(|ui| {
                    if let Some(url) = &me {
                        avatar(ui, url, 24.0);
                    }
                    ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click())
                })
                .inner;
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Reply…"));
            let hovered = resp.hovered();
            ui.painter().rect(rect, 6.0, p.canvas, Stroke::new(1.0, if hovered { p.fg_muted } else { p.border }), egui::StrokeKind::Inside);
            ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), egui::Align2::LEFT_CENTER, "Reply…", theme::body(14.0), p.fg_muted);
            if resp.on_hover_cursor(egui::CursorIcon::Text).clicked() {
                acts.push(Action::StartReply(t.id.clone()));
            }
        }
        ui.horizontal(|ui| resolve_button(ui, acts));
        return;
    };
    // Write / Preview, like the main comment box.
    let preview_id = egui::Id::new(("reply-preview", &t.id));
    let mut preview = ui.ctx().data(|d| d.get_temp::<bool>(preview_id)).unwrap_or(false);
    let mut format = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (label, on) in [("Write", false), ("Preview", true)] {
            if composer_tab(ui, p, label, preview == on).clicked() {
                preview = on;
            }
        }
        // The same formatting buttons as the main comment box.
        if !preview {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for f in Format::ALL.iter().rev() {
                    if toolbar_button(ui, p, *f).clicked() {
                        format = Some(*f);
                    }
                }
            });
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(preview_id, preview));
    if preview {
        let body = draft.clone();
        egui::Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(6).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(60.0);
            if body.trim().is_empty() {
                ui.label(RichText::new("Nothing to preview").color(p.fg_muted));
            } else {
                // Rendered read-only; links in a preview don't need to work.
                let text = crate::gfm::to_markdown(&util::clean_markdown(&body), "", &app.emojis);
                let _ = crate::markdown::show(ui, preview_id.with("md"), &text);
            }
        });
    }
    let Some(draft) = app.thread_replies.get_mut(&t.id) else { return };
    let edit_id = egui::Id::new(("reply", &t.id));
    if let Some(f) = format {
        let n = draft.chars().count();
        let (start, end) = egui::TextEdit::load_state(ui.ctx(), edit_id)
            .and_then(|s| s.cursor.char_range())
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b), a.max(b))
            })
            .unwrap_or((n, n));
        let (a, b) = f.apply(draft, start, end);
        set_cursor(ui.ctx(), edit_id, a, b);
        ui.memory_mut(|m| m.request_focus(edit_id));
    }
    let mut edit_resp = None;
    if !preview {
        edit_resp = Some(ui.add(
            egui::TextEdit::multiline(draft)
                .id(edit_id)
                .hint_text("Reply… (Markdown supported, ⌘Enter to send)")
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .font(theme::body(14.0))
                .margin(vec2(8.0, 8.0)),
        ));
    }
    let Some(edit) = edit_resp else {
        let empty = draft.trim().is_empty();
        ui.horizontal(|ui| {
            resolve_button(ui, acts);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!busy && !empty, primary_button("Reply", p)).clicked() {
                    acts.push(Action::SendReply(t.id.clone()));
                }
                if ui.add_enabled(!busy, button("Cancel", p)).clicked() {
                    acts.push(Action::CancelReply(t.id.clone()));
                }
            });
        });
        return;
    };
    // Focus the box when it first opens.
    if draft.is_empty() && !edit.has_focus() && ui.memory(|m| m.focused().is_none()) {
        edit.request_focus();
    }
    let send = edit.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
    let cancel = edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
    let empty = draft.trim().is_empty();
    ui.horizontal(|ui| {
        resolve_button(ui, acts);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!busy && !empty, primary_button("Reply", p)).clicked() || (send && !empty) {
                acts.push(Action::SendReply(t.id.clone()));
            }
            if ui.add_enabled(!busy, button("Cancel", p)).clicked() || cancel {
                acts.push(Action::CancelReply(t.id.clone()));
            }
        });
    });
}

/// The last few diff lines a thread points at, with old and new line
/// numbers, edge to edge like GitHub.
fn diff_tail(ui: &mut Ui, p: &Palette, hunk: &str, path: &str) {
    let lines: Vec<&str> = hunk.lines().collect();
    // Work out line numbers from the "@@ -a,b +c,d @@" header.
    let mut numbered = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for l in &lines {
        if l.starts_with("@@") {
            let num = |prefix: char| -> u32 {
                l.split_whitespace().find(|t| t.starts_with(prefix)).and_then(|t| t[1..].split(',').next()?.parse().ok()).unwrap_or(1)
            };
            (old, new) = (num('-'), num('+'));
            numbered.push((None, None, *l));
            continue;
        }
        match l.chars().next() {
            Some('+') => {
                numbered.push((None, Some(new), *l));
                new += 1;
            }
            Some('-') => {
                numbered.push((Some(old), None, *l));
                old += 1;
            }
            _ => {
                numbered.push((Some(old), Some(new), *l));
                old += 1;
                new += 1;
            }
        }
    }
    let tail_start = numbered.len().saturating_sub(4);
    let tail = &numbered[tail_start..];
    if tail.is_empty() {
        return;
    }
    // The whole hunk goes through the highlighter, so the lines shown have
    // the context they need; the "+"/"-" markers stay out of it.
    let code = |l: &str| if l.starts_with("@@") { String::new() } else { l.get(1..).unwrap_or("").replace('\t', "    ") };
    let colors = crate::syntax::lang_for_diff(path, hunk).and_then(|lang| {
        let lines: Vec<String> = numbered.iter().map(|(_, _, l)| code(l)).collect();
        crate::syntax::highlight_fragment(lang, &lines.iter().map(String::as_str).collect::<Vec<_>>())
    });
    let num_w = 44.0;
    // Long lines scroll sideways, like GitHub; the numbers scroll with them.
    let widest = tail.iter().map(|(_, _, l)| ui.painter().layout_no_wrap(code(l), theme::mono(12.0), p.fg).size().x).fold(0.0, f32::max);
    let row_w = ui.available_width().max(num_w * 2.0 + 10.0 + 14.0 + widest + 16.0);
    let scroll_id = ui.id().with(("snippet", hunk.len(), path));
    egui::ScrollArea::horizontal().id_salt(scroll_id).auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded).show(ui, |ui| {
    ui.spacing_mut().item_spacing.y = 0.0;
    for (k, (o, n, l)) in tail.iter().enumerate() {
        let (bg, num_bg) = match l.chars().next() {
            Some('+') => (p.diff_add, p.diff_add_num),
            Some('-') => (p.diff_del, p.diff_del_num),
            Some('@') => (p.diff_hunk, p.diff_hunk_num),
            _ => (p.canvas, p.canvas),
        };
        let (rect, _) = ui.allocate_exact_size(vec2(row_w, 20.0), Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, bg);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(num_w * 2.0, 20.0)), 0.0, num_bg);
        for (v, x) in [(o, num_w), (n, num_w * 2.0)] {
            if let Some(v) = v {
                painter.text(pos2(rect.left() + x - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, v.to_string(), theme::mono(12.0), p.fg_muted);
            }
        }
        let x = rect.left() + num_w * 2.0 + 10.0;
        if l.starts_with("@@") {
            painter.text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, *l, theme::mono(12.0), p.fg_muted);
            continue;
        }
        let marker = l.get(..1).unwrap_or(" ");
        painter.text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, marker, theme::mono(12.0), p.fg_muted);
        let text = code(l);
        let runs = colors.as_ref().and_then(|c| c.get(tail_start + k)).map(Vec::as_slice);
        let g = painter.layout_job(crate::syntax::job(&text, runs, theme::mono(12.0), p.fg, p));
        painter.galley(pos2(x + 14.0, rect.center().y - g.size().y / 2.0), g, p.fg);
    }
    });
}

/// The status box at the bottom: reviews, checks, and whether it can merge.
fn merge_box(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    struct Line {
        icon: Icon,
        color: Color32,
        title: String,
        detail: String,
        goto: Option<Tab>,
    }
    let mut lines = Vec::new();
    let big_icon;
    let big_color;
    match d.state.as_str() {
        "MERGED" => {
            big_icon = Icon::PrMerged;
            big_color = p.merged;
            lines.push(Line { icon: Icon::PrMerged, color: p.merged, title: "Pull request successfully merged and closed".into(), detail: format!("The {} branch was merged into {}.", d.head_ref_name, d.base_ref_name), goto: None });
        }
        "CLOSED" => {
            big_icon = Icon::PrClosed;
            big_color = p.closed;
            lines.push(Line { icon: Icon::PrClosed, color: p.closed, title: "Closed with unmerged commits".into(), detail: "This pull request is closed.".into(), goto: None });
        }
        _ => {
            // Same rules as the bar at the top (see detail_merge_status):
            // the reviewers' own verdicts, since reviewDecision ignores rulesets.
            let author = d.author.as_ref().map(|a| a.login.as_str()).unwrap_or("");
            let verdicts: Vec<&str> = d
                .latest_opinionated_reviews
                .nodes
                .iter()
                .filter(|r| r.author.as_ref().is_none_or(|a| a.login != author))
                .map(|r| r.state.as_str())
                .collect();
            let decision = if verdicts.contains(&"CHANGES_REQUESTED") {
                Some("CHANGES_REQUESTED")
            } else if verdicts.contains(&"APPROVED") {
                Some("APPROVED")
            } else {
                Some("REVIEW_REQUIRED")
            };
            match decision {
                Some("APPROVED") => lines.push(Line { icon: Icon::Check, color: p.open, title: "Changes approved".into(), detail: approvals(d), goto: None }),
                Some("CHANGES_REQUESTED") => lines.push(Line { icon: Icon::X, color: p.closed, title: "Changes requested".into(), detail: "A reviewer asked for changes before this can merge.".into(), goto: None }),
                Some("REVIEW_REQUIRED") => lines.push(Line { icon: Icon::Dot, color: p.attention, title: "Review required".into(), detail: "At least one approving review is required by reviewers with write access.".into(), goto: None }),
                _ => {}
            }
            if !d.checks.is_empty() {
                let s = CheckSummary::of(&d.checks);
                let (icon, color, title) = if s.failed > 0 {
                    (Icon::X, p.closed, "Some checks were not successful")
                } else if s.pending > 0 {
                    (Icon::Dot, p.attention, "Some checks haven't completed yet")
                } else {
                    (Icon::Check, p.open, "All checks have passed")
                };
                lines.push(Line { icon, color, title: title.into(), detail: s.describe(), goto: Some(Tab::Checks) });
            }
            let unresolved = d.review_threads.nodes.iter().filter(|t| !t.is_resolved).count();
            if unresolved > 0 {
                let title = if unresolved == 1 { "1 unresolved conversation".to_string() } else { format!("{unresolved} unresolved conversations") };
                lines.push(Line { icon: Icon::Comment, color: p.closed, title, detail: "Conversations must be resolved before merging.".into(), goto: None });
            }
            if d.is_draft {
                lines.push(Line { icon: Icon::PrDraft, color: p.neutral_emphasis, title: "This pull request is still a work in progress".into(), detail: "Draft pull requests cannot be merged.".into(), goto: None });
            } else {
                match d.mergeable.as_str() {
                    "MERGEABLE" => lines.push(Line { icon: Icon::Check, color: p.open, title: "No conflicts with base branch".into(), detail: "Merging can be performed automatically.".into(), goto: None }),
                    "CONFLICTING" => lines.push(Line { icon: Icon::X, color: p.closed, title: "This branch has conflicts that must be resolved".into(), detail: "Resolve conflicts on GitHub or on the command line.".into(), goto: None }),
                    _ => lines.push(Line { icon: Icon::Dot, color: p.fg_muted, title: "Checking for the ability to merge automatically…".into(), detail: String::new(), goto: None }),
                }
            }
            let ok = lines.iter().all(|l| l.color == p.open);
            big_icon = Icon::PrMerged;
            // Same color as the bar at the top, so the two never disagree.
            let bar = detail_merge_status(p, d, None).map(|m| m.color);
            big_color = if ok { p.open_emphasis } else if d.is_draft { p.neutral_emphasis } else { bar.unwrap_or(p.neutral_emphasis) };
        }
    }
    let open = d.state == "OPEN";
    let border = if big_color == p.open_emphasis { p.open } else { p.border };
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(AVATAR, AVATAR), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, big_color);
        icons::paint(ui.painter(), rect.shrink(10.0), big_icon, Color32::WHITE);
        ui.add_space(16.0 - ui.spacing().item_spacing.x);
        ui.vertical(|ui| {
            egui::Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, border)).corner_radius(6).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let n = lines.len();
                for (i, l) in lines.into_iter().enumerate() {
                    let resp = egui::Frame::new()
                        .inner_margin(Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                                ui.painter().circle_filled(rect.center(), 16.0, l.color);
                                icons::paint(ui.painter(), rect.shrink(9.0), l.icon, Color32::WHITE);
                                ui.add_space(4.0);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.label(RichText::new(&l.title).font(theme::bold(14.0)).color(p.fg));
                                    if !l.detail.is_empty() {
                                        ui.label(RichText::new(&l.detail).size(13.0).color(p.fg_muted));
                                    }
                                });
                                // A draft's "Ready for review" sits at the end of its row.
                                if l.icon == Icon::PrDraft && d.is_draft && open {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.set_min_height(32.0);
                                        merge_actions(app, ui, p, d);
                                    });
                                }
                            });
                        })
                        .response;
                    if let Some(t) = l.goto {
                        let resp = resp.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            app.actions.push(Action::Tab(t));
                        }
                    }
                    if i + 1 < n {
                        row_separator(ui);
                    }
                }
                if open && !d.is_draft {
                    row_separator(ui);
                    egui::Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        merge_footer(app, ui, p, d);
                    });
                }
            });
        });
    });
}

/// Bottom of the merge box: GitHub's split "Squash and merge ▾" button,
/// or auto-merge controls.
fn merge_footer(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    ui.horizontal(|ui| {
        if let Some(a) = d.auto_merge_request.as_ref().filter(|_| !app.posting) {
            icons::show(ui, Icon::AutoMerge, 16.0, p.open);
            let who = a.enabled_by.as_ref().map(|e| format!(" by {}", e.login)).unwrap_or_default();
            ui.label(RichText::new(format!("Auto-merge enabled{who}")).font(theme::bold(14.0)).color(p.fg));
            ui.label(RichText::new("It will merge when all requirements are met.").size(13.0).color(p.fg_muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| merge_actions(app, ui, p, d));
            return;
        }
        merge_actions(app, ui, p, d);
    });
}

fn approvals(d: &PrDetail) -> String {
    let mut who: Vec<&str> = d
        .reviews
        .nodes
        .iter()
        .filter(|r| r.state == "APPROVED")
        .filter_map(|r| r.author.as_ref().map(|a| a.login.as_str()))
        .collect();
    who.dedup();
    match who.len() {
        0 => "This pull request has been approved.".into(),
        n => format!("{n} approving review{} by {}.", if n == 1 { "" } else { "s" }, who.join(", ")),
    }
}

fn composer(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    // Until we know who you are, assume it might be yours: authors can't approve.
    let own = app.viewer.is_empty() || d.author.as_ref().is_some_and(|a| a.login == app.viewer);
    let showing_preview = app.composer_preview;
    let mut set_preview = None;
    let edit_id = egui::Id::new("composer");
    let mut format: Option<Format> = None;
    ui.horizontal_top(|ui| {
        let url = if app.viewer.is_empty() { String::new() } else { format!("https://github.com/{}.png?size=80", app.viewer) };
        ui.vertical(|ui| {
            ui.add_space(32.0);
            avatar(ui, &url, AVATAR);
        });
        ui.add_space(16.0 - ui.spacing().item_spacing.x);
        ui.vertical(|ui| {
            ui.label(RichText::new("Add a comment").font(theme::bold(16.0)).color(p.fg));
            ui.add_space(8.0);
            egui::Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(6).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                // Header: Write / Preview tabs on the left, formatting on the right.
                let head = egui::Frame::new()
                    .fill(p.canvas_subtle)
                    .corner_radius(CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                    .inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 0 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for (label, preview) in [("Write", false), ("Preview", true)] {
                                if composer_tab(ui, p, label, showing_preview == preview).clicked() {
                                    set_preview = Some(preview);
                                }
                            }
                            if !showing_preview {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for f in Format::ALL.iter().rev() {
                                        if toolbar_button(ui, p, *f).clicked() {
                                            format = Some(*f);
                                        }
                                    }
                                });
                            }
                        });
                    });
                // The line under the tabs, broken where the active tab joins the body.
                let r = head.response.rect;
                let _ = r;
                egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if showing_preview {
                        egui::Frame::new().inner_margin(Margin::symmetric(8, 8)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_min_height(100.0);
                            if app.composer.trim().is_empty() {
                                ui.label(RichText::new("Nothing to preview").color(p.fg_muted));
                            } else {
                                let body = app.composer.clone();
                                markdown(app, ui, p, &body, "composer-preview", &d.repository.name_with_owner);
                            }
                        });
                    } else {
                        let edit = ui.add(
                            egui::TextEdit::multiline(&mut app.composer)
                                .id(edit_id)
                                .hint_text("Leave a comment")
                                .desired_rows(5)
                                .desired_width(f32::INFINITY)
                                .font(theme::body(14.0))
                                .margin(vec2(8.0, 8.0)),
                        );
                        if app.focus_composer {
                            app.focus_composer = false;
                            edit.request_focus();
                            edit.scroll_to_me(Some(egui::Align::Center));
                            // Put the cursor at the end, after the quote.
                            let end = app.composer.chars().count();
                            set_cursor(ui.ctx(), edit_id, end, end);
                        }
                        if edit.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                            app.actions.push(Action::Comment);
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        // The hint gives way to the buttons.
                        let buttons_w = 120.0 + if d.state == "OPEN" { 175.0 } else { 0.0 } + if !own && d.state == "OPEN" { 160.0 } else { 0.0 };
                        let hint_w = (ui.available_width() - buttons_w).max(0.0);
                        // Whole hint or none: a cut-off hint is just noise.
                        let hint = ["Markdown is supported · ⌘Enter to comment", "Markdown is supported"]
                            .map(|t| ui.painter().layout_no_wrap(t.into(), theme::body(12.0), p.fg_muted))
                            .into_iter()
                            .find(|g| g.size().x <= hint_w);
                        if let Some(g) = hint {
                            let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(r.min, g, p.fg_muted);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(!app.posting, |ui| {
                                let empty = app.composer.trim().is_empty();
                                let comment = ui.add_enabled(!empty, primary_button("Comment", p));
                                if comment.clicked() {
                                    app.actions.push(Action::Comment);
                                }
                                let btn_h = comment.rect.height();
                                if !own && d.state == "OPEN" {
                                    let resp = ui.add(button("Review changes  ▾", p));
                                    egui::Popup::menu(&resp).show(|ui| {
                                        ui.set_width(260.0);
                                        ui.label(RichText::new("Uses the text in the comment box, if any.").size(12.0).color(p.fg_muted));
                                        ui.separator();
                                        if ui.button("Approve").tip("Submit an approving review").clicked() {
                                            app.actions.push(Action::Review("APPROVE"));
                                        }
                                        if ui.button("Request changes").tip("Needs a comment explaining what to change").clicked() {
                                            app.actions.push(Action::Review("REQUEST_CHANGES"));
                                        }
                                    });
                                }
                                // Same buttons in the same order every time.
                                if d.state == "OPEN" {
                                    // Default button, red icon, like GitHub.
                                    let g = ui.painter().layout_no_wrap("Close pull request".into(), theme::bold(14.0), p.fg);
                                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0 + 30.0, btn_h), Sense::click());
                                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close pull request"));
                                    let fill = if resp.hovered() { ui.visuals().widgets.hovered.weak_bg_fill } else { p.btn_bg };
                                    ui.painter().rect(r, 6.0, fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                                    icons::paint(ui.painter(), Rect::from_min_size(pos2(r.left() + 12.0, r.center().y - 8.0), vec2(16.0, 16.0)), Icon::PrClosed, p.closed);
                                    ui.painter().galley(pos2(r.left() + 34.0, r.center().y - g.size().y / 2.0), g, p.fg);
                                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                        app.actions.push(Action::ClosePr);
                                    }
                                }
                                if app.posting {
                                    ui.add(egui::Spinner::new().size(16.0));
                                }
                            });
                        });
                    });
                });
            });
        });
    });
    if let Some(v) = set_preview {
        app.composer_preview = v;
    }
    if let Some(f) = format {
        let (start, end) = egui::TextEdit::load_state(ui.ctx(), edit_id)
            .and_then(|s| s.cursor.char_range())
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b), a.max(b))
            })
            .unwrap_or_else(|| {
                let n = app.composer.chars().count();
                (n, n)
            });
        let (a, b) = f.apply(&mut app.composer, start, end);
        set_cursor(ui.ctx(), edit_id, a, b);
        ui.memory_mut(|m| m.request_focus(edit_id));
    }
}

fn set_cursor(ctx: &egui::Context, id: egui::Id, a: usize, b: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let range = egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// GitHub's Write / Preview tabs: the active one is white and joins the box.
fn composer_tab(ui: &mut Ui, p: &Palette, label: &str, active: bool) -> egui::Response {
    let font = if active { theme::bold(14.0) } else { theme::body(14.0) };
    let g = ui.painter().layout_no_wrap(label.to_string(), font, if active { p.fg } else { p.fg_muted });
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, 36.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    if active {
        let r = rect.expand2(vec2(0.0, 0.5));
        ui.painter().rect_filled(r, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, p.canvas);
        let s = Stroke::new(1.0, p.border);
        ui.painter().line_segment([r.left_bottom(), r.left_top() + vec2(0.0, 6.0)], s);
        ui.painter().line_segment([r.left_top() + vec2(6.0, 0.0), r.right_top() - vec2(6.0, 0.0)], s);
        ui.painter().line_segment([r.right_top() + vec2(0.0, 6.0), r.right_bottom()], s);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 4.0)), 6.0, p.border.gamma_multiply(0.4));
    }
    let c = if active { p.fg } else { p.fg_muted };
    ui.painter().galley(rect.center() - g.size() / 2.0, g, c);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn toolbar_button(ui: &mut Ui, p: &Palette, f: Format) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, f.tip()));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.border.gamma_multiply(0.4));
    }
    let c = p.fg_muted;
    let at = rect.center();
    match f {
        Format::Link => icons::paint(ui.painter(), Rect::from_center_size(at, vec2(16.0, 16.0)), Icon::Link, c),
        Format::Task => {
            let b = Rect::from_center_size(at, vec2(12.0, 12.0));
            ui.painter().rect_stroke(b, 2.0, Stroke::new(1.5, c), egui::StrokeKind::Middle);
            icons::paint(ui.painter(), b.shrink(1.0), Icon::Check, c);
        }
        Format::Bullet | Format::Number => {
            for i in 0..3 {
                let y = at.y - 5.0 + i as f32 * 5.0;
                if f == Format::Bullet {
                    ui.painter().circle_filled(pos2(at.x - 5.0, y), 1.3, c);
                } else {
                    ui.painter().text(pos2(at.x - 5.0, y), egui::Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold(6.0), c);
                }
                ui.painter().line_segment([pos2(at.x - 2.0, y), pos2(at.x + 7.0, y)], Stroke::new(1.5, c));
            }
        }
        _ => {
            let (text, font) = match f {
                Format::Heading => ("H", theme::bold(14.0)),
                Format::Bold => ("B", theme::bold(14.0)),
                Format::Italic => ("I", theme::body(14.0)),
                Format::Quote => ("❝", theme::body(15.0)),
                _ => ("<>", theme::mono(12.0)),
            };
            let mut job = egui::text::LayoutJob::single_section(text.into(), egui::TextFormat { font_id: font, color: c, italics: f == Format::Italic, ..Default::default() });
            job.halign = egui::Align::Center;
            let g = ui.painter().layout_job(job);
            ui.painter().galley(at - vec2(0.0, g.size().y / 2.0), g, c);
        }
    }
    resp.tip(f.tip()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The composer toolbar's Markdown shortcuts.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Format {
    Heading,
    Bold,
    Italic,
    Quote,
    Code,
    Link,
    Bullet,
    Number,
    Task,
}

impl Format {
    const ALL: [Format; 9] =
        [Format::Heading, Format::Bold, Format::Italic, Format::Quote, Format::Code, Format::Link, Format::Bullet, Format::Number, Format::Task];

    fn tip(self) -> &'static str {
        match self {
            Format::Heading => "Heading",
            Format::Bold => "Bold",
            Format::Italic => "Italic",
            Format::Quote => "Quote",
            Format::Code => "Code",
            Format::Link => "Link",
            Format::Bullet => "Bulleted list",
            Format::Number => "Numbered list",
            Format::Task => "Task list",
        }
    }

    /// Applies to chars `start..end` of `text`. Returns the new selection.
    fn apply(self, text: &mut String, start: usize, end: usize) -> (usize, usize) {
        let chars: Vec<char> = text.chars().collect();
        let (start, end) = (start.min(chars.len()), end.min(chars.len()));
        let before: String = chars[..start].iter().collect();
        let picked: String = chars[start..end].iter().collect();
        let after: String = chars[end..].iter().collect();
        let len = |s: &str| s.chars().count();
        let wrap = |open: &str, close: &str, placeholder: &str| {
            let inner = if picked.is_empty() { placeholder.to_string() } else { picked.clone() };
            let a = start + len(open);
            (format!("{before}{open}{inner}{close}{after}"), (a, a + len(&inner)))
        };
        let prefix_lines = |make: &dyn Fn(usize) -> String| {
            // Start at the beginning of the first selected line.
            let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let head = &before[..line_start];
            let block = format!("{}{}", &before[line_start..], picked);
            let prefixed: Vec<String> = block.split('\n').enumerate().map(|(i, l)| format!("{}{l}", make(i))).collect();
            let block = prefixed.join("\n");
            let a = len(head);
            let b = a + len(&block);
            (format!("{head}{block}{after}"), (b, b))
        };
        let (new, sel) = match self {
            Format::Bold => wrap("**", "**", "bold text"),
            Format::Italic => wrap("_", "_", "italic text"),
            Format::Code if picked.contains('\n') => wrap("```\n", "\n```", ""),
            Format::Code => wrap("`", "`", "code"),
            Format::Link => {
                let inner = if picked.is_empty() { "text".to_string() } else { picked.clone() };
                let a = start + 1 + len(&inner) + 2;
                (format!("{before}[{inner}](url){after}"), (a, a + 3))
            }
            Format::Heading => prefix_lines(&|_| "### ".into()),
            Format::Quote => prefix_lines(&|_| "> ".into()),
            Format::Bullet => prefix_lines(&|_| "- ".into()),
            Format::Number => prefix_lines(&|i| format!("{}. ", i + 1)),
            Format::Task => prefix_lines(&|_| "- [ ] ".into()),
        };
        *text = new;
        sel
    }
}

fn sidebar(ui: &mut Ui, p: &Palette, d: &PrDetail) {
    let section = |ui: &mut Ui, title: &str, body: &mut dyn FnMut(&mut Ui)| {
        ui.label(RichText::new(title).font(theme::bold(14.0)).color(p.fg_muted));
        ui.add_space(6.0);
        body(ui);
        ui.add_space(12.0);
        row_separator(ui);
        ui.add_space(12.0);
    };

    // Reviewers: people asked to review, plus everyone who reviewed, with
    // their latest verdict.
    let mut reviewers: Vec<(String, String, Option<&str>)> = Vec::new();
    for r in d.reviews.nodes.iter().rev() {
        if let Some(a) = &r.author {
            if a.login != d.author.as_ref().map(|x| x.login.clone()).unwrap_or_default()
                && !reviewers.iter().any(|(l, _, _)| l == &a.login)
            {
                reviewers.push((a.login.clone(), a.avatar_url.clone(), Some(r.state.as_str())));
            }
        }
    }
    for rr in &d.review_requests.nodes {
        match &rr.requested_reviewer {
            Some(Reviewer::User { login, avatar_url }) if !reviewers.iter().any(|(l, _, _)| l == login) => {
                reviewers.push((login.clone(), avatar_url.clone(), None))
            }
            Some(Reviewer::Team { name }) => reviewers.push((name.clone(), String::new(), None)),
            _ => {}
        }
    }
    section(ui, "Reviewers", &mut |ui| {
        if reviewers.is_empty() {
            ui.label(RichText::new("No reviews").size(13.0).color(p.fg_muted));
        }
        for (login, url, state) in &reviewers {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                avatar(ui, url, 24.0);
                ui.label(RichText::new(login).font(theme::bold(14.0)).color(p.fg));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, color) = match *state {
                        Some("APPROVED") => (Icon::Check, p.open),
                        Some("CHANGES_REQUESTED") => (Icon::FileDiff, p.closed),
                        Some(_) => (Icon::Comment, p.fg_muted),
                        None => (Icon::Dot, p.attention),
                    };
                    let size = if state.is_none() { 10.0 } else { 16.0 };
                    icons::show(ui, icon, size, color);
                });
            });
        }
    });
    section(ui, "Assignees", &mut |ui| {
        if d.assignees.nodes.is_empty() {
            ui.label(RichText::new("No one assigned").size(13.0).color(p.fg_muted));
        }
        for a in &d.assignees.nodes {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                avatar(ui, &a.avatar_url, 24.0);
                ui.label(RichText::new(&a.login).font(theme::bold(14.0)).color(p.fg));
            });
        }
    });
    section(ui, "Labels", &mut |ui| {
        if d.labels.nodes.is_empty() {
            ui.label(RichText::new("None yet").size(13.0).color(p.fg_muted));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            // Wrapped rows are otherwise a full button height apart.
            ui.spacing_mut().interact_size.y = 0.0;
            let mut labels: Vec<&_> = d.labels.nodes.iter().collect();
            labels.sort_by_key(|l| l.name.to_lowercase());
            for l in labels {
                let (bg, fg, border) = theme::label_colors(&l.color, p);
                pill(ui, &l.name, theme::bold(13.0), bg, fg, border);
            }
        });
    });
}

// ---------- Commits ----------

fn commits(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail) {
    let nodes = &d.commits.nodes;
    if (d.commits.total_count as usize) > nodes.len() {
        ui.label(RichText::new(format!("Showing the latest {} of {} commits.", nodes.len(), d.commits.total_count)).color(p.fg_muted).size(13.0));
        ui.add_space(8.0);
    }
    let repo = &d.repository.name_with_owner;
    // The head commit's status comes from the checks list, where policy-bot
    // is ignored; GitHub's own rollup would count it.
    let head_state = (!d.checks.is_empty()).then(|| CheckSummary::of(&d.checks).state());
    // A line runs through the day markers, like GitHub's commit timeline.
    let line_slot = ui.painter().add(egui::Shape::Noop);
    let line_x = ui.min_rect().left() + 8.0;
    let (mut line_top, mut line_bottom) = (None, 0.0f32);
    let mut i = 0;
    while i < nodes.len() {
        let day = util::day(&nodes[i].commit.committed_date);
        let mut j = i;
        while j < nodes.len() && util::day(&nodes[j].commit.committed_date) == day {
            j += 1;
        }
        let head = ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 9.0, p.canvas);
            icons::paint(ui.painter(), rect, Icon::Commit, p.fg_muted);
            ui.label(RichText::new(format!("Commits on {day}")).size(14.0).color(p.fg_muted));
        });
        line_top.get_or_insert(head.response.rect.center().y);
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.add_space(28.0);
            ui.vertical(|ui| {
                plain_box(ui, |ui| {
                    for (k, n) in nodes[i..j].iter().enumerate() {
                        let c = &n.commit;
                        let is_head = i + k + 1 == nodes.len();
                        commit_row(app, ui, p, d, repo, c, if is_head { head_state.or(c.status_check_rollup.as_ref().map(|r| r.state.as_str())) } else { c.status_check_rollup.as_ref().map(|r| r.state.as_str()) });
                        if k + 1 < j - i {
                            row_separator(ui);
                        }
                    }
                });
            });
        });
        line_bottom = ui.cursor().top();
        ui.add_space(16.0);
        i = j;
    }
    if let Some(top) = line_top {
        ui.painter().set(line_slot, egui::Shape::line_segment([pos2(line_x, top), pos2(line_x, line_bottom)], Stroke::new(2.0, p.border_muted)));
    }
}

fn commit_row(app: &mut App, ui: &mut Ui, p: &Palette, d: &PrDetail, repo: &str, c: &crate::github::Commit, state: Option<&str>) {
    let url = format!("{}/commits/{}", d.url, c.oid);
    egui::Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let verified = c.signature.as_ref().is_some_and(|s| s.is_valid);
            let right_w = 150.0 + if verified { 76.0 } else { 0.0 };
            let w = (ui.available_width() - right_w).max(120.0);
            let body_id = egui::Id::new(("commit-body", &c.oid));
            let body_open = ui.ctx().data(|d| d.get_temp::<bool>(body_id)).unwrap_or(false);
            let has_body = !c.message_body.trim().is_empty();
            ui.allocate_ui(vec2(w, 0.0), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    // Rows only as tall as their text, like GitHub's.
                    ui.spacing_mut().interact_size.y = 20.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let icon_w = if state.is_some() { 22.0 } else { 0.0 } + if has_body { 30.0 } else { 0.0 };
                        let mut job = egui::text::LayoutJob::default();
                        super::append_title(&mut job, &c.message_headline, 14.0, true, 0.0, p);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(w - icon_w);
                        let g = ui.painter().layout_job(job);
                        let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                        ui.painter().galley(rect.min, g, p.fg);
                        if resp.hovered() {
                            ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.accent));
                        }
                        if resp.tip(&c.message_headline).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            app.actions.push(Action::OpenUrl(url.clone()));
                        }
                        if has_body {
                            // "⋯" shows the rest of the commit message.
                            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 16.0), Sense::click());
                            // Pressed look while the message is showing.
                            let (bg, fg) = if body_open { (p.accent_subtle, p.accent) } else if resp.hovered() { (p.border, p.fg_muted) } else { (p.border_muted, p.fg_muted) };
                            ui.painter().rect_filled(r, 4.0, bg);
                            icons::paint(ui.painter(), r.shrink2(vec2(5.0, 2.0)), Icon::Kebab, fg);
                            if resp.tip(if body_open { "Hide the full message" } else { "Show the full message" }).clicked() {
                                ui.ctx().data_mut(|d| d.insert_temp(body_id, !body_open));
                            }
                        }
                        if let Some(state) = state {
                            let (icon, color) = status_icon(state, p);
                            icons::show(ui, icon, 16.0, color);
                        }
                    });
                    if has_body && body_open {
                        egui::Frame::new().inner_margin(Margin::symmetric(0, 4)).show(ui, |ui| {
                            ui.label(RichText::new(c.message_body.trim()).font(theme::mono(12.0)).color(p.fg_muted));
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let user = c.author.as_ref().and_then(|a| a.user.as_ref());
                        avatar(ui, user.map(|u| u.avatar_url.as_str()).unwrap_or(""), 16.0);
                        let name = user.map(|u| u.login.clone()).or_else(|| c.author.as_ref().and_then(|a| a.name.clone())).unwrap_or_default();
                        ui.label(RichText::new(name).font(theme::bold(12.0)).color(p.fg));
                        ui.label(RichText::new(format!("committed {}", util::ago(&c.committed_date))).size(12.0).color(p.fg_muted));
                    });
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // Right to left: browse <>, copy, then the SHA, like GitHub.
                if icon_button(ui, Icon::Code, "Browse the repository at this point", p).clicked() {
                    app.actions.push(Action::OpenUrl(format!("https://github.com/{repo}/tree/{}", c.oid)));
                }
                if icon_button(ui, Icon::Copy, "Copy full SHA", p).clicked() {
                    app.actions.push(Action::Copy(c.oid.clone()));
                }
                // Painted, so the monospace SHA sits dead center.
                let g = ui.painter().layout_no_wrap(c.abbreviated_oid.clone(), theme::mono(12.0), p.fg_muted);
                let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 32.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &c.abbreviated_oid));
                let fill = if resp.hovered() { ui.visuals().widgets.hovered.weak_bg_fill } else { p.btn_bg };
                ui.painter().rect(r, 6.0, fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &c.abbreviated_oid, theme::mono(12.0), p.fg_muted);
                super::tip(&resp, "View commit details");
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.actions.push(Action::OpenUrl(url.clone()));
                }
                if verified {
                    ui.add_space(4.0);
                    ui.allocate_ui_with_layout(vec2(70.0, 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        pill(ui, "Verified", theme::bold(12.0), Color32::TRANSPARENT, p.open, p.open.gamma_multiply(0.6))
                            .tip("This commit was signed with a verified signature.");
                    });
                }
            });
        });
    });
}

/// "Successful in 12s", "Failing after 1m 3s", "Skipped", or what the check said.
fn check_note(c: &Check) -> String {
    let took = c.seconds.filter(|s| *s >= 0).map(|s| match s {
        0..=59 => format!("{s}s"),
        60..=3599 if s % 60 == 0 => format!("{}m", s / 60),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    });
    // Shown green on purpose (it always reports the same failure); its own
    // text would contradict the check mark.
    if crate::github::is_policy_bot(&c.name) {
        return "Ignored here: policy-bot always reports this".into();
    }
    match (check_rank(&c.result), took) {
        (2, Some(t)) => format!("Successful in {t}"),
        (0, Some(t)) => format!("Failing after {t}"),
        (3, _) if c.result == "SKIPPED" => "Skipped".into(),
        (1, _) if c.description.as_deref().is_none_or(str::is_empty) => "In progress".into(),
        _ => c.description.clone().unwrap_or_default(),
    }
}

fn checks(app: &mut App, ui: &mut Ui, p: &Palette, checks: &[Check]) {
    if checks.is_empty() {
        egui::Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(6).inner_margin(Margin::same(24)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                icons::show(ui, Icon::Checklist, 24.0, p.fg_muted);
                ui.add_space(8.0);
                ui.label(RichText::new("No checks").font(theme::bold(16.0)).color(p.fg));
                ui.label(RichText::new("Nothing reported a status on the latest commit.").size(13.0).color(p.fg_muted));
            });
        });
        return;
    }
    let s = CheckSummary::of(checks);
    // Failing, then running, then passing; required ones first in each.
    let mut sorted: Vec<&Check> = checks.iter().collect();
    sorted.sort_by_key(|c| (check_rank(&c.result), !c.required, c.group.clone().unwrap_or_default(), c.name.clone()));
    let (shown, skipped): (Vec<&Check>, Vec<&Check>) = sorted.into_iter().partition(|c| check_rank(&c.result) < 3);
    let skipped_open_id = egui::Id::new("checks-skipped-open");
    let skipped_open = ui.ctx().data(|d| d.get_temp::<bool>(skipped_open_id)).unwrap_or(false);
    let mut toggle_skipped = false;
    egui::Frame::new().fill(p.canvas).stroke(Stroke::new(1.0, p.border)).corner_radius(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (icon, color, title) = if s.failed > 0 {
                    (Icon::X, p.closed_emphasis, "Some checks were not successful")
                } else if s.pending > 0 {
                    (Icon::Dot, p.attention, "Some checks haven't completed yet")
                } else {
                    (Icon::Check, p.open_emphasis, "All checks have passed")
                };
                let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 16.0, color);
                icons::paint(ui.painter(), rect.shrink(8.0), icon, Color32::WHITE);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(title).font(theme::bold(16.0)).color(p.fg));
                    ui.label(RichText::new(s.describe()).size(13.0).color(p.fg_muted));
                });
            });
        });
        for c in &shown {
            row_separator(ui);
            check_row(app, ui, p, c);
        }
        // The fold opens downward, so its rows appear under where you clicked.
        if !skipped.is_empty() {
            row_separator(ui);
            let n = skipped.len();
            let label = format!("{n} skipped check{}", if n == 1 { "" } else { "s" });
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if resp.hovered() {
                let r = if skipped_open { 0 } else { 6 };
                ui.painter().rect_filled(rect, CornerRadius { nw: 0, ne: 0, sw: r, se: r }, p.hover_row);
            }
            let chevron = if skipped_open { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + 16.0, rect.center().y - 8.0), vec2(16.0, 16.0)), chevron, p.fg_muted);
            icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + 40.0, rect.center().y - 8.0), vec2(16.0, 16.0)), Icon::Skip, p.fg_muted);
            ui.painter().text(pos2(rect.left() + 64.0, rect.center().y), egui::Align2::LEFT_CENTER, label, theme::body(13.0), p.fg_muted);
            toggle_skipped = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
            if skipped_open {
                for c in &skipped {
                    row_separator(ui);
                    check_row(app, ui, p, c);
                }
            }
        }
    });
    if toggle_skipped {
        ui.ctx().data_mut(|d| d.insert_temp(skipped_open_id, !skipped_open));
    }
}

fn check_row(app: &mut App, ui: &mut Ui, p: &Palette, c: &Check) {
    egui::Frame::new().inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, color) = status_icon(&c.result, p);
            icons::show(ui, icon, 16.0, color);
            // Skipped checks step back: faded picture, muted name.
            let skipped = check_rank(&c.result) >= 3;
            let name_color = if skipped { p.fg_muted } else { p.fg };
            // The app's picture (Actions, CircleCI, ...).
            let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            match c.avatar.as_deref().filter(|u| !u.is_empty()) {
                Some(url) => {
                    let tint = if skipped { Color32::from_white_alpha(110) } else { Color32::WHITE };
                    egui::Image::new(url).corner_radius(6.0).tint(tint).show_loading_spinner(false).paint_at(ui, rect);
                }
                None => {
                    ui.painter().rect_filled(rect, 6.0, p.canvas_subtle);
                    icons::paint(ui.painter(), rect.shrink(3.0), Icon::Checklist, p.fg_muted);
                }
            }
            let details_w = if c.url.is_some() { 60.0 } else { 0.0 };
            let req_w = if c.required { 72.0 } else { 0.0 };
            let max = (ui.available_width() - details_w - req_w).max(80.0);
            let mut job = egui::text::LayoutJob::default();
            let name = match &c.group {
                Some(g) => format!("{g} / {}", c.name),
                None => c.name.clone(),
            };
            job.append(&name, 0.0, egui::TextFormat { font_id: theme::bold(13.0), color: name_color, ..Default::default() });
            // "(pull_request)" only if it fits whole; it's the first thing to go.
            if let Some(e) = &c.event {
                let need = ui.painter().layout_no_wrap(format!("{name} ({e})"), theme::bold(13.0), p.fg).size().x;
                let room = max - ui.painter().layout_no_wrap(check_note(c), theme::body(13.0), p.fg_muted).size().x - 8.0;
                if need <= room {
                    job.append(&format!("({e})"), 4.0, egui::TextFormat { font_id: theme::body(13.0), color: p.fg_muted, ..Default::default() });
                }
            }
            // The status ("Failing after 7m") matters more, so the name is cut first.
            let note = check_note(c);
            let note_w = ui.painter().layout_no_wrap(note.clone(), theme::body(13.0), p.fg_muted).size().x;
            // ...but the name keeps at least 60% of the row.
            let name_max = (max - note_w - 8.0).max(max * 0.6).max(80.0);
            job.wrap = egui::text::TextWrapping::truncate_at_width(name_max);
            let g = ui.painter().layout_job(job);
            let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
            ui.painter().galley(r.min, g, p.fg);
            if !note.is_empty() {
                let mut job = egui::text::LayoutJob::single_section(note.clone(), egui::TextFormat { font_id: theme::body(13.0), color: p.fg_muted, ..Default::default() });
                job.wrap = egui::text::TextWrapping::truncate_at_width((max - r.width() - 8.0).max(40.0));
                let g = ui.painter().layout_job(job);
                let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                ui.painter().galley(r.min, g, p.fg_muted);
            }
            resp.tip(format!("{name}\n{note}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(u) = &c.url {
                    if link(ui, "Details", 13.0, p).clicked() {
                        app.actions.push(Action::OpenUrl(u.clone()));
                    }
                }
                if c.required {
                    pill(ui, "Required", theme::body(12.0), Color32::TRANSPARENT, p.fg_muted, p.border);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::Format;

    fn run(f: Format, text: &str, a: usize, b: usize) -> (String, (usize, usize)) {
        let mut t = text.to_string();
        let sel = f.apply(&mut t, a, b);
        (t, sel)
    }

    #[test]
    fn formats_selection() {
        assert_eq!(run(Format::Bold, "say hi now", 4, 6), ("say **hi** now".into(), (6, 8)));
        assert_eq!(run(Format::Code, "x", 1, 1).0, "x`code`");
        assert_eq!(run(Format::Link, "go", 0, 2), ("[go](url)".into(), (5, 8)));
        assert_eq!(run(Format::Quote, "a\nb", 0, 3).0, "> a\n> b");
        assert_eq!(run(Format::Number, "one\ntwo", 1, 5).0, "1. one\n2. two");
    }
}
