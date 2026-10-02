//! GitHub's extras on top of plain Markdown: @mentions, #123 links, bare
//! URLs, :emoji: codes, color emoji, and ```suggestion blocks. Turns them
//! into ordinary Markdown that the viewer already knows how to draw.

use std::collections::{HashMap, HashSet};

/// Where GitHub keeps its emoji pictures, by Unicode code points.
const EMOJI_CDN: &str = "https://github.githubassets.com/images/icons/emoji/unicode/";

/// GitHub's emoji list: `:name:` -> picture URL. Loaded once from the API.
#[derive(Default)]
pub struct Emojis {
    by_name: HashMap<String, String>,
    /// File names (hex code points, like "1f44d-1f3fb") GitHub has pictures for.
    files: HashSet<String>,
}

impl Emojis {
    pub fn new(by_name: HashMap<String, String>) -> Self {
        let files = by_name.values().filter_map(|u| u.split("/unicode/").nth(1)?.split('.').next().map(str::to_string)).collect();
        Emojis { by_name, files }
    }

    /// Picture for one emoji character sequence, or None if GitHub has none.
    fn unicode_url(&self, seq: &str) -> Option<String> {
        let hex = |s: &str| s.chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-");
        let full = hex(seq);
        let plain = hex(&seq.replace('\u{fe0f}', ""));
        let first = hex(&seq.chars().take(1).collect::<String>());
        if self.files.is_empty() {
            // List not loaded yet: GitHub's names mostly drop U+FE0F.
            return Some(format!("{EMOJI_CDN}{plain}.png"));
        }
        [full, plain, first].into_iter().find(|h| self.files.contains(h)).map(|h| format!("{EMOJI_CDN}{h}.png"))
    }
}

/// Picture URIs start with this so our emoji loader (not the normal image
/// loader) draws them at text size.
pub const EMOJI_SCHEME: &str = "emoji:";

fn emoji_image(alt: &str, url: &str) -> String {
    format!("![{alt}]({EMOJI_SCHEME}{url})")
}

/// Rewrites GitHub-flavored Markdown into plain Markdown. `repo` is
/// "owner/name", used for #123 links.
pub fn to_markdown(src: &str, repo: &str, emojis: &Emojis) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    // The open code fence, and whether it's a ```suggestion.
    let mut fence: Option<(String, bool)> = None;
    for line in src.lines() {
        let t = line.trim_start();
        if let Some((marker, suggestion)) = &fence {
            if t.starts_with(marker.as_str()) {
                fence = None;
            } else if *suggestion {
                // Show the proposed lines as additions.
                out.push_str("+ ");
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            let marker: String = t.chars().take_while(|&c| c == '`' || c == '~').collect();
            let suggestion = t[marker.len()..].trim() == "suggestion";
            if suggestion {
                out.push_str("**Suggested change**\n\n");
                out.push_str(&marker);
                out.push_str("diff\n");
            } else {
                out.push_str(line);
                out.push('\n');
            }
            fence = Some((marker, suggestion));
            continue;
        }
        // Indented code: leave alone.
        if line.starts_with("    ") || line.starts_with('\t') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        inline(line, repo, emojis, &mut out);
        out.push('\n');
    }
    out
}

/// One line outside code blocks. Skips `code spans`, link targets and <autolinks>.
fn inline(line: &str, repo: &str, emojis: &Emojis, out: &mut String) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    // Inside [link text]: no nested links, but emoji still work.
    let mut brackets = 0usize;
    let at_word_start = |i: usize| i == 0 || matches!(chars[i - 1], ' ' | '\t' | '(' | '[' | '>' | '*' | '_' | ',' | ';' | '"' | '\'');
    while i < chars.len() {
        let c = chars[i];
        // `code span`: copy through the matching run of backticks.
        if c == '`' {
            let run = chars[i..].iter().take_while(|&&c| c == '`').count();
            let fence: String = "`".repeat(run);
            let rest: String = chars[i + run..].iter().collect();
            if let Some(end) = rest.find(&fence) {
                let n = rest[..end].chars().count();
                out.extend(&chars[i..i + run + n + run]);
                i += run + n + run;
                continue;
            }
            out.extend(&chars[i..i + run]);
            i += run;
            continue;
        }
        // ](target) and <autolink>: copy verbatim.
        if (c == '(' && i > 0 && chars[i - 1] == ']') || (c == '<' && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic())) {
            let close = if c == '(' { ')' } else { '>' };
            if let Some(n) = chars[i..].iter().position(|&x| x == close) {
                out.extend(&chars[i..=i + n]);
                i += n + 1;
                continue;
            }
        }
        match c {
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            _ => {}
        }
        let rest: String = chars[i..].iter().collect();
        // Bare links, like GitHub's autolinking.
        if brackets == 0 && at_word_start(i) && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let mut n = rest.chars().take_while(|c| !c.is_whitespace() && *c != '<').count();
            // Trailing punctuation is usually the sentence, not the link.
            while n > 0 && matches!(chars[i + n - 1], '.' | ',' | ')' | ';' | ':' | '!' | '?' | '*' | '_') {
                n -= 1;
            }
            let url: String = chars[i..i + n].iter().collect();
            out.push('<');
            out.push_str(&url);
            out.push('>');
            i += n;
            continue;
        }
        // @mention
        if c == '@' && brackets == 0 && at_word_start(i) {
            let name: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '-' || **c == '/').collect();
            let name = name.trim_end_matches(['-', '/']);
            if !name.is_empty() {
                out.push_str(&format!("[**@{name}**](https://github.com/{name})"));
                i += 1 + name.chars().count();
                continue;
            }
        }
        // owner/repo#123 and #123
        if brackets == 0
            && at_word_start(i)
            && let Some((text, target, len)) = issue_ref(&chars[i..], repo)
        {
            out.push_str(&format!("[{text}](https://github.com/{target})"));
            i += len;
            continue;
        }
        // :shortcode:
        if c == ':' {
            let name: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')).collect();
            if !name.is_empty()
                && chars.get(i + 1 + name.len()) == Some(&':')
                && let Some(url) = emojis.by_name.get(&name)
            {
                out.push_str(&emoji_image(&format!(":{name}:"), url));
                i += name.len() + 2;
                continue;
            }
        }
        // Unicode emoji, including skin tones, flags and joined sequences.
        let n = emoji_len(&chars[i..]);
        if n > 0 {
            let seq: String = chars[i..i + n].iter().collect();
            match emojis.unicode_url(&seq) {
                Some(url) => out.push_str(&emoji_image(&seq, &url)),
                None => out.push_str(&seq),
            }
            i += n;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

/// "#123" or "owner/repo#123" at the start of `s`: (link text, path, length).
fn issue_ref(s: &[char], repo: &str) -> Option<(String, String, usize)> {
    let hash = s.iter().position(|&c| c == '#')?;
    let prefix: String = s[..hash].iter().collect();
    let owner_repo = if prefix.is_empty() {
        repo.to_string()
    } else {
        let ok = prefix.split('/').count() == 2
            && prefix.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
        if !ok {
            return None;
        }
        prefix.clone()
    };
    let digits: String = s[hash + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || owner_repo.is_empty() {
        return None;
    }
    // "#12abc" isn't a reference.
    if s.get(hash + 1 + digits.len()).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
        return None;
    }
    // GitHub redirects /issues/N to /pull/N when it's a PR.
    Some((format!("{prefix}#{digits}"), format!("{owner_repo}/issues/{digits}"), hash + 1 + digits.len()))
}

/// Length (in chars) of the emoji at the start of `s`, or 0.
fn emoji_len(s: &[char]) -> usize {
    let Some(&c) = s.first() else { return 0 };
    let next = s.get(1).copied();
    // Flags: two regional indicator letters.
    if is_regional(c) {
        return if next.is_some_and(is_regional) { 2 } else { 0 };
    }
    // Keycaps: 1️⃣ #️⃣ *️⃣
    if (c.is_ascii_digit() || c == '#' || c == '*') && next == Some('\u{fe0f}') && s.get(2) == Some(&'\u{20e3}') {
        return 3;
    }
    let pictographic = matches!(c as u32, 0x1F000..=0x1FAFF) && !matches!(c as u32, 0x1F100..=0x1F1E5);
    // Symbols like ✔ ☀ ❤ are plain text unless followed by U+FE0F, except
    // those that always show as emoji.
    let emoji_style = next == Some('\u{fe0f}') && matches!(c as u32, 0x2000..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0xA9 | 0xAE);
    if !(pictographic || emoji_style || DEFAULT_EMOJI.contains(&c)) {
        return 0;
    }
    let mut n = 1;
    loop {
        match s.get(n) {
            Some('\u{fe0f}') => n += 1,
            Some(&m) if matches!(m as u32, 0x1F3FB..=0x1F3FF) => n += 1,
            // Zero-width joiner glues the next emoji on (👩‍💻).
            Some('\u{200d}') if s.get(n + 1).is_some() => n += 2,
            _ => break,
        }
    }
    n
}

fn is_regional(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

/// Symbols below U+1F000 that show as color emoji even without U+FE0F.
const DEFAULT_EMOJI: &[char] = &[
    '⌚', '⌛', '⏩', '⏪', '⏫', '⏬', '⏰', '⏳', '◽', '◾', '☔', '☕', '♈', '♉', '♊', '♋', '♌', '♍', '♎', '♏', '♐', '♑', '♒', '♓', '♿', '⚓',
    '⚡', '⚪', '⚫', '⚽', '⚾', '⛄', '⛅', '⛎', '⛔', '⛪', '⛲', '⛳', '⛵', '⛺', '⛽', '✅', '✊', '✋', '✨', '❌', '❎', '❓', '❔', '❕', '❗', '➕',
    '➖', '➗', '➰', '➿', '⬛', '⬜', '⭐', '⭕',
];

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> String {
        to_markdown(s, "o/r", &Emojis::default())
    }

    #[test]
    fn links_and_mentions() {
        assert_eq!(md("see #12 and a/b#3").trim(), "see [#12](https://github.com/o/r/issues/12) and [a/b#3](https://github.com/a/b/issues/3)");
        assert_eq!(md("hi @nate-s!").trim(), "hi [**@nate-s**](https://github.com/nate-s)!");
        assert_eq!(md("mail a@b.com").trim(), "mail a@b.com");
        assert_eq!(md("go to https://x.io/a.").trim(), "go to <https://x.io/a>.");
        assert_eq!(md("[#1](https://x/#2) `#3` <https://y>").trim(), "[#1](https://x/#2) `#3` <https://y>");
        assert_eq!(md("# Title").trim(), "# Title");
    }

    #[test]
    fn emoji() {
        let e = Emojis::new(HashMap::from([
            ("tada".to_string(), format!("{EMOJI_CDN}1f389.png?v8")),
            ("heart".to_string(), format!("{EMOJI_CDN}2764.png?v8")),
            ("robot".to_string(), format!("{EMOJI_CDN}1f916.png?v8")),
        ]));
        let r = to_markdown(":tada: 🤖 ❤️ ✔ plain", "o/r", &e);
        assert!(r.contains(&format!("![:tada:](emoji:{EMOJI_CDN}1f389.png?v8)")));
        assert!(r.contains(&format!("(emoji:{EMOJI_CDN}1f916.png)")));
        assert!(r.contains(&format!("(emoji:{EMOJI_CDN}2764.png)")));
        assert!(r.contains("✔ plain"));
    }

    #[test]
    fn code_is_untouched() {
        let r = md("```\n#1 @x :tada:\n```\n");
        assert_eq!(r, "```\n#1 @x :tada:\n```\n");
        let r = md("```suggestion\nlet x = 1;\n```\n");
        assert_eq!(r, "**Suggested change**\n\n```diff\n+ let x = 1;\n```\n");
    }
}
