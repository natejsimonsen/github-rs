//! "Files changed" view. Diffs can have tens of thousands of lines, so we
//! compute every row's position once, then draw only the rows on screen.

use crate::views::Tip;
use crate::app::{Action, App, Panel};
use crate::github::FileDiff;
use crate::icons::{self, Icon};
use crate::theme::{self, Palette};
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;

const HEADER_H: f32 = 44.0;
const LINE_H: f32 = 20.0;
const NOTE_H: f32 = 40.0;
const GAP_H: f32 = 16.0;
const FOOT_H: f32 = 7.0;
const NUM_W: f32 = 52.0;
const MAX_LINES: usize = 60_000;
/// Lines shown per click on an expander, like GitHub.
pub const EXPAND: u32 = 20;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ctx,
    Add,
    Del,
    Hunk,
    Meta,
}

enum Row {
    Header(usize, bool),
    Line { kind: Kind, old: Option<u32>, new: Option<u32>, text: String },
    Note(String),
    /// The rounded bottom edge of a file's box. The file's sideways
    /// scrollbar lives here.
    Foot,
    Gap,
    /// A hunk header (or the end of the file) with hidden lines next to it.
    /// Clicking shows more. `hidden` is None when we don't know yet.
    Expander { file: usize, idx: usize, hidden: Option<u32>, kind: Expand, text: String },
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Header(..) => HEADER_H,
            Row::Line { .. } => LINE_H,
            Row::Note(_) => NOTE_H,
            Row::Gap => GAP_H,
            Row::Foot => FOOT_H,
            Row::Expander { kind: Expand::UpDown, .. } => LINE_H * 2.0,
            Row::Expander { .. } => LINE_H,
        }
    }
}

pub struct Layout {
    rows: Vec<Row>,
    tops: Vec<f32>,
    total: f32,
    files: Arc<Vec<FileDiff>>,
    /// Top of each file's header, for the file list on the left.
    file_tops: Vec<f32>,
    /// Bottom of each file's box (where its last line ends).
    file_bottoms: Vec<f32>,
    /// Which file each row belongs to.
    row_file: Vec<usize>,
    /// Longest line in each file, in characters: each file scrolls sideways
    /// on its own, like GitHub.
    file_chars: Vec<usize>,
    /// Syntax colors for each code row: that file's highlighted lines, and
    /// which of them this row is.
    colors: Vec<Option<(crate::syntax::Lines, usize)>>,
    /// The changed part of each changed line that has a partner line on
    /// the other side, as a byte range.
    changes: Vec<Option<std::ops::Range<usize>>>,
}

/// Pairs each block of deleted lines with the added lines right after it,
/// line by line, and marks what changed between each pair: everything but
/// the shared start and end, widened to whole words.
fn word_changes(rows: &[Row]) -> Vec<Option<std::ops::Range<usize>>> {
    let mut out = vec![None; rows.len()];
    let kind = |r: &Row| match r {
        Row::Line { kind, .. } => Some(*kind),
        _ => None,
    };
    let text = |r: &Row| match r {
        Row::Line { text, .. } => text.clone(),
        _ => String::new(),
    };
    let mut i = 0;
    while i < rows.len() {
        if kind(&rows[i]) != Some(Kind::Del) {
            i += 1;
            continue;
        }
        let dels = i;
        while i < rows.len() && kind(&rows[i]) == Some(Kind::Del) {
            i += 1;
        }
        let adds = i;
        while i < rows.len() && kind(&rows[i]) == Some(Kind::Add) {
            i += 1;
        }
        for n in 0..(adds - dels).min(i - adds) {
            let (a, b) = (text(&rows[dels + n]), text(&rows[adds + n]));
            if let Some((ra, rb)) = changed_span(&a, &b) {
                out[dels + n] = Some(ra);
                out[adds + n] = Some(rb);
            }
        }
    }
    out
}

/// The changed middles of two lines, or None when they share too little
/// to be worth marking (or nothing changed).
fn changed_span(a: &str, b: &str) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    // Indentation isn't content: it's left out of the comparison and never
    // marked, so a re-indented block doesn't pair up unrelated lines.
    let (ia, ib) = (a.len() - a.trim_start().len(), b.len() - b.trim_start().len());
    let (a, b) = (&a[ia..], &b[ib..]);
    if a == b {
        return None;
    }
    let pre = a.char_indices().zip(b.chars()).find(|((_, x), y)| x != y).map(|((i, _), _)| i).unwrap_or(a.len().min(b.len()));
    let max_suf = a.len().min(b.len()) - pre;
    let mut suf = 0;
    for (x, y) in a[pre..].chars().rev().zip(b[pre..].chars().rev()) {
        if x != y || suf + x.len_utf8() > max_suf {
            break;
        }
        suf += x.len_utf8();
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // Widen both ends to whole words, the same amount on each side.
    let mut start = pre;
    while start > 0 && a[..start].chars().next_back().is_some_and(word) && a[start..].chars().next().is_some_and(word) {
        start -= a[..start].chars().next_back().map_or(1, char::len_utf8);
    }
    while suf > 0 && a[a.len() - suf..].chars().next().is_some_and(word) && a[..a.len() - suf].chars().next_back().is_some_and(word) {
        suf -= a[a.len() - suf..].chars().next().map_or(1, char::len_utf8);
    }
    let (ea, eb) = (a.len() - suf, b.len() - suf);
    let shared = start + suf;
    let longest = a.len().max(b.len());
    // Mostly rewritten lines, or ones that only share punctuation: GitHub
    // leaves those plain.
    let shares_word = a[..start].chars().chain(a[ea..].chars()).any(word);
    if !shares_word || shared * 3 < longest {
        return None;
    }
    Some((ia + start..ia + ea.max(start), ib + start..ib + eb.max(start)))
}

/// Highlights each hunk twice, once as the new side (context and added
/// lines) and once as the old side (context and deleted lines), so the
/// parser always sees real code, never both versions of a line mixed.
fn highlight_rows(rows: &[Row], row_file: &[usize], files: &[FileDiff]) -> Vec<Option<(crate::syntax::Lines, usize)>> {
    let mut out = vec![None; rows.len()];
    let mut i = 0;
    while i < rows.len() {
        // One run of code lines: a hunk, until the next header/expander/file.
        let is_code = |r: &Row| matches!(r, Row::Line { kind: Kind::Add | Kind::Del | Kind::Ctx, .. });
        if !is_code(&rows[i]) {
            i += 1;
            continue;
        }
        let fi = row_file[i];
        let mut j = i;
        while j < rows.len() && row_file[j] == fi && is_code(&rows[j]) {
            j += 1;
        }
        if let Some(lang) = files.get(fi).and_then(|f| crate::syntax::lang_for_diff(&f.filename, f.patch.as_deref().unwrap_or(""))) {
            for side in [Kind::Add, Kind::Del] {
                let idx: Vec<usize> = (i..j)
                    .filter(|&k| matches!(&rows[k], Row::Line { kind, .. } if *kind == side || *kind == Kind::Ctx))
                    .collect();
                let texts: Vec<&str> = idx
                    .iter()
                    .map(|&k| match &rows[k] {
                        Row::Line { text, .. } => text.as_str(),
                        _ => "",
                    })
                    .collect();
                if let Some(lines) = crate::syntax::highlight_fragment(lang, &texts) {
                    for (n, &k) in idx.iter().enumerate() {
                        // Context lines take the new side's colors.
                        let ctx = matches!(rows[k], Row::Line { kind: Kind::Ctx, .. });
                        if side == Kind::Add || !ctx {
                            out[k] = Some((lines.clone(), n));
                        }
                    }
                }
            }
        }
        i = j;
    }
    out
}

/// Which way an expander shows hidden lines, like GitHub's: up from the
/// hunk below, down from the hunk above, both (two buttons, when more than
/// 20 are hidden between hunks), or all at once (20 or fewer).
#[derive(Clone, Copy, PartialEq)]
pub enum Expand {
    Up,
    Down,
    UpDown,
    All,
}

/// Expansion keys: a hunk's index for lines above it, this plus the index
/// for lines below the hunk before it.
pub const DOWN_KEY: usize = 100_000;

/// Extra lines shown around a file's hunks: the file's full text (once
/// fetched) and how many lines each expander has revealed.
pub type Expansion = (Option<Arc<Vec<String>>>, HashMap<usize, u32>);

pub fn build(files: Arc<Vec<FileDiff>>, is_collapsed: impl Fn(&FileDiff) -> bool, expansion: impl Fn(&FileDiff) -> Expansion) -> Layout {
    let mut rows = Vec::new();
    let mut row_file = Vec::new();
    let mut file_chars = vec![0usize; files.len()];
    let mut lines = 0usize;
    for (i, f) in files.iter().enumerate() {
        let folded = is_collapsed(f);
        rows.push(Row::Header(i, folded));
        if folded {
            rows.push(Row::Gap);
            row_file.resize(rows.len(), i);
            continue;
        }
        match &f.patch {
            _ if lines > MAX_LINES => rows.push(Row::Note("Diff not shown because this pull request is very large. Open it on GitHub.".into())),
            None if f.status == "renamed" && f.additions + f.deletions == 0 => rows.push(Row::Note("File renamed without changes.".into())),
            None => rows.push(Row::Note("Binary file, or diff too large to show here.".into())),
            Some(patch) => {
                let (full, shown) = expansion(f);
                let can_expand = f.status != "removed" && f.status != "added";
                // A context line from the full file; `n` is 1-based.
                let ctx_line = |n: u32, offset: i64, file_chars: &mut Vec<usize>| {
                    let text = full.as_ref().and_then(|t| t.get(n as usize - 1)).map(|l| l.replace('\t', "    ")).unwrap_or_default();
                    file_chars[i] = file_chars[i].max(text.chars().count() + 1);
                    Row::Line { kind: Kind::Ctx, old: Some((n as i64 - offset) as u32), new: Some(n), text }
                };
                let (mut old, mut new) = (0u32, 0u32);
                let mut hunk = 0usize;
                // Last new-file line shown so far.
                let mut last_new = 0u32;
                for raw in patch.lines() {
                    lines += 1;
                    let text = raw.replace('\t', "    ");
                    // Hunk headers are cut with "…", so they never need scrolling.
                    if !raw.starts_with("@@") {
                        file_chars[i] = file_chars[i].max(text.chars().count());
                    }
                    let row = if raw.starts_with("@@") {
                        (old, new) = hunk_start(raw);
                        let gap = new.saturating_sub(last_new + 1);
                        let get = |k: usize| if full.is_some() { shown.get(&k).copied().unwrap_or(0) } else { 0 };
                        // Lines shown down from the hunk above, then up from this one.
                        let down = if hunk > 0 { get(DOWN_KEY + hunk).min(gap) } else { 0 };
                        let up = get(hunk).min(gap - down);
                        let hidden = gap - down - up;
                        let hunk_offset = new as i64 - old as i64;
                        for n in last_new + 1..=last_new + down {
                            rows.push(ctx_line(n, hunk_offset, &mut file_chars));
                        }
                        // The header covers the lines shown above it, like GitHub.
                        let text = if up > 0 { shift_hunk_header(&text, up) } else { text };
                        if can_expand && hidden > 0 {
                            let kind = if hidden <= EXPAND {
                                Expand::All
                            } else if hunk > 0 {
                                Expand::UpDown
                            } else {
                                Expand::Up
                            };
                            rows.push(Row::Expander { file: i, idx: hunk, hidden: Some(hidden), kind, text });
                        } else if !(gap > 0 && hidden == 0) {
                            // Fully expanded up to the hunk above: GitHub drops the header.
                            rows.push(Row::Line { kind: Kind::Hunk, old: None, new: None, text });
                        }
                        for n in new - up..new {
                            rows.push(ctx_line(n, hunk_offset, &mut file_chars));
                        }
                        hunk += 1;
                        last_new = new.saturating_sub(1);
                        continue;
                    } else if let Some(t) = text.strip_prefix('+') {
                        new += 1;
                        Row::Line { kind: Kind::Add, old: None, new: Some(new - 1), text: t.to_string() }
                    } else if let Some(t) = text.strip_prefix('-') {
                        old += 1;
                        Row::Line { kind: Kind::Del, old: Some(old - 1), new: None, text: t.to_string() }
                    } else if raw.starts_with('\\') {
                        Row::Line { kind: Kind::Meta, old: None, new: None, text }
                    } else {
                        old += 1;
                        new += 1;
                        let t = text.strip_prefix(' ').unwrap_or(&text).to_string();
                        Row::Line { kind: Kind::Ctx, old: Some(old - 1), new: Some(new - 1), text: t }
                    };
                    if let Row::Line { new: Some(n), .. } = &row {
                        last_new = *n;
                    }
                    rows.push(row);
                }
                // Old-minus-new shift after the last hunk's own adds and deletes.
                let offset = new as i64 - old as i64;
                // Lines after the last hunk.
                if can_expand && hunk > 0 {
                    let total = full.as_ref().map(|t| t.len() as u32);
                    let after = total.map(|t| t.saturating_sub(last_new));
                    let reveal = shown.get(&hunk).copied().unwrap_or(0).min(after.unwrap_or(0));
                    for n in last_new + 1..=last_new + reveal {
                        rows.push(ctx_line(n, offset, &mut file_chars));
                    }
                    match after {
                        Some(a) if a <= reveal => {}
                        _ => rows.push(Row::Expander { file: i, idx: hunk, hidden: after.map(|a| a - reveal), kind: Expand::Down, text: String::new() }),
                    }
                }
            }
        }
        rows.push(Row::Foot);
        rows.push(Row::Gap);
        row_file.resize(rows.len(), i);
    }
    let mut tops = Vec::with_capacity(rows.len());
    let mut file_tops = Vec::new();
    let mut file_bottoms = Vec::new();
    let mut y = 0.0;
    for r in &rows {
        match r {
            Row::Header(..) => file_tops.push(y),
            Row::Gap => file_bottoms.push(y),
            _ => {}
        }
        tops.push(y);
        y += r.height();
    }
    let colors = highlight_rows(&rows, &row_file, &files);
    let changes = word_changes(&rows);
    Layout { rows, tops, total: y, files, file_tops, file_bottoms, row_file, file_chars, colors, changes }
}

/// "@@ -12,7 +12,9 @@ fn x" with 3 more lines above -> "@@ -9,10 +9,12 @@ fn x".
fn shift_hunk_header(line: &str, up: u32) -> String {
    let Some(rest) = line.strip_prefix("@@ ") else { return line.to_string() };
    let Some((ranges, tail)) = rest.split_once(" @@") else { return line.to_string() };
    let range = |r: &str| -> String {
        let (start, count) = r[1..].split_once(',').unwrap_or((&r[1..], "1"));
        let (start, count): (u32, u32) = (start.parse().unwrap_or(1), count.parse().unwrap_or(1));
        format!("{}{},{}", &r[..1], start.saturating_sub(up), count + up)
    };
    let shifted: Vec<String> = ranges.split(' ').map(range).collect();
    format!("@@ {} @@{tail}", shifted.join(" "))
}

/// "@@ -12,7 +12,9 @@" -> (12, 12)
fn hunk_start(line: &str) -> (u32, u32) {
    let num = |prefix: char| -> u32 {
        line.split_whitespace()
            .find(|p| p.starts_with(prefix))
            .and_then(|p| p[1..].split(',').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(1)
    };
    (num('-'), num('+'))
}

/// Where file links point: the repo and the PR's head commit.
pub struct Source<'a> {
    pub repo: &'a str,
    pub head: &'a str,
}

pub fn show(app: &mut App, ui: &mut Ui, pr_id: &str, files: Arc<Vec<FileDiff>>, src: Source<'_>) {
    let p = theme::palette(ui.ctx());
    let layout = match app.diff_layouts.get(pr_id) {
        Some(l) => l.clone(),
        None => {
            let l = Arc::new(build(files.clone(), |f| app.is_collapsed(pr_id, &f.filename), |f| app.expansion(pr_id, &f.filename)));
            app.diff_layouts.insert(pr_id.to_string(), l.clone());
            l
        }
    };

    let jump_id = egui::Id::new(("diff-jump", pr_id));
    let current_id = egui::Id::new(("diff-current", pr_id));
    let mut toggle = false;
    // File tree on the left. Closed, it's gone entirely; the button to bring
    // it back sits next to the summary line.
    // Like GitHub, the tree folds away when the page is narrow.
    let narrow = ui.available_width() < 900.0;
    app.tree_narrow = narrow;
    let tree_open = if narrow { app.tree_open_narrow } else { app.panels.tree };
    if tree_open && narrow {
        // No room to dock it: float the tree over the diff instead of
        // squeezing the code. A click outside, Esc, or picking a file closes it.
        let top_left = ui.available_rect_before_wrap().min;
        let width = ui.available_width();
        let height = ui.available_height().max(200.0);
        let area = egui::Area::new(egui::Id::new(("tree-overlay", pr_id)))
            .order(egui::Order::Foreground)
            .fixed_pos(top_left)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(p.overlay)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::same(12))
                    .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if p.dark { 160 } else { 50 }) })
                    .show(ui, |ui| {
                        ui.set_width(300.0f32.min(width * 0.45));
                        ui.set_height(height - 24.0);
                        tree_header(ui, p, &layout, &mut toggle);
                        let current = ui.ctx().data(|d| d.get_temp::<usize>(current_id));
                        file_tree(ui, p, &layout, jump_id, pr_id, current);
                    });
            });
        let picked = ui.ctx().data(|d| d.get_temp::<f32>(jump_id)).is_some();
        let clicked_outside = ui.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|pos| !area.response.rect.contains(pos)));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if picked || clicked_outside || escape {
            app.tree_open_narrow = false;
        }
    } else if tree_open {
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(280.0)
            .size_range(200.0..=480.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 16, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                tree_header(ui, p, &layout, &mut toggle);
                let current = ui.ctx().data(|d| d.get_temp::<usize>(current_id));
                file_tree(ui, p, &layout, jump_id, pr_id, current);
            });
    }
    ui.vertical(|ui| diff_rows(app, ui, p, &layout, jump_id, current_id, &src, &mut toggle));
    if toggle {
        app.actions.push(Action::TogglePanel(Panel::Tree));
    }
}

/// Puts files in the tree's order, so the diff reads top to bottom like it.
pub fn sort_like_tree(files: &mut [FileDiff]) {
    files.sort_by(|a, b| tree_order(&a.filename, &b.filename));
}

/// Path order in the file tree: folder by folder, folders before files.
fn tree_order(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.split('/').peekable(), b.split('/').peekable());
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) => {
                let (x_dir, y_dir) = (a.peek().is_some(), b.peek().is_some());
                if x_dir != y_dir {
                    return y_dir.cmp(&x_dir);
                }
                if x != y {
                    return x.cmp(y);
                }
            }
            (x, y) => return x.is_some().cmp(&y.is_some()),
        }
    }
}

#[cfg(test)]
#[test]
fn folders_come_first() {
    let mut v = vec!["CODEOWNERS", "cicd/b.go", "cicd/x/a.go", "a.txt"];
    v.sort_by(|a, b| tree_order(a, b));
    assert_eq!(v, ["cicd/x/a.go", "cicd/b.go", "CODEOWNERS", "a.txt"]);
}

#[cfg(test)]
#[test]
fn shifts_hunk_headers() {
    assert_eq!(shift_hunk_header("@@ -12,7 +12,9 @@ fn x", 3), "@@ -9,10 +9,12 @@ fn x");
    assert_eq!(shift_hunk_header("@@ -0,0 +1 @@", 0), "@@ -0,0 +1,1 @@");
}

#[cfg(test)]
#[test]
fn marks_changed_words() {
    let (a, b) = changed_span("let timeout = 30;", "let timeout = 60;").unwrap();
    assert_eq!((&"let timeout = 30;"[a], &"let timeout = 60;"[b]), ("30", "60"));
    // Part of a word changed: the whole word is marked.
    let (a, b) = changed_span("call(fooBar)", "call(fooBaz)").unwrap();
    assert_eq!((&"call(fooBar)"[a], &"call(fooBaz)"[b]), ("fooBar", "fooBaz"));
    // Only added text: the old side marks nothing.
    let (a, b) = changed_span("f(x)", "f(x, y)").unwrap();
    assert!(a.is_empty());
    assert_eq!(&"f(x, y)"[b], ", y");
    assert!(changed_span("alpha beta gamma", "something else entirely").is_none());
    // Re-indented blocks: indentation alone doesn't make lines a pair.
    assert!(changed_span("        },", "            {name: \"x\"}").is_none());
    assert!(changed_span("    foo(1)", "        foo(1)").is_none());
    let (a, b) = changed_span("    foo(1)", "        foo(2)").unwrap();
    assert_eq!((&"    foo(1)"[a], &"        foo(2)"[b]), ("1", "2"));
}

/// "Files 38" with the button that hides the tree.
fn tree_header(ui: &mut Ui, p: &Palette, layout: &Layout, toggle: &mut bool) {
    ui.horizontal(|ui| {
        *toggle |= crate::views::icon_button_tip_above(ui, Icon::SidebarLeftClose, "Hide file tree (⇧⌘B)", p).clicked();
        ui.label(egui::RichText::new("Files").font(theme::bold(14.0)).color(p.fg));
        ui.label(egui::RichText::new(layout.files.len().to_string()).size(12.0).color(p.fg_muted));
    });
    ui.add_space(8.0);
}

/// A folder in the file tree, with files and subfolders.
#[derive(Default)]
struct Dir {
    dirs: std::collections::BTreeMap<String, Dir>,
    files: Vec<(String, usize)>,
}

impl Dir {
    fn of(files: &[FileDiff], filter: &str) -> Dir {
        let mut root = Dir::default();
        let filter = filter.to_lowercase();
        for (i, f) in files.iter().enumerate() {
            if !filter.is_empty() && !f.filename.to_lowercase().contains(&filter) {
                continue;
            }
            let mut parts: Vec<&str> = f.filename.split('/').collect();
            let name = parts.pop().unwrap_or_default().to_string();
            let mut dir = &mut root;
            for part in parts {
                dir = dir.dirs.entry(part.to_string()).or_default();
            }
            dir.files.push((name, i));
        }
        root.squash();
        root
    }

    /// Joins folders that only hold one folder, like GitHub ("cicd/public/argocd").
    fn squash(&mut self) {
        let names: Vec<String> = self.dirs.keys().cloned().collect();
        for name in names {
            let mut d = self.dirs.remove(&name).unwrap();
            let mut name = name;
            while d.files.is_empty() && d.dirs.len() == 1 {
                let (child, inner) = d.dirs.into_iter().next().unwrap();
                name = format!("{name}/{child}");
                d = inner;
            }
            d.squash();
            self.dirs.insert(name, d);
        }
    }
}

fn file_tree(ui: &mut Ui, p: &Palette, layout: &Layout, jump_id: egui::Id, pr_id: &str, current: Option<usize>) {
    let filter_id = egui::Id::new(("tree-filter", pr_id));
    let mut filter = ui.ctx().data(|d| d.get_temp::<String>(filter_id)).unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(filter_id));
    ui.set_max_width(ui.available_width());
    egui::Frame::new()
        .fill(p.canvas)
        .stroke(if focused { Stroke::new(2.0, p.accent_emphasis) } else { Stroke::new(1.0, p.border) })
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, if focused { 1 } else { 2 }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Search, 14.0, p.fg_muted);
                let clear = !filter.is_empty();
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .id(filter_id)
                        .hint_text(egui::RichText::new("Filter files…").color(p.fg_muted))
                        .frame(egui::Frame::NONE)
                        // Exactly what's left, so the panel never grows to fit it.
                        .desired_width((ui.available_width() - if clear { 18.0 + ui.spacing().item_spacing.x } else { 0.0 }).max(20.0))
                        .font(theme::body(13.0)),
                );
                if clear {
                    let (r, x) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    x.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Clear filter"));
                    icons::paint(ui.painter(), r.shrink(3.0), Icon::X, if x.hovered() { p.fg } else { p.fg_muted });
                    if crate::views::Tip::tip(x, "Clear filter").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        filter.clear();
                    }
                }
            });
        });
    ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    ui.add_space(8.0);
    let tree = Dir::of(&layout.files, &filter);
    if !filter.trim().is_empty() && !(tree.dirs.is_empty() && tree.files.is_empty()) {
        let n = layout.files.iter().filter(|f| f.filename.to_lowercase().contains(&filter.to_lowercase())).count();
        ui.label(egui::RichText::new(format!("{n} of {} files", layout.files.len())).size(12.0).color(p.fg_muted));
        ui.add_space(4.0);
    }
    egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if tree.dirs.is_empty() && tree.files.is_empty() {
            ui.label(egui::RichText::new("No files match.").size(13.0).color(p.fg_muted));
        }
        tree_dir(ui, p, layout, &tree, 0, "", jump_id, pr_id, current);
    });
}

#[allow(clippy::too_many_arguments)]
fn tree_dir(ui: &mut Ui, p: &Palette, layout: &Layout, dir: &Dir, depth: usize, path: &str, jump_id: egui::Id, pr_id: &str, current: Option<usize>) {
    let indent = depth as f32 * 16.0;
    for (name, sub) in &dir.dirs {
        let full = format!("{path}{name}/");
        let open_id = egui::Id::new(("tree-dir", pr_id, &full));
        let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(true);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, p.hover_row);
        }
        let x = rect.left() + 4.0 + indent;
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        icons::paint(ui.painter(), Rect::from_min_size(pos2(x, rect.center().y - 6.0), vec2(12.0, 12.0)), chevron, p.fg_muted);
        let folder = if open { Icon::FolderOpen } else { Icon::Folder };
        icons::paint(ui.painter(), Rect::from_min_size(pos2(x + 16.0, rect.center().y - 7.0), vec2(14.0, 14.0)), folder, p.accent.gamma_multiply(0.8));
        let label_x = x + 36.0;
        // Folder chains lose their start, so the deepest folder stays readable.
        let shown = crate::views::elide_front(ui.painter(), name, theme::body(13.0), rect.right() - label_x - 4.0);
        let cut = shown != *name;
        ui.painter().text(pos2(label_x, rect.center().y), egui::Align2::LEFT_CENTER, shown, theme::body(13.0), p.fg);
        let resp = if cut { resp.tip(name) } else { resp };
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(open_id, !open));
        }
        if open {
            tree_dir(ui, p, layout, sub, depth + 1, &full, jump_id, pr_id, current);
        }
    }
    for (name, i) in &dir.files {
        let f = &layout.files[*i];
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
        let is_current = current == Some(*i);
        if is_current {
            ui.painter().rect_filled(rect, 6.0, p.border.gamma_multiply(0.35));
            let bar = Rect::from_min_size(pos2(rect.left(), rect.top() + 4.0), vec2(4.0, rect.height() - 8.0));
            ui.painter().rect_filled(bar, 6.0, p.accent_emphasis);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, p.hover_row);
        }
        let x = rect.left() + 4.0 + indent + 16.0;
        let (icon, color) = file_icon(f, p);
        icons::paint(ui.painter(), Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0)), Icon::File, p.fg_muted);
        let name_color = if f.viewed { p.fg_muted } else { p.fg };
        let right = rect.right() - 26.0;
        let cut = elided(ui.painter(), p, pos2(x + 20.0, rect.center().y), right - x - 24.0, name, theme::body(13.0), name_color);
        // Status at the right: added, removed, renamed or changed. Viewed files get a check.
        let status = Rect::from_min_size(pos2(rect.right() - 22.0, rect.center().y - 8.0), vec2(16.0, 16.0));
        if f.viewed {
            icons::paint(ui.painter(), status, Icon::Check, p.fg_muted);
        } else {
            icons::paint(ui.painter(), status, icon, color);
        }
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let resp = if cut { resp.tip(&f.filename) } else { resp };
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(jump_id, layout.file_tops[*i]));
        }
    }
}

/// GitHub's file status icons and colors.
fn file_icon(f: &FileDiff, p: &Palette) -> (Icon, egui::Color32) {
    match f.status.as_str() {
        "added" => (Icon::FileAdded, p.open),
        "removed" => (Icon::FileRemoved, p.closed),
        "renamed" => (Icon::FileMoved, p.fg_muted),
        _ => (Icon::FileModified, p.attention),
    }
}

/// Text cut to `max_w` with "…" at the end.
/// Returns true if it had to cut the text.
fn elided(painter: &egui::Painter, _p: &Palette, left_center: egui::Pos2, max_w: f32, text: &str, font: egui::FontId, color: egui::Color32) -> bool {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let g = painter.layout_job(job);
    let cut = g.elided;
    painter.galley(pos2(left_center.x, left_center.y - g.size().y / 2.0), g, color);
    cut
}

#[allow(clippy::too_many_arguments)]
fn diff_rows(app: &mut App, ui: &mut Ui, p: &Palette, layout: &Layout, jump_id: egui::Id, current_id: egui::Id, src: &Source<'_>, toggle: &mut bool) {
    let files = &layout.files;
    let (adds, dels) = files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    let viewed = files.iter().filter(|f| f.viewed).count();
    ui.horizontal(|ui| {
        // Whenever the tree isn't showing, its button is.
        let shown = if app.tree_narrow { app.tree_open_narrow } else { app.panels.tree };
        if !shown {
            *toggle |= crate::views::icon_button_tip_above(ui, Icon::SidebarLeftOpen, "Show file tree (⇧⌘B)", p).clicked();
        }
        let summary = format!(
            "Showing {} changed file{} with {adds} addition{} and {dels} deletion{}.",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            if adds == 1 { "" } else { "s" },
            if dels == 1 { "" } else { "s" },
        );
        // The summary gives way ("…") before the viewed count does.
        let room = (ui.available_width() - 210.0).max(60.0);
        let mut job = egui::text::LayoutJob::single_section(summary.clone(), egui::TextFormat { font_id: theme::body(13.0), color: p.fg_muted, ..Default::default() });
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = ui.painter().layout_job(job);
        let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
        ui.painter().galley(r.min, g, p.fg_muted);
        resp.tip(&summary);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(14.0);
            // GitHub's progress bar for files viewed.
            let (bar, _) = ui.allocate_exact_size(vec2(60.0, 8.0), Sense::hover());
            ui.painter().rect_filled(bar, 4.0, p.border_muted);
            let frac = if files.is_empty() { 0.0 } else { viewed as f32 / files.len() as f32 };
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), 4.0, p.open);
            ui.label(egui::RichText::new(format!("{viewed} / {} files viewed", files.len())).color(p.fg_muted).size(13.0));
        });
    });
    ui.add_space(8.0);

    let char_w = ui.fonts_mut(|f| f.glyph_width(&theme::mono(12.0), 'M'));
    let mut area = egui::ScrollArea::vertical().id_salt("diff").auto_shrink([false, false]);
    if let Some(y) = ui.ctx().data_mut(|d| d.remove_temp::<f32>(jump_id)) {
        area = area.vertical_scroll_offset(y);
    }
    let pan_id = egui::Id::new(("diff-pan", files.as_ptr()));
    let mut pans: Vec<f32> = ui.ctx().data(|d| d.get_temp::<Vec<f32>>(pan_id)).unwrap_or_default();
    pans.resize(files.len(), 0.0);
    area.show_viewport(ui, |ui, viewport| {
        // Room for the scrollbar, so it never covers the boxes' right edge.
        let width = ui.available_width() - 14.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, layout.total), Sense::hover());
        let origin = rect.min;
        let painter = ui.painter().clone();
        let painter = &painter;
        let code_left = NUM_W * 2.0 + 10.0;
        let code_w = width - code_left - 8.0;
        let file_w = |fi: usize| 16.0 + layout.file_chars[fi] as f32 * char_w + 24.0;

        // Which file is at the top, for the tree's highlight.
        let current = layout.file_tops.partition_point(|&t| t <= viewport.min.y + 1.0).saturating_sub(1);
        ui.ctx().data_mut(|d| d.insert_temp(current_id, current));

        // Sideways scrolling for the file under the pointer.
        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|h| rect.contains(*h));
        let hovered_file = hover.map(|h| {
            let y = h.y - origin.y;
            let row = layout.tops.partition_point(|&t| t <= y).saturating_sub(1);
            layout.row_file.get(row).copied().unwrap_or(0)
        });
        if let Some(fi) = hovered_file {
            let dx = ui.input(|i| i.smooth_scroll_delta.x);
            if dx != 0.0 {
                let max = (file_w(fi) - code_w).max(0.0);
                pans[fi] = (pans[fi] - dx).clamp(0.0, max);
            }
        }

        let first = layout.tops.partition_point(|&t| t + LINE_H < viewport.min.y).saturating_sub(1);
        for i in first..layout.rows.len() {
            let top = layout.tops[i];
            if top > viewport.max.y {
                break;
            }
            let row = &layout.rows[i];
            let fi = layout.row_file[i];
            let r = Rect::from_min_size(pos2(origin.x, origin.y + top), vec2(width, row.height()));
            match row {
                Row::Header(fi, collapsed) => file_header(app, ui, p, &files[*fi], *fi, r, *collapsed, src),
                Row::Line { kind, old, new, text } => {
                    let (bg, num_bg) = match kind {
                        Kind::Add => (p.diff_add, p.diff_add_num),
                        Kind::Del => (p.diff_del, p.diff_del_num),
                        Kind::Hunk => (p.diff_hunk, p.diff_hunk_num),
                        _ => (p.canvas, p.canvas),
                    };
                    painter.rect_filled(r, 0.0, bg);
                    let nums = Rect::from_min_size(r.min, vec2(NUM_W * 2.0, r.height()));
                    painter.rect_filled(nums, 0.0, num_bg);
                    let num_color = if *kind == Kind::Ctx { p.fg_muted.gamma_multiply(0.8) } else { p.fg_muted };
                    for (n, x) in [(old, NUM_W), (new, NUM_W * 2.0)] {
                        if let Some(n) = n {
                            painter.text(pos2(r.left() + x - 10.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), theme::mono(12.0), num_color);
                        }
                    }
                    let marker = match kind {
                        Kind::Add => "+",
                        Kind::Del => "-",
                        _ => "",
                    };
                    let code = Rect::from_min_max(pos2(r.left() + code_left, r.top()), pos2(r.right() - 8.0, r.bottom()));
                    let clipped = painter.with_clip_rect(code.intersect(painter.clip_rect()));
                    let tx = code.left() - pans[fi];
                    clipped.text(pos2(tx, r.center().y), Align2::LEFT_CENTER, marker, theme::mono(12.0), p.fg_muted);
                    let color = match kind {
                        Kind::Hunk | Kind::Meta => p.fg_muted,
                        _ => p.fg,
                    };
                    let runs = layout.colors[i].as_ref().and_then(|(lines, n)| lines.get(*n)).map(Vec::as_slice);
                    let g = painter.layout_job(crate::syntax::job(text, runs, theme::mono(12.0), color, p));
                    let at = pos2(tx + 16.0, r.center().y - g.size().y / 2.0);
                    if let Some(span) = layout.changes[i].as_ref().filter(|s| s.start < s.end) {
                        let x = |b: usize| g.pos_from_cursor(egui::text::CCursor::new(text[..b].chars().count())).left();
                        let word = if *kind == Kind::Add { p.diff_add_word } else { p.diff_del_word };
                        let mark = Rect::from_x_y_ranges(at.x + x(span.start)..=at.x + x(span.end), r.top() + 1.0..=r.bottom() - 1.0);
                        clipped.rect_filled(mark, 3.0, word);
                    }
                    clipped.galley(at, g, color);
                    side_borders(painter, p, r);
                }
                Row::Note(msg) => {
                    painter.rect_filled(r, 0.0, p.canvas);
                    painter.text(r.center(), Align2::CENTER_CENTER, msg, theme::body(13.0), p.fg_muted);
                    side_borders(painter, p, r);
                }
                Row::Expander { file, idx, hidden, kind, text } => {
                    painter.rect_filled(r, 0.0, p.diff_hunk);
                    let gutter = Rect::from_min_size(r.min, vec2(NUM_W * 2.0, r.height()));
                    let path = &files[*file].filename;
                    let lines = |n: u32| format!("{n} line{}", if n == 1 { "" } else { "s" });
                    // One or two buttons in the gutter: (area, arrow, key, amount, label).
                    let mut buttons: Vec<(Rect, Expand, usize, u32, String)> = Vec::new();
                    match (kind, hidden) {
                        (Expand::UpDown, Some(h)) => {
                            let (top, bottom) = gutter.split_top_bottom_at_fraction(0.5);
                            buttons.push((top, Expand::Down, DOWN_KEY + *idx, EXPAND, format!("Expand {} below", lines(EXPAND.min(*h)))));
                            buttons.push((bottom, Expand::Up, *idx, EXPAND, format!("Expand {} above", lines(EXPAND.min(*h)))));
                        }
                        (Expand::All, Some(h)) => buttons.push((gutter, Expand::All, *idx, *h, format!("Expand all {}", lines(*h)))),
                        (Expand::Up, Some(h)) => buttons.push((gutter, Expand::Up, *idx, EXPAND, format!("Expand {} above", lines(EXPAND.min(*h))))),
                        (_, Some(h)) => buttons.push((gutter, Expand::Down, *idx, EXPAND, format!("Expand {} below", lines(EXPAND.min(*h))))),
                        (_, None) => buttons.push((gutter, Expand::Down, *idx, EXPAND, "Expand lines below".into())),
                    }
                    for (n, (area, arrow, key, amount, label)) in buttons.into_iter().enumerate() {
                        let resp = ui.interact(area, egui::Id::new(("expand", fi, *idx, n)), Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                        painter.rect_filled(area, 0.0, if resp.hovered() { p.accent_emphasis } else { p.diff_hunk_num });
                        let color = if resp.hovered() { egui::Color32::WHITE } else { p.fg_muted };
                        unfold_arrow(painter, area.center(), arrow, color);
                        let under_header = i > 0 && matches!(layout.rows[i - 1], Row::Header(..));
                        let resp = if under_header { resp.tip_below(&label) } else { resp.tip_above(&label) };
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            app.actions.push(Action::ExpandHunk(path.clone(), key, amount));
                        }
                    }
                    // With two buttons the text sits on the lower (↑) row, like GitHub.
                    let text_y = if *kind == Expand::UpDown { r.bottom() - LINE_H / 2.0 } else { r.center().y };
                    let code = Rect::from_min_max(pos2(r.left() + code_left, r.top()), pos2(r.right() - 8.0, r.bottom()));
                    // The @@ line stays put and ends in "…" rather than mid-letter.
                    let mut job = egui::text::LayoutJob::single_section(text.clone(), egui::TextFormat { font_id: theme::mono(12.0), color: p.fg_muted, ..Default::default() });
                    job.wrap = egui::text::TextWrapping::truncate_at_width((code.width() - 16.0).max(20.0));
                    let g = painter.layout_job(job);
                    let at = pos2(code.left() + 16.0, text_y - g.size().y / 2.0);
                    painter.with_clip_rect(code.intersect(painter.clip_rect())).galley(at, g, p.fg_muted);
                    side_borders(painter, p, r);
                }
                Row::Foot => {
                    painter.rect_filled(r, CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 }, p.canvas);
                    rounded_bottom(painter, r.shrink2(vec2(0.5, 0.0)), 6.0, Stroke::new(1.0, p.border));
                }
                Row::Gap => {}
            }
        }

        // A thin scrollbar at the bottom of the visible part of each wide file.
        for fi in 0..files.len() {
            let max = (file_w(fi) - code_w).max(0.0);
            if max <= 0.0 || layout.file_bottoms.get(fi).is_none() {
                continue;
            }
            let (top, bottom) = (layout.file_tops[fi] + HEADER_H, layout.file_bottoms[fi]);
            if bottom < viewport.min.y || top > viewport.max.y || bottom - top < 30.0 {
                continue;
            }
            let y = if bottom - FOOT_H <= viewport.max.y { bottom - FOOT_H + 1.0 } else { viewport.max.y - 8.0 };
            let track = Rect::from_min_size(pos2(origin.x + code_left, origin.y + y), vec2(code_w, 5.0));
            let thumb_w = (code_w * code_w / file_w(fi)).max(30.0);
            let thumb_x = track.left() + (track.width() - thumb_w) * (pans[fi] / max);
            let thumb = Rect::from_min_size(pos2(thumb_x, track.top()), vec2(thumb_w, 5.0));
            let resp = ui.interact(track.expand(3.0), egui::Id::new(("hbar", fi)), Sense::drag());
            let strong = resp.hovered() || resp.dragged() || hovered_file == Some(fi);
            // Pinned to the window bottom it sits over code, so give it a band.
            if bottom - FOOT_H > viewport.max.y {
                let card = (origin.x + 1.0)..=(origin.x + ui.max_rect().width() - 1.0);
                painter.rect_filled(Rect::from_x_y_ranges(card, (track.top() - 3.0)..=(track.bottom() + 3.0)), 0.0, p.canvas);
            }
            // Always a faint thumb, so wide files show they scroll.
            let alpha = if resp.dragged() { 0.7 } else if strong { 0.45 } else { 0.2 };
            painter.rect_filled(thumb, 3.0, p.fg_muted.gamma_multiply(alpha));
            if resp.dragged() {
                let per_px = max / (track.width() - thumb_w).max(1.0);
                pans[fi] = (pans[fi] + resp.drag_delta().x * per_px).clamp(0.0, max);
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(pan_id, pans));
}

/// A file's header: fold chevron, change count, diffstat, path, copy, menu,
/// and the Viewed checkbox.
#[allow(clippy::too_many_arguments)]
fn file_header(app: &mut App, ui: &mut Ui, p: &Palette, f: &FileDiff, fi: usize, r: Rect, collapsed: bool, src: &Source<'_>) {
    let painter = ui.painter();
    let resp = ui.interact(r, egui::Id::new(("file-header", fi)), Sense::click());
    let right = r.right() - 12.0;
    let viewed_rect = Rect::from_min_max(pos2(right - 82.0, r.center().y - 14.0), pos2(right, r.center().y + 14.0));
    let menu_rect = Rect::from_min_size(pos2(viewed_rect.left() - 8.0 - 28.0, r.center().y - 14.0), vec2(28.0, 28.0));
    // Registered after the header, so these win clicks on top of it.
    let viewed_resp = ui.interact(viewed_rect, egui::Id::new(("file-viewed", fi)), Sense::click());
    viewed_resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, f.viewed, "Viewed"));
    let menu_resp = ui.interact(menu_rect, egui::Id::new(("file-menu", fi)), Sense::click());
    let radius = if collapsed { CornerRadius::same(6) } else { CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 } };
    painter.rect_filled(r, radius, p.canvas_subtle);
    painter.rect_stroke(r, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let mut x = r.left() + 10.0;
    let chevron = if collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
    icons::paint(painter, Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(16.0, 16.0)), chevron, p.fg_muted);
    x += 24.0;
    let total = f.additions + f.deletions;
    let g = painter.layout_no_wrap(format!("{total}"), theme::bold(12.0), p.fg);
    let gw = g.size().x;
    painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, p.fg);
    x += gw + 6.0;
    x = diffstat_blocks(painter, p, pos2(x, r.center().y), f.additions, f.deletions) + 8.0;
    let name = match &f.previous_filename {
        Some(prev) if f.status == "renamed" => format!("{prev} → {}", f.filename),
        _ => f.filename.clone(),
    };
    // Path, then a copy button right after it.
    let path_max = (menu_rect.left() - 8.0 - 28.0 - x).max(40.0);
    // Long paths lose their start, so the file name always shows.
    let shown = crate::views::elide_front(painter, &name, theme::mono(12.0), path_max);
    let cut = shown != name;
    let g = painter.layout_no_wrap(shown, theme::mono(12.0), p.fg);
    let path_w = g.size().x;
    let path_rect = Rect::from_min_size(pos2(x, r.center().y - g.size().y / 2.0), g.size());
    painter.galley(path_rect.min, g, p.fg);
    if cut {
        crate::views::tip(&ui.interact(path_rect, egui::Id::new(("file-path", fi)), Sense::hover()), &name);
    }
    let copy_rect = Rect::from_min_size(pos2(x + path_w + 6.0, r.center().y - 12.0), vec2(24.0, 24.0));
    let copy_resp = ui.interact(copy_rect, egui::Id::new(("file-copy", fi)), Sense::click());
    copy_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Copy path"));
    if copy_resp.hovered() {
        painter.rect_filled(copy_rect, 6.0, p.border.gamma_multiply(0.4));
    }
    icons::paint(painter, copy_rect.shrink(5.0), Icon::Copy, p.fg_muted);
    // "⋯" menu.
    if menu_resp.hovered() {
        painter.rect_filled(menu_rect, 6.0, p.border.gamma_multiply(0.4));
    }
    icons::paint(painter, menu_rect.shrink(6.0), Icon::Kebab, p.fg_muted);
    viewed_box(painter, p, viewed_rect, f.viewed, viewed_resp.hovered());
    if resp.hovered() || viewed_resp.hovered() || copy_resp.hovered() || menu_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let file_url = format!("https://github.com/{}/blob/{}/{}", src.repo, src.head, f.filename);
    egui::Popup::menu(&menu_resp).show(|ui| {
        ui.set_min_width(200.0);
        if ui.button("Copy path").clicked() {
            app.actions.push(Action::Copy(f.filename.clone()));
        }
        if !src.head.is_empty() && f.status != "removed" && ui.button("View file on GitHub").clicked() {
            app.actions.push(Action::OpenUrl(file_url.clone()));
        }
        ui.separator();
        let label = if collapsed { "Expand file" } else { "Collapse file" };
        if ui.button(label).clicked() {
            app.actions.push(Action::ToggleFile(f.filename.clone()));
        }
    });
    let copy_resp = copy_resp.tip("Copy path");
    if viewed_resp.clicked() {
        app.actions.push(Action::SetViewed(f.filename.clone(), !f.viewed));
    } else if copy_resp.clicked() {
        app.actions.push(Action::Copy(f.filename.clone()));
    } else if resp.clicked() && !menu_resp.clicked() {
        app.actions.push(Action::ToggleFile(f.filename.clone()));
    }
}

/// GitHub's "☐ Viewed" button at the right of each file header.
fn viewed_box(painter: &egui::Painter, p: &Palette, r: Rect, viewed: bool, hovered: bool) {
    let bg = if viewed {
        p.accent_subtle
    } else if hovered {
        p.hover_row
    } else {
        p.btn_bg
    };
    let border = if viewed { p.accent.gamma_multiply(0.5) } else { p.border };
    painter.rect_filled(r, 6.0, bg);
    painter.rect_stroke(r, 6.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let b = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 7.0), vec2(14.0, 14.0));
    if viewed {
        painter.rect_filled(b, 3.0, p.accent_emphasis);
        icons::paint(painter, b.shrink(1.5), Icon::Check, egui::Color32::WHITE);
    } else {
        painter.rect_stroke(b, 3.0, Stroke::new(1.0, p.fg_muted), StrokeKind::Inside);
    }
    painter.text(pos2(b.right() + 8.0, r.center().y), Align2::LEFT_CENTER, "Viewed", theme::body(12.0), p.fg);
}

/// GitHub's unfold arrows: ↑, ↓, or ↕ for "expand all".
fn unfold_arrow(painter: &egui::Painter, c: egui::Pos2, dir: Expand, color: egui::Color32) {
    let s = Stroke::new(1.5, color);
    let head = |d: f32| {
        painter.line_segment([pos2(c.x - 4.0, c.y + 1.0 * d), pos2(c.x, c.y + 5.0 * d)], s);
        painter.line_segment([pos2(c.x + 4.0, c.y + 1.0 * d), pos2(c.x, c.y + 5.0 * d)], s);
    };
    painter.line_segment([pos2(c.x, c.y - 5.0), pos2(c.x, c.y + 5.0)], s);
    match dir {
        Expand::Up => head(-1.0),
        Expand::Down => head(1.0),
        _ => {
            head(-1.0);
            head(1.0);
        }
    }
}

/// The sides and rounded bottom of a box, as one line.
fn rounded_bottom(painter: &egui::Painter, r: Rect, radius: f32, stroke: Stroke) {
    let arc = |cx: f32, cy: f32, from: f32, to: f32| -> Vec<egui::Pos2> {
        (0..=6).map(|i| {
            let a = from + (to - from) * i as f32 / 6.0;
            pos2(cx + radius * a.cos(), cy + radius * a.sin())
        }).collect()
    };
    use std::f32::consts::PI;
    let mut pts = vec![pos2(r.left(), r.top())];
    pts.extend(arc(r.left() + radius, r.bottom() - radius, PI, PI / 2.0));
    pts.extend(arc(r.right() - radius, r.bottom() - radius, PI / 2.0, 0.0));
    pts.push(pos2(r.right(), r.top()));
    painter.add(egui::Shape::line(pts, stroke));
}

fn side_borders(painter: &egui::Painter, p: &Palette, r: Rect) {
    let s = Stroke::new(1.0, p.border);
    painter.line_segment([pos2(r.left() + 0.5, r.top()), pos2(r.left() + 0.5, r.bottom())], s);
    painter.line_segment([pos2(r.right() - 0.5, r.top()), pos2(r.right() - 0.5, r.bottom())], s);
}

/// GitHub's five little squares showing the add/delete ratio.
pub fn diffstat_blocks(painter: &egui::Painter, p: &Palette, left_center: egui::Pos2, add: u64, del: u64) -> f32 {
    // Like GitHub: under 5 changes, fill only that many blocks.
    let total = add + del;
    let filled = total.min(5) as usize;
    let green = if total == 0 { 0 } else { ((add as f32 / total as f32) * filled as f32).round() as usize };
    let red = filled - green;
    let mut x = left_center.x;
    for i in 0..5 {
        let color = if i < green {
            p.open
        } else if i < green + red {
            p.closed
        } else {
            p.border
        };
        let r = Rect::from_min_size(pos2(x, left_center.y - 4.0), vec2(8.0, 8.0));
        painter.rect_filled(r, 0.0, color);
        x += 9.0;
    }
    x
}
