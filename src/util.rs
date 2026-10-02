//! Small helpers: relative times, opening links, cleaning up GitHub markdown.

use chrono::{DateTime, Datelike, Local, Utc};

/// "just now", "5 minutes ago", "3 days ago", "on Mar 4", like github.com.
pub fn ago(iso: &str) -> String {
    let Ok(t) = DateTime::parse_from_rfc3339(iso) else { return String::new() };
    let t = t.with_timezone(&Utc);
    let secs = (Utc::now() - t).num_seconds().max(0);
    let plural = |n: i64, unit: &str| {
        if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") }
    };
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=86_399 => plural(secs / 3600, "hour"),
        86_400..=2_591_999 => {
            let d = secs / 86_400;
            if d == 1 { "yesterday".into() } else { format!("{d} days ago") }
        }
        _ => {
            let local = t.with_timezone(&Local);
            if local.year() == Local::now().year() { format!("on {}", local.format("%b %-d")) } else { format!("on {}", local.format("%b %-d, %Y")) }
        }
    }
}

/// "Sep 28, 2026" for grouping commits by day.
pub fn day(iso: &str) -> String {
    DateTime::parse_from_rfc3339(iso).map(|t| t.with_timezone(&Local).format("%b %-d, %Y").to_string()).unwrap_or_default()
}

/// Open a link in the default browser with each OS's own API.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    {
        // Uses NSWorkspace under the hood.
        let _ = webbrowser::open(url);
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, target) = (wide("open"), wide(url));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Ask the desktop through the XDG "OpenURI" portal over D-Bus.
        let url = url.to_string();
        std::thread::spawn(move || {
            let run = || -> zbus::Result<()> {
                let conn = zbus::blocking::Connection::session()?;
                let opts: std::collections::HashMap<&str, zbus::zvariant::Value> = Default::default();
                conn.call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.OpenURI"),
                    "OpenURI",
                    &("", url.as_str(), opts),
                )?;
                Ok(())
            };
            if let Err(e) = run() {
                eprintln!("could not open link: {e}");
            }
        });
    }
}

/// GitHub markdown often contains HTML (template comments, <details>, <img>,
/// <br>, bot tables). Turn the common cases into plain markdown and drop the
/// rest. `<details>`, `<summary>…</summary>` and `</details>` stay, each on
/// its own line, for the markdown viewer to fold.
pub fn clean_markdown(src: &str) -> String {
    let mut s = String::with_capacity(src.len());
    let mut rest = src;
    // Remove <!-- comments --> (PR templates are full of them).
    while let Some(i) = rest.find("<!--") {
        s.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => {
                rest = "";
            }
        }
    }
    s.push_str(rest);

    let mut out = String::with_capacity(s.len());
    let mut in_fence = false;
    let mut lines = s.lines();
    while let Some(line) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if in_fence || !line.contains('<') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // A <summary> or <table> can span lines: gather it into one.
        let mut line = line.to_string();
        for (open, close, join) in [("<summary", "</summary>", " "), ("<table", "</table>", "\n")] {
            let mut n = 0;
            while unclosed(&line, open, close) && n < 500 {
                let Some(next) = lines.next() else { break };
                line.push_str(join);
                line.push_str(next);
                n += 1;
            }
        }
        html_line(&line, &mut out);
    }
    out
}

/// Whether the last `open` tag in `s` has no `close` after it.
fn unclosed(s: &str, open: &str, close: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.rfind(open).is_some_and(|i| !lower[i..].contains(close))
}

/// Where `<name` starts as a whole tag (so "<th" doesn't match "<thead").
fn find_tag(lower: &str, from: usize, name: &str) -> Option<usize> {
    let mut at = from;
    while let Some(i) = lower.get(at..)?.find(name) {
        let i = at + i;
        match lower[i + name.len()..].chars().next() {
            Some('>' | ' ' | '\t' | '\n' | '/') => return Some(i),
            _ => at = i + name.len(),
        }
    }
    None
}

/// One line with HTML in it. Block tags (details, summary, tables) go on
/// lines of their own, followed by a blank line so Markdown after them
/// still parses. Lines inside a quote or list keep their prefix.
fn html_line(line: &str, out: &mut String) {
    let lower = line.to_ascii_lowercase();
    let next_block = |from: usize| ["<details", "</details", "<summary", "<table"].into_iter().filter_map(|t| find_tag(&lower, from, t).map(|i| (i, t))).min();
    if next_block(0).is_none() {
        out.push_str(&strip_tags(line));
        out.push('\n');
        return;
    }
    let (prefix, cont) = line_prefix(line);
    let blank = cont.trim_end().to_string();
    let mut lead = prefix.to_string();
    let mut emit = |text: &str, block: bool, out: &mut String| {
        if text.trim().is_empty() {
            return;
        }
        for l in text.lines() {
            out.push_str(&lead);
            out.push_str(l);
            out.push('\n');
            lead = cont.clone();
        }
        if block {
            out.push_str(&blank);
            out.push('\n');
        }
    };
    let mut at = prefix.len();
    while let Some((i, tag)) = next_block(at) {
        emit(&strip_tags(&line[at..i]), false, out);
        let end = line[i..].find('>').map_or(line.len(), |j| i + j + 1);
        match tag {
            "<details" => {
                let open = lower[i..end].contains("open");
                emit(if open { "<details open>" } else { "<details>" }, true, out);
                at = end;
            }
            "</details" => {
                emit("</details>", true, out);
                at = end;
            }
            "<summary" => {
                let close = lower[end..].find("</summary").map_or(line.len(), |k| end + k);
                let text = strip_tags(&line[end..close]).replace('\n', " ");
                emit(&format!("<summary>{}</summary>", text.trim()), true, out);
                at = line[close..].find('>').map_or(line.len(), |j| close + j + 1);
            }
            _ => {
                let close = lower[i..].find("</table>").map_or(line.len(), |k| i + k + "</table>".len());
                if !out.ends_with("\n\n") && !out.is_empty() {
                    out.push_str(&blank);
                    out.push('\n');
                }
                let table = &line[i..close];
                let t = table.to_ascii_lowercase();
                // Cells holding folds or code can't live in a Markdown table
                // cell; show each cell as ordinary blocks instead.
                if t.contains("<details") || t.contains("<pre") || table.contains("```") {
                    emit(&table_blocks(table), true, out);
                } else {
                    emit(&html_table(table), true, out);
                }
                at = close;
            }
        }
    }
    emit(&strip_tags(&line[at..]), false, out);
}

/// The quote markers and list bullet at the start of a line, and the same
/// width of indent for lines that continue it.
fn line_prefix(line: &str) -> (&str, String) {
    let quote = line.len() - line.trim_start_matches([' ', '\t', '>']).len();
    let rest = &line[quote..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let marker = if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        2
    } else if digits > 0 && (rest[digits..].starts_with(". ") || rest[digits..].starts_with(") ")) {
        digits + 2
    } else {
        0
    };
    let prefix = &line[..quote + marker];
    (prefix, format!("{}{}", &line[..quote], " ".repeat(marker)))
}

/// An HTML `<table>` whose cells hold block content (folds, code), as one
/// block per cell, top to bottom.
fn table_blocks(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut at = 0;
    while let Some(start) = [find_tag(&lower, at, "<td"), find_tag(&lower, at, "<th")].into_iter().flatten().min() {
        let open_end = lower[start..].find('>').map_or(lower.len(), |j| start + j + 1);
        let stop = ["</td", "</th"].iter().filter_map(|t| lower[open_end..].find(t)).min().map_or(lower.len(), |k| open_end + k);
        let md = clean_markdown(&dedent(&html[open_end..stop]));
        if !md.trim().is_empty() {
            out.push_str(md.trim_matches('\n'));
            out.push_str("\n\n");
        }
        at = stop.max(open_end);
    }
    out
}

/// Removes the indent HTML source adds to every line, so cell content
/// doesn't turn into indented code blocks.
fn dedent(s: &str) -> String {
    let mut lines = s.lines();
    let first = lines.next().unwrap_or("").trim();
    let rest: Vec<&str> = lines.collect();
    let indent = rest.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = first.to_string();
    for l in rest {
        out.push('\n');
        out.push_str(l.get(indent..).unwrap_or(l.trim_start()));
    }
    out
}

/// An HTML `<table>` as a Markdown table. The first row is the header.
fn html_table(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut align = Vec::new();
    let mut at = 0;
    while let Some(tr) = find_tag(&lower, at, "<tr") {
        let row_end = find_tag(&lower, tr + 3, "<tr").unwrap_or(lower.len());
        let mut cells = Vec::new();
        let mut c = tr;
        loop {
            let next = [find_tag(&lower[..row_end], c, "<td"), find_tag(&lower[..row_end], c, "<th")].into_iter().flatten().min();
            let Some(start) = next else { break };
            let open_end = lower[start..row_end].find('>').map_or(row_end, |j| start + j + 1);
            if rows.is_empty() {
                align.push(match attr(&html[start..open_end], "align") {
                    Some("center") => ":---:",
                    Some("right") => "---:",
                    _ => "---",
                });
            }
            let stop = ["</td", "</th", "<td", "<th", "</tr"].iter().filter_map(|t| lower[open_end..row_end].find(t)).min().map_or(row_end, |k| open_end + k);
            // <br> becomes a newline in strip_tags; keep it as a line break in the cell.
            let text = strip_tags(&html[open_end..stop].replace('\n', " "));
            let lines: Vec<String> = text.split('\n').map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect();
            cells.push(lines.join("<br>").replace('|', "\\|"));
            c = stop.max(open_end);
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
        at = row_end;
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    align.resize(cols, "---");
    let line = |cells: &[String]| {
        let mut s = String::from("|");
        for i in 0..cols {
            s.push(' ');
            s.push_str(cells.get(i).map_or("", String::as_str));
            s.push_str(" |");
        }
        s
    };
    let mut md = line(&rows[0]);
    md.push_str(&format!("\n|{}|", align.iter().map(|a| format!(" {a} ")).collect::<Vec<_>>().join("|")));
    for r in &rows[1..] {
        md.push('\n');
        md.push_str(&line(r));
    }
    md
}

fn strip_tags(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let tag = &rest[i + 1..i + j];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        match name.as_str() {
            "br" | "p" | "div" => out.push('\n'),
            "b" | "strong" => out.push_str("**"),
            "i" | "em" => out.push('*'),
            "code" => out.push('`'),
            "img" => {
                let alt = attr(tag, "alt").unwrap_or("image");
                if let Some(src) = attr(tag, "src") {
                    out.push_str(&format!("![{alt}]({src})"));
                }
            }
            "a" if !tag.starts_with('/') => {
                if let Some(href) = attr(tag, "href") {
                    // Leave the text; append the link after it closes.
                    let after = &rest[i + j + 1..];
                    if let Some(k) = after.to_ascii_lowercase().find("</a>") {
                        let text = strip_tags(&after[..k]);
                        out.push_str(&format!("[{}]({href})", text.trim()));
                        rest = &after[k + 4..];
                        continue;
                    }
                }
            }
            // A real tag we can't render: drop it. Anything else (like
            // "a < b" or "<T>") isn't HTML, so keep it as text. Details and
            // tables only get here from odd spots, like inside a table cell.
            "details" | "summary" | "sub" | "sup" | "span" | "table" | "tr" | "td" | "th" | "thead" | "tbody" | "ul" | "ol" | "li" | "h1" | "h2" | "h3"
            | "h4" | "picture" | "source" | "video" | "a" | "kbd" | "blockquote" | "hr" | "center" | "font" | "u" | "s" | "del" => {}
            _ => out.push_str(&rest[i..i + j + 1]),
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let i = tag.find(&format!("{name}="))?;
    let v = &tag[i + name.len() + 1..];
    let q = v.chars().next()?;
    if q == '"' || q == '\'' {
        let v = &v[1..];
        Some(&v[..v.find(q)?])
    } else {
        Some(v.split_whitespace().next()?)
    }
}

/// Reads what you typed in "Go to pull request" as (repo, number). Accepts
/// a link, `owner/repo#123`, or `#123` / `123` (then `default_repo` is used).
pub fn parse_pr_ref(input: &str, default_repo: Option<&str>) -> Option<(String, u64)> {
    let s = input.trim().trim_end_matches('/');
    if let Some(i) = s.find("github.com/") {
        // github.com/owner/repo/pull/123, maybe with /files or #comment after.
        let mut parts = s[i + "github.com/".len()..].split(['/', '#', '?']);
        let (owner, repo, kind, n) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        return (kind == "pull" && !owner.is_empty() && !repo.is_empty()).then(|| Some((format!("{owner}/{repo}"), n.parse().ok()?))).flatten();
    }
    if let Some((repo, n)) = s.split_once('#').filter(|(r, _)| r.contains('/')) {
        return Some((repo.trim().to_string(), n.trim().parse().ok()?));
    }
    let n: u64 = s.trim_start_matches('#').parse().ok()?;
    Some((default_repo?.to_string(), n))
}

/// Whether every typed word shows up somewhere in `text`, ignoring case.
/// `_ - / ( ) :` count as spaces, so "argo rollouts" finds "argo_rollouts".
pub fn words_match(text: &str, typed: &str) -> bool {
    let norm = |t: &str| t.to_lowercase().replace(['_', '-', '/', '(', ')', ':', '.'], " ");
    let text = norm(text);
    let typed = norm(typed);
    let mut words = typed.split_whitespace().peekable();
    words.peek().is_some() && words.all(|w| text.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_with_folds_become_blocks() {
        let md = clean_markdown("<table>\n<tr><td>\n  <details><summary>Effort</summary>\n\n  ```yaml\n  a: 1\n  ```\n  </details>\n</td></tr>\n</table>");
        assert!(md.contains("<summary>Effort</summary>"), "{md}");
        assert!(md.contains("```yaml\na: 1\n```"), "{md}");
        assert!(!md.contains('|'), "{md}");
    }

    #[test]
    fn matches_words() {
        assert!(words_match("feat(cicd/argo_rollouts): add canary", "argo rollouts"));
        assert!(words_match("Fix the thing", "THING fix"));
        assert!(!words_match("Fix the thing", "fix other"));
        assert!(!words_match("anything", "  "));
    }

    #[test]
    fn parses_pr_refs() {
        let r = |s| parse_pr_ref(s, Some("o/r"));
        let want = |repo: &str, n| Some((repo.to_string(), n));
        assert_eq!(r("https://github.com/a/b/pull/12"), want("a/b", 12));
        assert_eq!(r("github.com/a/b/pull/12/files#diff-1"), want("a/b", 12));
        assert_eq!(r(" a/b#7 "), want("a/b", 7));
        assert_eq!(r("#119316"), want("o/r", 119316));
        assert_eq!(r("42"), want("o/r", 42));
        assert_eq!(parse_pr_ref("42", None), None);
        assert_eq!(r("https://github.com/a/b/issues/3"), None);
        assert_eq!(r("hello"), None);
    }

    #[test]
    fn cleans_html() {
        let md = clean_markdown("hi <!-- template\nstuff -->there\n<img alt=\"x\" src=\"https://a/b.png\">\na < b");
        assert!(md.contains("hi there"));
        assert!(md.contains("![x](https://a/b.png)"));
        assert!(md.contains("a < b"));
        assert_eq!(clean_markdown("<a href=\"u\">text</a>").trim(), "[text](u)");
    }

    #[test]
    fn keeps_details() {
        let md = clean_markdown("<details><summary><b>Logs</b>\nhere</summary>\nbody <code>x</code></details>after");
        assert_eq!(md, "<details>\n\n<summary>**Logs** here</summary>\n\nbody `x`\n</details>\n\nafter\n");
        assert!(clean_markdown("<details open>\n").starts_with("<details open>\n"));
        // In a list or quote, split lines keep their place.
        assert_eq!(clean_markdown("- <details><summary>S</summary>"), "- <details>\n\n  <summary>S</summary>\n\n");
        assert_eq!(clean_markdown("> </details>"), "> </details>\n>\n");
        // In code, HTML stays as written.
        assert_eq!(clean_markdown("```\n<details>\n```"), "```\n<details>\n```\n");
    }

    #[test]
    fn html_tables_become_markdown() {
        let md = clean_markdown(
            "Report:\n<table>\n<thead><tr><th>Name</th><th align=\"right\">Count</th></tr></thead>\n<tr><td>a|b</td><td>1<br>2</td></tr>\n<tr><td><a href=\"u\">x</a></td></tr>\n</table>\nend",
        );
        assert_eq!(md, "Report:\n\n| Name | Count |\n| --- | ---: |\n| a\\|b | 1<br>2 |\n| [x](u) |  |\n\nend\n");
    }
}
