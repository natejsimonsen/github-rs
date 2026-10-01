//! Left side: the github.com/pulls page. Filter buttons, search, and the list.

use super::{avatar, plain_box, pr_icon, row_separator, status_icon};
use super::Tip;
use crate::app::{Action, App, View};
use crate::github::{PrDetail, PrSummary, SECTIONS};
use crate::icons::{self, Icon};
use crate::theme::{self, Palette};
use crate::util;
use egui::{Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

pub fn show(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    subnav(app, ui, p);
    ui.add_space(12.0);
    search(app, ui, p);
    ui.add_space(12.0);

    let list = app.current_list();
    let data = list.and_then(|l| l.data.clone());
    let loading = list.is_some_and(|l| l.loading);
    let error = list.and_then(|l| l.error.clone());

    let mut retry = false;
    plain_box(ui, |ui| {
        // Box header: "44 Open   120 Closed"
        egui::Frame::new()
            .fill(p.canvas_subtle)
            .corner_radius(CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // Counts are unknown until the list loads: "– Open", not "0 Open".
                    // Both lists share one pair, so the other one's will do.
                    let other = app.lists.get(&format!("list-{}-{}", !app.closed, app.view.query())).and_then(|l| l.data.as_ref());
                    let counts = data.as_ref().map(|d| (d.open, d.closed)).or_else(|| other.map(|d| (d.open, d.closed)));
                    let counts = counts.map(|(o, c)| (o.to_string(), c.to_string()));
                    let (open, closed) = counts.unwrap_or(("–".into(), "–".into()));
                    for (is_closed, icon, n, word) in [(false, Icon::PrOpen, open, "Open"), (true, Icon::Check, closed, "Closed")] {
                        let active = app.closed == is_closed;
                        if state_toggle(ui, p, icon, &format!("{n} {word}"), active).clicked() && !active {
                            app.actions.push(Action::SetClosed(is_closed));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if loading && data.is_some() {
                            ui.add(egui::Spinner::new().size(18.0).color(p.fg_muted));
                        }
                    });
                });
            });
        row_separator(ui);

        if let Some(e) = &error {
            if data.as_ref().is_some_and(|d| !d.rows.is_empty()) {
                // Rows from before are still showing; don't push them down.
                retry = super::stale_note(ui.ctx(), "list", e);
            } else {
                egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
                    retry = super::error_banner(ui, p, "Couldn't load pull requests.", e);
                });
                row_separator(ui);
            }
        }

        let rows = data.as_ref().map(|d| d.rows.clone()).unwrap_or_default();
        if rows.is_empty() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                if loading {
                    ui.add(egui::Spinner::new().size(32.0).color(p.fg_muted));
                } else if data.is_none() && error.is_none() {
                    // Nothing came back at all (e.g. cache-only mode): don't
                    // claim the search is empty.
                    ui.label(RichText::new("Not loaded yet").font(theme::bold(16.0)).color(p.fg));
                    ui.add_space(4.0);
                    if super::link(ui, "Refresh", 14.0, p).clicked() {
                        app.actions.push(Action::Refresh);
                    }
                } else if error.is_none() {
                    icons::show(ui, Icon::PrOpen, 24.0, p.fg_muted);
                    ui.add_space(8.0);
                    let other = data.as_ref().map(|d| if app.closed { d.open } else { d.closed }).unwrap_or(0);
                    let (here, there) = if app.closed { ("closed", "open") } else { ("open", "closed") };
                    if other > 0 {
                        // Same message as the PR pane: the other half has some.
                        ui.label(RichText::new(format!("No {here} pull requests")).font(theme::bold(16.0)).color(p.fg));
                        ui.add_space(4.0);
                        if super::link(ui, &format!("View {other} {there}"), 14.0, p).clicked() {
                            app.actions.push(Action::SetClosed(!app.closed));
                        }
                    } else {
                        ui.label(RichText::new("No results matched your search.").font(theme::bold(16.0)).color(p.fg));
                        ui.add_space(4.0);
                        // Two short centered lines, so it fits a narrow list.
                        let line = |ui: &mut Ui, lead: &str, link: &str, tail: &str| {
                            let w: f32 = [lead, link, tail].iter().map(|t| ui.painter().layout_no_wrap(t.to_string(), theme::body(14.0), p.fg).size().x).sum();
                            let mut clicked = false;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                                ui.label(RichText::new(lead).size(14.0).color(p.fg_muted));
                                clicked = super::link(ui, link, 14.0, p).clicked();
                                ui.label(RichText::new(tail).size(14.0).color(p.fg_muted));
                            });
                            clicked
                        };
                        if line(ui, "You could search ", "all of GitHub", "") {
                            app.actions.push(Action::RunSearch("is:pr is:open".into()));
                        }
                        if line(ui, "or try an ", "advanced search", ".") {
                            app.actions.push(Action::OpenUrl("https://github.com/search/advanced".into()));
                        }
                    }
                }
            });
            ui.add_space(48.0);
            return;
        }

        let scroll_to = ui.ctx().data_mut(|d| d.remove_temp::<usize>(egui::Id::new("scroll-to-row")));
        // Whatever changed the selection (keys, ⌘K, a script), show that row.
        let shown_id = egui::Id::new("list-shown-selection");
        let shown: Option<String> = ui.ctx().data(|d| d.get_temp(shown_id));
        let sel_id = app.selected.as_ref().map(|s| s.id.clone());
        egui::ScrollArea::vertical().id_salt("pr-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, pr) in rows.iter().enumerate() {
                let selected = app.selected.as_ref().is_some_and(|s| s.id == pr.id);
                let detail = fresher_detail(app, pr);
                let resp = row(ui, p, pr, detail.as_deref(), selected);
                if scroll_to == Some(i) || (selected && shown != sel_id) {
                    resp.scroll_to_me(None);
                    ui.ctx().data_mut(|d| d.insert_temp(shown_id, sel_id.clone()));
                }
                if resp.clicked() {
                    app.actions.push(Action::Select(pr.clone()));
                }
                if resp.double_clicked() {
                    app.actions.push(Action::OpenUrl(pr.url.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Open on GitHub").clicked() {
                        app.actions.push(Action::OpenUrl(pr.url.clone()));
                        ui.close();
                    }
                    if ui.button("Copy link").clicked() {
                        app.actions.push(Action::Copy(pr.url.clone()));
                        ui.close();
                    }
                });
                if i + 1 < rows.len() {
                    row_separator(ui);
                }
            }
            // More pages: "Showing 10 of 809" and a button for the next 25.
            if let Some(d) = data.as_ref().filter(|d| d.next.is_some()) {
                row_separator(ui);
                let total = if app.closed { d.closed } else { d.open };
                let busy = app.loading_more.contains(&app.list_key(&app.view));
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    if busy {
                        ui.add(egui::Spinner::new().size(16.0));
                    } else if ui.add(super::button("Load more", p)).clicked() {
                        app.actions.push(Action::LoadMore);
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("Showing {} of {total}", rows.len())).size(12.0).color(p.fg_muted));
                });
                ui.add_space(16.0);
            }
        });
    });
    if retry {
        app.actions.push(Action::Refresh);
    }
}

/// "12 Open" / "34 Closed" in the list header: icon and text are one button.
fn state_toggle(ui: &mut Ui, p: &Palette, icon: Icon, text: &str, active: bool) -> egui::Response {
    let color = if active { p.fg } else { p.fg_muted };
    let font = if active { theme::bold(14.0) } else { theme::body(14.0) };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0 + 6.0 + 16.0, 32.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, text));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.hover_row);
    }
    let icon_rect = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 8.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), icon_rect, icon, color);
    ui.painter().galley(pos2(icon_rect.right() + 6.0, rect.center().y - g.size().y / 2.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The collapsed PR list: one state icon per PR, with the title on hover.
pub fn rail(app: &mut App, ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let rows = app.current_list().and_then(|l| l.data.clone()).map(|d| d.rows.clone()).unwrap_or_default();
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        // Which list this is, as an icon (GitHub never shortens labels).
        let (section, icon) = match &app.view {
            View::Section(i) => (SECTIONS[*i].title, [Icon::PrOpen, Icon::Person, Icon::Comment, Icon::Eye, Icon::Sync][*i % 5]),
            View::Search(_) => ("Search", Icon::Search),
        };
        // Clicking it opens the list, like ⌘B.
        let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 26.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Show {section}")));
        ui.painter().rect_filled(rect, 6.0, if resp.hovered() { p.accent_subtle.gamma_multiply(1.6) } else { p.accent_subtle });
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), icon, p.accent);
        super::tip(&resp, &format!("{section} · Show list (⌘B)"));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            app.actions.push(Action::TogglePanel(crate::app::Panel::List));
        }
        // A rule between the section badge and the PRs, so it doesn't read as one.
        ui.add_space(6.0);
        let y = ui.cursor().top();
        ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, p.border));
        ui.add_space(6.0);
        let more = app.current_list().and_then(|l| l.data.as_ref()).filter(|d| d.next.is_some()).map(|d| if app.closed { d.closed } else { d.open });
        let busy = app.loading_more.contains(&app.list_key(&app.view));
        let shown_id = egui::Id::new("rail-shown-selection");
        let shown: Option<String> = ui.ctx().data(|d| d.get_temp(shown_id));
        egui::ScrollArea::vertical().id_salt("pr-rail").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for pr in &rows {
            let selected = app.selected.as_ref().is_some_and(|s| s.id == pr.id);
            let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
            // A new selection (keys, ⌘K) scrolls into view, once.
            if selected && shown.as_deref() != Some(pr.id.as_str()) {
                resp.scroll_to_me(None);
                ui.ctx().data_mut(|d| d.insert_temp(shown_id, pr.id.clone()));
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("#{} {}", pr.number, pr.title)));
            if selected {
                ui.painter().rect_filled(rect, 6.0, p.selected_row);
                let bar = Rect::from_min_size(pos2(rect.left() - 6.0, rect.top() + 6.0), vec2(2.0, rect.height() - 12.0));
                ui.painter().rect_filled(bar, 1.0, p.accent_emphasis);
            } else if resp.hovered() {
                // The rail sits on canvas_subtle, so hover needs a stronger tint.
                ui.painter().rect_filled(rect, 6.0, p.border.gamma_multiply(0.45));
            }
            let (icon, color, _) = pr_icon(&pr.state, pr.is_draft, p);
            icons::paint(ui.painter(), rect.shrink(10.0), icon, color);
            // A small dot shows merge readiness at a glance.
            // The dot is merge readiness (not CI), same color as the list's chip.
            let detail = fresher_detail(app, pr);
            let status = row_status(p, pr, detail.as_deref()).filter(|m| m.short != "Checking" && m.short != "Draft");
            if let Some(ms) = &status {
                ui.painter().circle(rect.right_bottom() - vec2(8.0, 8.0), 4.0, ms.color, Stroke::new(1.5, p.canvas_subtle));
            }
            let tip = match &status {
                Some(ms) => format!("{}#{} {}\n{}", pr.repository.name_with_owner, pr.number, pr.title, ms.short),
                None => format!("{}#{} {}", pr.repository.name_with_owner, pr.number, pr.title),
            };
            super::tip(&resp, &tip);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                app.actions.push(Action::Select(pr.clone()));
            }
        }
        // More pages: "+N" loads the next one.
        if let Some(total) = more {
            let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Load more"));
            if resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, p.border.gamma_multiply(0.45));
            }
            if busy {
                egui::Spinner::new().size(16.0).paint_at(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
            } else {
                let left = total.saturating_sub(rows.len() as u64);
                let label = if left > 99 { "+99".to_string() } else { format!("+{left}") };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, theme::bold(12.0), p.fg_muted);
            }
            super::tip(&resp, &format!("Showing {} of {total} · Load more", rows.len()));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.actions.push(Action::LoadMore);
            }
        }
        });
    });
}

/// GitHub's joined filter buttons: Created | Assigned | ... They never
/// wrap: on a narrow list they tighten, then the rest go under "More ▾".
fn subnav(app: &mut App, ui: &mut Ui, p: &Palette) {
    let avail = ui.available_width();
    let widths = |pad: f32| -> Vec<f32> {
        SECTIONS.iter().map(|s| ui.painter().layout_no_wrap(s.title.to_string(), theme::bold(13.0), p.fg).size().x + pad).collect()
    };
    let mut pad = 24.0;
    let mut w = widths(pad);
    if w.iter().sum::<f32>() > avail {
        pad = 14.0;
        w = widths(pad);
    }
    // How many fit, keeping room for "More ▾" if some don't.
    let more_w = ui.painter().layout_no_wrap("More ▾".into(), theme::bold(13.0), p.fg).size().x + pad;
    let mut shown = SECTIONS.len();
    while shown > 1 && w[..shown].iter().sum::<f32>() + if shown < SECTIONS.len() { more_w } else { 0.0 } > avail {
        shown -= 1;
    }
    let active = match app.view {
        View::Section(i) => Some(i),
        View::Search(_) => None,
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let n = if shown < SECTIONS.len() { shown + 1 } else { shown };
        let seg = |ui: &mut Ui, i: usize, text: &str, width: f32, on: bool| -> egui::Response {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
            let radius = CornerRadius {
                nw: if i == 0 { 6 } else { 0 },
                sw: if i == 0 { 6 } else { 0 },
                ne: if i == n - 1 { 6 } else { 0 },
                se: if i == n - 1 { 6 } else { 0 },
            };
            let fill = if on { p.accent_emphasis } else if resp.hovered() { p.hover_row } else { p.canvas };
            ui.painter().rect(rect, radius, fill, Stroke::new(1.0, if on { p.accent_emphasis } else { p.border }), StrokeKind::Inside);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, theme::bold(13.0), if on { Color32::WHITE } else { p.fg });
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, text));
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        };
        for i in 0..shown {
            if seg(ui, i, SECTIONS[i].title, w[i], active == Some(i)).clicked() {
                app.actions.push(Action::SetView(View::Section(i)));
            }
        }
        if shown < SECTIONS.len() {
            // The active section stays visible: it names the "More" button.
            let hidden_active = active.filter(|&i| i >= shown);
            let label = hidden_active.map(|i| format!("{} ▾", SECTIONS[i].title)).unwrap_or_else(|| "More ▾".into());
            let width = ui.painter().layout_no_wrap(label.clone(), theme::bold(13.0), p.fg).size().x + pad;
            let resp = seg(ui, shown, &label, width, hidden_active.is_some());
            egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_END).show(|ui| {
                ui.set_min_width(180.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in shown..SECTIONS.len() {
                    let on = active == Some(i);
                    let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                    row.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, SECTIONS[i].title));
                    if row.hovered() {
                        ui.painter().rect_filled(r.shrink2(vec2(4.0, 0.0)), 6.0, p.hover_row);
                    }
                    if on {
                        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), vec2(14.0, 14.0)), Icon::Check, p.fg);
                    }
                    let font = if on { theme::bold(14.0) } else { theme::body(14.0) };
                    ui.painter().text(pos2(r.left() + 36.0, r.center().y), egui::Align2::LEFT_CENTER, SECTIONS[i].title, font, p.fg);
                    if row.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        app.actions.push(Action::SetView(View::Section(i)));
                        ui.close();
                    }
                }
            });
        }
    });
}

/// The search box holds the real query (like github.com), so you can edit
/// the scope, not just add to it. Enter runs it; Esc puts it back.
fn search(app: &mut App, ui: &mut Ui, p: &Palette) {
    let id = egui::Id::new("search");
    let focused = ui.memory(|m| m.has_focus(id));
    if !focused {
        app.search_text = app.display_query();
    }
    let stroke = if focused { Stroke::new(2.0, p.accent_emphasis) } else { Stroke::new(1.0, p.border) };
    egui::Frame::new()
        .fill(p.canvas)
        .stroke(stroke)
        .corner_radius(6)
        .inner_margin(Margin { left: 10, right: 10, top: if focused { 3 } else { 4 }, bottom: if focused { 3 } else { 4 } })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Search, 16.0, p.fg_muted);
                // A custom search gets a × that goes back to your PRs.
                let custom = matches!(app.view, View::Search(_));
                let clearable = custom || app.search_text.trim() != app.display_query().trim();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.search_text)
                        .id(id)
                        .hint_text(RichText::new("Search all pull requests").color(p.fg_muted))
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width() - if clearable { 28.0 + ui.spacing().item_spacing.x } else { 0.0 })
                        .font(theme::body(14.0))
                        .margin(vec2(4.0, 4.0)),
                );
                if app.focus_search {
                    resp.request_focus();
                    app.focus_search = false;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let q = app.search_text.clone();
                    app.actions.push(Action::RunSearch(q));
                }
                if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.search_text = app.display_query();
                    resp.surrender_focus();
                }
                if clearable {
                    // A 28px target around a 20px button: easy to hit.
                    let (hit, x) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    let r = Rect::from_center_size(hit.center(), vec2(20.0, 20.0));
                    x.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Clear search"));
                    if x.hovered() {
                        ui.painter().rect_filled(r, 4.0, p.hover_row);
                    }
                    icons::paint(ui.painter(), r.shrink(3.0), Icon::X, if x.hovered() { p.fg } else { p.fg_muted });
                    if x.tip("Clear search").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        if custom {
                            app.actions.push(Action::RunSearch(String::new()));
                        } else {
                            // Only typed, not run: just put the tab's query back.
                            app.search_text = app.display_query();
                            ui.memory_mut(|m| m.surrender_focus(id));
                        }
                    }
                }
            });
        });
}

/// A PR's details, if they're at least as new as the list's own data.
/// Whichever was fetched last says what the row shows, so it can't flip
/// to older news when you select it.
fn fresher_detail(app: &App, pr: &PrSummary) -> Option<std::sync::Arc<PrDetail>> {
    let entry = app.details.get(&pr.id)?;
    let list_at = app.current_list().and_then(|l| l.fetched);
    match (entry.fetched, list_at) {
        (Some(d), Some(l)) if d < l => None,
        (None, _) => None,
        _ => entry.data.clone(),
    }
}

/// Merge status for a list row: from the PR's details when loaded (more
/// accurate), otherwise from the list's own data.
fn row_status(p: &Palette, pr: &PrSummary, detail: Option<&PrDetail>) -> Option<super::MergeStatus> {
    match detail {
        Some(d) => super::detail_merge_status(p, d, pr.merge_state_status.as_deref()),
        None => super::merge_status(p, &pr.state, pr.is_draft, pr.merge_state_status.as_deref(), None, pr.auto_merge, String::new()),
    }
}

/// "19 hours ago" that never wraps in the middle.
fn nb(s: &str) -> String {
    s.replace(' ', "\u{a0}")
}

/// One PR, laid out like github.com/pulls: state icon, "owner/repo Title",
/// then CI status and labels flowing after the title, a meta line below,
/// and the author and comment count at top right.
fn row(ui: &mut Ui, p: &Palette, pr: &PrSummary, detail: Option<&PrDetail>, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let left = 16.0 + 16.0 + 8.0;
    let comments = pr.comments.total_count;
    // The right column is only as wide as what's in it: the comment count
    // and the assignees' faces. The title gets the rest.
    let who = &pr.assignees.nodes;
    let comment_w = if comments > 0 { 44.0 } else { 0.0 };
    let faces_w = if who.is_empty() { 0.0 } else { 20.0 + (who.len() - 1) as f32 * 12.0 + if comments > 0 { 8.0 } else { 0.0 } };
    let right = 16.0 + comment_w + faces_w + if comment_w + faces_w > 0.0 { 12.0 } else { 0.0 };
    let text_w = (width - left - right).max(80.0);

    // "owner/repo Title": one run of text, so it wraps naturally. The
    // repo's hyphens can't break (U+2011), so "ZR-Private" stays whole.
    let repo_name = pr.repository.name_with_owner.replace('-', "\u{2011}");
    let ci = match detail {
        Some(d) if !d.checks.is_empty() => Some(super::CheckSummary::of(&d.checks).state()),
        _ => pr.ci(),
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = text_w;
    // A real space after the repo, so a long title wraps there and not at
    // its punctuation.
    // A narrow list puts "owner/repo" on its own smaller line above the
    // title, so it never breaks mid-name.
    // Decided by width, so every row in a list looks the same.
    if text_w < 260.0 {
        job.append(&format!("{repo_name}\n"), 0.0, egui::TextFormat { font_id: theme::bold(12.0), color: p.fg_muted, ..Default::default() });
    } else {
        job.append(&format!("{repo_name} "), 0.0, egui::TextFormat { font_id: theme::bold(14.0), color: p.fg_muted, ..Default::default() });
    }
    super::append_title(&mut job, &pr.title, 14.0, true, 0.0, p);
    // Room for the CI icon, glued to the last word with no-break spaces so
    // it always sits right after the title.
    const CI_GAP: &str = "\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}";
    if ci.is_some() {
        job.append(CI_GAP, 0.0, egui::TextFormat { font_id: theme::bold(14.0), color: Color32::TRANSPARENT, ..Default::default() });
    }
    let title = ui.painter().layout_job(job);
    // Where the gap landed: the last glyphs of the last row.
    let ci_at = ci.and_then(|_| {
        let row = title.rows.last()?;
        let g = row.glyphs.last()?;
        Some((row.pos + vec2(g.pos.x + g.advance_width - 16.0, (row.rect().height() - 16.0) / 2.0)).to_vec2())
    });

    // Labels flow after the title's last line.
    struct Item {
        at: egui::Vec2,
        size: egui::Vec2,
        pill: Option<(std::sync::Arc<egui::Galley>, Color32, Color32, Color32)>,
    }
    let mut items = Vec::new();
    let title_top = 0.0;
    let last = title.rows.last().map(|r| r.rect()).unwrap_or(Rect::ZERO);
    let (mut cx, mut cy) = (last.right() + 6.0, title_top + last.top());
    let line_h = last.height().max(20.0);
    let place = |size: egui::Vec2, cx: &mut f32, cy: &mut f32| {
        if *cx + size.x > text_w && *cx > 0.0 {
            *cx = 0.0;
            *cy += line_h + 2.0;
        }
        let at = vec2(*cx, *cy + (line_h - size.y) / 2.0);
        *cx += size.x + 4.0;
        at
    };
    if let Some(at) = ci_at {
        items.push(Item { at, size: vec2(16.0, 16.0), pill: None });
    }
    // A line or two of labels at most; the rest become "+N".
    let first_cy = cy;
    let mut more: Option<(egui::Vec2, egui::Vec2, String)> = None;
    let labels = &pr.labels.nodes;
    for (i, l) in labels.iter().enumerate() {
        let (bg, fg, border) = theme::label_colors(&l.color, p);
        let g = ui.painter().layout_no_wrap(l.name.clone(), theme::bold(12.0), fg);
        let size = vec2(g.size().x + 14.0, 20.0);
        let rest = labels.len() - i - 1;
        // Room for this label, plus a "+N" after it if more are coming.
        let more_w = if rest > 0 { 40.0 } else { 0.0 };
        let wraps = cx + size.x + more_w > text_w && cx > 0.0;
        // Wide lists get two extra lines, narrow ones one.
        let extra_lines = if text_w > 300.0 { 2.0 } else { 1.0 };
        if wraps && cy > first_cy + (line_h + 2.0) * (extra_lines - 1.0) + 0.5 {
            let (bg, fg, border) = (Color32::TRANSPARENT, p.fg_muted, p.border);
            let g = ui.painter().layout_no_wrap(format!("+{}", labels.len() - i), theme::bold(12.0), fg);
            let size = vec2(g.size().x + 14.0, 20.0);
            let at = place(size, &mut cx, &mut cy);
            items.push(Item { at, size, pill: Some((g, bg, fg, border)) });
            more = Some((at, size, labels[i..].iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")));
            break;
        }
        let at = place(size, &mut cx, &mut cy);
        items.push(Item { at, size, pill: Some((g, bg, fg, border)) });
    }
    let flow_bottom = items.iter().map(|i| i.at.y + i.size.y).fold(title_top + title.size().y, f32::max);

    // "#123 opened 2 days ago by octocat", or "was merged" / "was closed".
    let who = pr.author.as_ref().map(|a| a.login.as_str()).unwrap_or("ghost");
    // "by octocat" never splits across lines.
    let who = &who.to_string();
    // One unbreakable run, so it only wraps before a "•" chip.
    let meta = nb(&match pr.state.as_str() {
        "MERGED" => format!("#{} by {who} was merged {}", pr.number, util::ago(pr.merged_at.as_deref().unwrap_or(&pr.updated_at))),
        "CLOSED" => format!("#{} by {who} was closed {}", pr.number, util::ago(pr.closed_at.as_deref().unwrap_or(&pr.updated_at))),
        _ => format!("#{} opened {} by {who}", pr.number, util::ago(&pr.created_at)),
    });
    let mut meta_job = egui::text::LayoutJob::default();
    meta_job.wrap.max_width = text_w;
    meta_job.append(&meta, 0.0, egui::TextFormat { font_id: theme::body(12.0), color: p.fg_muted, ..Default::default() });
    if pr.is_draft && pr.state == "OPEN" {
        meta_job.append(" •\u{a0}Draft", 0.0, egui::TextFormat { font_id: theme::body(12.0), color: p.fg_muted, ..Default::default() });
    }
    // Review state, in words like github.com's list.
    let review = match pr.review_decision.as_deref() {
        _ if pr.state != "OPEN" || pr.is_draft => None,
        Some("APPROVED") => Some(("Approved", p.open)),
        Some("CHANGES_REQUESTED") => Some(("Changes requested", p.closed)),
        Some("REVIEW_REQUIRED") => Some(("Review required", p.fg_muted)),
        _ => None,
    };
    if let Some((text, color)) = review {
        meta_job.append(&format!(" •\u{a0}{}", text.replace(' ', "\u{a0}")), 0.0, egui::TextFormat { font_id: theme::body(12.0), color, ..Default::default() });
    }
    // Merge readiness, e.g. "● Ready to merge", once GitHub has worked it out.
    if let Some(ms) = row_status(p, pr, detail).filter(|m| m.short != "Checking" && m.short != "Draft") {
        // No-break spaces: the chip moves to the next line whole.
        meta_job.append(&format!(" ●\u{a0}{}", ms.short.replace(' ', "\u{a0}").replace('-', "\u{2011}")), 0.0, egui::TextFormat { font_id: theme::bold(12.0), color: ms.color, ..Default::default() });
    }
    let meta_g = ui.painter().layout_job(meta_job);

    let pad = 8.0;
    let h = pad + flow_bottom + 4.0 + meta_g.size().y + pad;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("#{} {}", pr.number, pr.title)));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, p.selected_row);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, p.accent_emphasis);
    } else if resp.hovered() {
        painter.rect_filled(rect, 0.0, p.hover_row);
    }

    let origin = pos2(rect.left() + left, rect.top() + pad);
    let (icon, color, _) = pr_icon(&pr.state, pr.is_draft, p);
    icons::paint(painter, Rect::from_min_size(pos2(rect.left() + 16.0, origin.y + 2.0), vec2(16.0, 16.0)), icon, color);
    painter.galley(origin + vec2(0.0, title_top), title, p.fg);
    for it in items {
        let r = Rect::from_min_size(origin + it.at, it.size);
        match it.pill {
            None => {
                let (icon, color) = status_icon(ci.unwrap_or(""), p);
                icons::paint(painter, r, icon, color);
            }
            Some((g, bg, fg, border)) => {
                painter.rect(r, CornerRadius::same(255), bg, Stroke::new(1.0, border), StrokeKind::Inside);
                painter.galley(pos2(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, fg);
            }
        }
    }
    painter.galley(origin + vec2(0.0, flow_bottom + 4.0), meta_g, p.fg_muted);
    if let Some((at, size, names)) = more {
        let r = Rect::from_min_size(origin + at, size);
        let hover = ui.interact(r, resp.id.with("more-labels"), Sense::hover());
        super::tip(&hover, &names);
    }

    // Top right: author, then comment count, level with the title.
    let slot_left = rect.right() - 16.0 - comment_w;
    if comments > 0 {
        icons::paint(painter, Rect::from_min_size(pos2(slot_left + 4.0, origin.y + 2.0), vec2(16.0, 16.0)), Icon::Comment, p.fg_muted);
        let g = painter.layout_no_wrap(comments.to_string(), theme::bold(12.0), p.fg_muted);
        painter.galley(pos2(slot_left + 24.0, origin.y + 2.0), g, p.fg_muted);
    }
    // Assignees, like github.com (the author is already in the meta line).
    // Overlapping, newest assignee on the right.
    let who = &pr.assignees.nodes;
    let faces_right = slot_left - if comments > 0 { 8.0 } else { 0.0 };
    for (i, a) in who.iter().enumerate().rev() {
        let left = faces_right - 20.0 - (who.len() - 1 - i) as f32 * 12.0;
        let r = Rect::from_min_size(pos2(left, origin.y), vec2(20.0, 20.0));
        if who.len() > 1 {
            // A ring in the row's color separates overlapping faces.
            let bg = if selected { p.selected_row } else if resp.hovered() { p.hover_row } else { p.canvas };
            ui.painter().circle_filled(r.center(), 11.5, bg);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        avatar(&mut child, &a.avatar_url, 20.0);
    }
    if !who.is_empty() {
        let span = Rect::from_min_max(pos2(faces_right - 20.0 - (who.len() - 1) as f32 * 12.0, origin.y), pos2(faces_right, origin.y + 20.0));
        let hover = ui.interact(span, resp.id.with("assignees"), Sense::hover());
        let names: Vec<&str> = who.iter().map(|a| a.login.as_str()).collect();
        super::tip(&hover, &format!("Assigned to {}", names.join(", ")));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
