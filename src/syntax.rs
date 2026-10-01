//! Syntax highlighting with tree-sitter (via arborium's bundled grammars),
//! colored like GitHub's Primer syntax theme.

use crate::theme::Palette;
use egui::Color32;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// What a piece of code is, as far as coloring goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Comment,
    Keyword,
    String,
    Number,
    Function,
    Type,
    Constant,
    Variable,
    Property,
    Tag,
    Attribute,
    Operator,
    Punctuation,
    // Markdown.
    Heading,
    Strong,
    Emphasis,
    Link,
    ListMarker,
}

/// Colored runs for each line: (byte range within the line, kind).
pub type Lines = Arc<Vec<Vec<(std::ops::Range<usize>, Tok)>>>;

thread_local! {
    static HIGHLIGHTER: RefCell<arborium::Highlighter> = RefCell::new(arborium::Highlighter::new());
    static CACHE: RefCell<HashMap<u64, Lines>> = RefCell::new(HashMap::new());
}

/// The tree-sitter language for a file, from its name.
pub fn lang_for_path(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let special = match name.to_ascii_lowercase().as_str() {
        "dockerfile" | "containerfile" => Some("dockerfile"),
        "makefile" | "gnumakefile" => Some("make"),
        "cmakelists.txt" => Some("cmake"),
        "build" | "build.bazel" | "workspace" | "tiltfile" => Some("starlark"),
        "justfile" => Some("just"),
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => Some("bash"),
        "go.mod" | "go.sum" => None,
        _ => None,
    };
    if special.is_some() {
        return special;
    }
    if name.starts_with("Dockerfile.") || name.ends_with(".Dockerfile") {
        return Some("dockerfile");
    }
    arborium::detect_language(name)
}

/// The language for a file in a diff: by name, else by a `#!` line when
/// the diff starts at the top of the file.
pub fn lang_for_diff(path: &str, patch: &str) -> Option<&'static str> {
    lang_for_path(path).or_else(|| {
        let mut lines = patch.lines();
        let header = lines.next()?;
        let new_start = header.split(' ').find(|w| w.starts_with('+'))?;
        if !(new_start == "+1" || new_start.starts_with("+1,")) {
            return None;
        }
        let first = lines.find(|l| !l.starts_with('-'))?.get(1..)?;
        lang_for_shebang(first)
    })
}

/// "#!/bin/bash", "#!/usr/bin/env python3", ...
fn lang_for_shebang(line: &str) -> Option<&'static str> {
    let rest = line.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-'))?;
    }
    let prog = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    Some(match prog {
        "sh" | "bash" | "dash" | "ksh" => "bash",
        "zsh" => "zsh",
        "fish" => "fish",
        "python" => "python",
        "node" | "deno" | "bun" => "javascript",
        "ruby" => "ruby",
        "perl" => "perl",
        "php" => "php",
        "lua" => "lua",
        _ => return None,
    })
}

/// The language for a fenced code block's info string ("rust", "yml", "sh", ...).
pub fn lang_for_fence(info: &str) -> Option<String> {
    let word = info.split([' ', ',', '{']).next().unwrap_or("").trim().to_ascii_lowercase();
    if word.is_empty() {
        return None;
    }
    // Names people write that aren't file extensions.
    let as_file = match word.as_str() {
        "shell" | "console" | "shell-session" | "zsh" => "x.sh",
        "javascript" | "node" => "x.js",
        "typescript" => "x.ts",
        "python" | "python3" => "x.py",
        "rust" => "x.rs",
        "golang" => "x.go",
        "ruby" => "x.rb",
        "terraform" => "x.tf",
        "docker" | "dockerfile" => "Dockerfile",
        "make" | "makefile" => "Makefile",
        "c++" => "x.cpp",
        "c#" | "csharp" => "x.cs",
        "jsonc" | "json5" => "x.json",
        "markdown" => "x.md",
        _ => "",
    };
    if !as_file.is_empty() {
        return lang_for_path(as_file).map(str::to_string);
    }
    lang_for_path(&format!("x.{word}"))
        .map(str::to_string)
        .or_else(|| HIGHLIGHTER.with(|h| h.borrow().store().get(&word).is_some()).then_some(word))
}

/// Highlights `source` as `lang`, split into lines. Cached by content, so
/// redraws and rebuilt layouts are free.
pub fn highlight(lang: &str, source: &str) -> Option<Lines> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut h);
    source.hash(&mut h);
    let key = h.finish();
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(hit);
    }
    let spans = HIGHLIGHTER.with(|hl| hl.borrow_mut().highlight_spans(lang, source)).ok()?;

    // One kind per byte; later (inner) captures win over the ones they sit in.
    let mut kinds = vec![Tok::Plain; source.len()];
    // Same range captured twice ("key" as property and string): the first
    // query match wins, as in tree-sitter's own highlighter.
    let mut spans = spans;
    spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
    spans.dedup_by(|b, a| a.start == b.start && a.end == b.end);
    for s in &spans {
        let tok = tok_for(&s.capture);
        let (a, b) = (s.start as usize, (s.end as usize).min(source.len()));
        if a < b {
            kinds[a..b].fill(tok);
        }
    }

    let mut lines = Vec::new();
    let mut start = 0;
    for line in source.split('\n') {
        let end = start + line.len();
        let mut runs: Vec<(std::ops::Range<usize>, Tok)> = Vec::new();
        let mut i = start;
        while i < end {
            let k = kinds[i];
            let mut j = i + 1;
            while j < end && kinds[j] == k {
                j += 1;
            }
            runs.push((i - start..j - start, k));
            i = j;
        }
        lines.push(runs);
        start = end + 1;
    }
    if lang == "markdown" {
        markdown_lines(source, &mut lines);
    }
    let lines: Lines = Arc::new(lines);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 400 {
            c.clear();
        }
        c.insert(key, lines.clone());
    });
    Some(lines)
}

/// Highlights lines cut out of a file (a diff hunk). They're dedented by
/// their shared indent first, so a hunk from deep inside a block still
/// parses. YAML can't survive starting mid-block, so it's done a line at a
/// time (its lines mostly stand alone anyway). Ranges are for the original
/// lines.
pub fn highlight_fragment(lang: &str, lines: &[&str]) -> Option<Lines> {
    let indent_of = |l: &str| l.len() - l.trim_start_matches([' ', '\t']).len();
    let shift = |runs: &[(std::ops::Range<usize>, Tok)], by: usize| runs.iter().map(|(r, t)| (r.start + by..r.end + by, *t)).collect::<Vec<_>>();
    if lang == "yaml" {
        let out = lines
            .iter()
            .map(|l| {
                let ind = indent_of(l);
                // GitHub shows YAML keys in its tag green.
                let runs = highlight(lang, &l[ind..]).and_then(|h| h.first().map(|r| shift(r, ind))).unwrap_or_default();
                runs.into_iter().map(|(r, t)| (r, if t == Tok::Property { Tok::Tag } else { t })).collect()
            })
            .collect();
        return Some(Arc::new(out));
    }
    let common = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| indent_of(l)).min().unwrap_or(0);
    let source = lines.iter().map(|l| l.get(common..).unwrap_or("")).collect::<Vec<_>>().join("\n");
    let h = highlight(lang, &source)?;
    Some(Arc::new(h.iter().map(|r| shift(r, common)).collect()))
}

/// The bundled Markdown grammar only knows blocks, so headings, list
/// markers, `code`, **bold**, *italic* and links are found here. Lines in
/// fenced code keep the code's own colors.
fn markdown_lines(source: &str, lines: &mut [Vec<(std::ops::Range<usize>, Tok)>]) {
    let mut fenced = false;
    for (line, runs) in source.split('\n').zip(lines.iter_mut()) {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            *runs = markdown_inline(line);
        }
    }
}

/// Colors for one line of Markdown prose.
fn markdown_inline(line: &str) -> Vec<(std::ops::Range<usize>, Tok)> {
    let b = line.as_bytes();
    let mut kinds = vec![Tok::Plain; b.len()];
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|&c| c == b'#').count();
    if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') || rest.len() == hashes && hashes > 0 {
        return vec![(0..line.len(), Tok::Heading)];
    }
    // A list marker: "-", "*", "+" or "1." / "1)", then a space.
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let marker = match rest.as_bytes() {
        [b'-' | b'*' | b'+', b' ', ..] => 1,
        r if digits > 0 && matches!(r.get(digits), Some(b'.' | b')')) && r.get(digits + 1) == Some(&b' ') => digits + 1,
        _ => 0,
    };
    kinds[indent..indent + marker].fill(Tok::ListMarker);
    // Code spans first: nothing inside them is formatting, and they keep
    // their color inside **bold** too.
    let mut i = indent + marker;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let ticks = b[i..].iter().take_while(|&&c| c == b'`').count();
        match line[i + ticks..].find(&"`".repeat(ticks)).map(|j| i + ticks + j) {
            Some(end) => {
                kinds[i..end + ticks].fill(Tok::Constant);
                i = end + ticks;
            }
            None => i += ticks,
        }
    }
    let mut i = indent + marker;
    let word = |c: u8| c.is_ascii_alphanumeric();
    while i < b.len() {
        if kinds[i] == Tok::Constant {
            i += 1;
            continue;
        }
        match b[i] {
            c @ (b'*' | b'_') => {
                let double = b.get(i + 1) == Some(&c);
                let n = if double { 2 } else { 1 };
                let pair = [c, c];
                let mark = &pair[..n];
                // "_" inside a word (snake_case) isn't emphasis.
                let opens = b.get(i + n).is_some_and(|&x| x != b' ' && x != c) && !(c == b'_' && i > 0 && word(b[i - 1]));
                let close = opens.then(|| b[i + n..].windows(n).position(|w| w == mark)).flatten().map(|j| i + n + j);
                match close {
                    Some(end) if end > i + n && b[end - 1] != b' ' => {
                        let style = if double { Tok::Strong } else { Tok::Emphasis };
                        for k in &mut kinds[i..end + n] {
                            if *k != Tok::Constant {
                                *k = style;
                            }
                        }
                        i = end + n;
                    }
                    _ => i += n,
                }
            }
            b']' if b.get(i + 1) == Some(&b'(') => {
                match line[i + 1..].find(')') {
                    Some(j) => {
                        kinds[i + 2..i + 1 + j].fill(Tok::Link);
                        i += j + 2;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    let mut runs = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        let k = kinds[i];
        let mut j = i + 1;
        while j < kinds.len() && kinds[j] == k {
            j += 1;
        }
        runs.push((i..j, k));
        i = j;
    }
    runs
}

/// Tree-sitter capture names ("keyword.function", "string.special", ...)
/// grouped into the handful of colors GitHub uses.
fn tok_for(capture: &str) -> Tok {
    let head = capture.split('.').next().unwrap_or(capture);
    match head {
        "comment" => Tok::Comment,
        "keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass" | "preproc" | "define" => Tok::Keyword,
        "string" | "character" | "escape" | "regexp" => Tok::String,
        "number" | "float" | "boolean" => Tok::Number,
        "function" | "method" | "constructor" => Tok::Function,
        "type" | "namespace" | "module" | "label" => Tok::Type,
        "constant" => Tok::Constant,
        "variable" if capture.contains("builtin") => Tok::Constant,
        "variable" | "parameter" => Tok::Variable,
        "property" | "field" => Tok::Property,
        "tag" => Tok::Tag,
        "attribute" => Tok::Attribute,
        "text" if capture == "text.title" => Tok::Heading,
        "text" if capture == "text.uri" => Tok::Link,
        "operator" => Tok::Operator,
        "punctuation" => Tok::Punctuation,
        _ => Tok::Plain,
    }
}

/// GitHub's Primer syntax colors (prettylights), light and dark.
pub fn color(tok: Tok, p: &Palette) -> Color32 {
    let hex = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let (light, dark) = match tok {
        Tok::Plain | Tok::Variable | Tok::Punctuation | Tok::Strong | Tok::Emphasis => return p.fg,
        Tok::Heading => (0x0550ae, 0x79c0ff),
        Tok::Link => (0x0a3069, 0xa5d6ff),
        Tok::ListMarker => (0x3b2300, 0xf2cc60),
        Tok::Comment => (0x59636e, 0x9198a1),
        Tok::Keyword | Tok::Operator => (0xcf222e, 0xff7b72),
        Tok::String => (0x0a3069, 0xa5d6ff),
        Tok::Number | Tok::Constant | Tok::Property | Tok::Attribute => (0x0550ae, 0x79c0ff),
        Tok::Function => (0x6639ba, 0xd2a8ff),
        Tok::Type => (0x953800, 0xffa657),
        Tok::Tag => (0x116329, 0x7ee787),
    };
    hex(if p.dark { dark } else { light })
}

/// One line of code as a layout job with its colors.
pub fn job(text: &str, runs: Option<&[(std::ops::Range<usize>, Tok)]>, font: egui::FontId, fallback: Color32, p: &Palette) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let fmt = |color| egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
    let bold = egui::FontId::new(
        font.size,
        match font.family {
            egui::FontFamily::Monospace => egui::FontFamily::Name(crate::theme::MONO_BOLD.into()),
            _ => egui::FontFamily::Name(crate::theme::BOLD.into()),
        },
    );
    let styled = |tok: Tok, color: Color32| {
        let mut f = fmt(color);
        match tok {
            Tok::Heading | Tok::Strong => f.font_id = bold.clone(),
            Tok::Emphasis => f.italics = true,
            Tok::Link => f.underline = egui::Stroke::new(1.0, color),
            _ => {}
        }
        f
    };
    match runs {
        Some(runs) if !runs.is_empty() => {
            let mut at = 0;
            for (r, tok) in runs {
                let (a, b) = (r.start.min(text.len()), r.end.min(text.len()));
                if a > at {
                    job.append(&text[at..a], 0.0, fmt(fallback));
                }
                if b > a && text.is_char_boundary(a) && text.is_char_boundary(b) {
                    let c = if *tok == Tok::Plain { fallback } else { color(*tok, p) };
                    job.append(&text[a..b], 0.0, styled(*tok, c));
                    at = b;
                }
            }
            if at < text.len() {
                job.append(&text[at..], 0.0, fmt(fallback));
            }
        }
        _ => job.append(text, 0.0, fmt(fallback)),
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_languages() {
        assert_eq!(lang_for_path("src/main.rs"), Some("rust"));
        assert_eq!(lang_for_path("deploy/Dockerfile"), Some("dockerfile"));
        assert_eq!(lang_for_fence("yml").as_deref(), lang_for_path("x.yaml"));
        assert_eq!(lang_for_fence("rust").as_deref(), Some("rust"));
        assert_eq!(lang_for_fence(""), None);
        assert_eq!(lang_for_diff("bin/publish", "@@ -0,0 +1,3 @@\n+#!/usr/bin/env bash\n+set -e"), Some("bash"));
        assert_eq!(lang_for_diff("bin/publish", "@@ -10,3 +10,3 @@\n #!/bin/sh"), None);
    }

    #[test]
    fn first_capture_wins() {
        // tree-sitter-yaml tags keys as both property and string.
        let lines = highlight("yaml", "key: 'v'").unwrap();
        assert_eq!(lines[0][0], (0..3, Tok::Property));
    }

    #[test]
    fn colors_rust() {
        let lines = highlight("rust", "fn main() {\n    let x = \"hi\"; // note\n}").unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].iter().any(|(_, t)| *t == Tok::Keyword), "{:?}", lines[0]);
        assert!(lines[1].iter().any(|(_, t)| *t == Tok::String), "{:?}", lines[1]);
        assert!(lines[1].iter().any(|(_, t)| *t == Tok::Comment), "{:?}", lines[1]);
    }
}

#[cfg(test)]
#[test]
fn colors_markdown() {
    let lines = highlight("markdown", "### How to\n- Some **bold**, `code` and [a](http://x).\n```sh\nls -la\n```").unwrap();
    assert_eq!(lines[0], vec![(0..10, Tok::Heading)]);
    let kind_of = |n: usize, word: &str, line: &str| {
        let at = line.find(word).unwrap();
        lines[n].iter().find(|(r, _)| r.contains(&at)).unwrap().1
    };
    let l1 = "- Some **bold**, `code` and [a](http://x).";
    assert_eq!(kind_of(1, "-", l1), Tok::ListMarker);
    assert_eq!(kind_of(1, "bold", l1), Tok::Strong);
    assert_eq!(kind_of(1, "code", l1), Tok::Constant);
    assert_eq!(kind_of(1, "http", l1), Tok::Link);
    assert_eq!(kind_of(1, "Some", l1), Tok::Plain);
    let l = highlight("markdown", "**`template` is required**").unwrap();
    assert_eq!(l[0][0], (0..2, Tok::Strong));
    assert_eq!(l[0][1], (2..12, Tok::Constant));
}
