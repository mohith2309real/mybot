//! Text actions: pure functions on strings.

use std::collections::{BTreeMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;

use crate::{Action, b_or, local, n_or, num, s, s_or};

const C: &str = "Text";
const T: (&str, &str, &str, bool) = ("text", "string", "The text", true);

fn words_of(t: &str) -> Vec<&str> {
    t.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).collect()
}

fn split_words_for_case(t: &str) -> Vec<String> {
    // "helloWorld foo_bar-baz" → [hello, world, foo, bar, baz]
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = t.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let boundary = c.is_uppercase() && i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
        if boundary && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn cap(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

static EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"'()]+[^\s<>"'().,;:!?]"#).unwrap());
static PHONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\+?\d[\d\s().-]{6,}\d").unwrap());
static NUMBER: Lazy<Regex> = Lazy::new(|| Regex::new(r"-?\d+(?:[.,]\d+)*(?:\.\d+)?").unwrap());
static HASHTAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"#[\p{L}\p{N}_]+").unwrap());
static MENTION: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:^|\s)@([A-Za-z0-9_.]{2,30})").unwrap());
static TAGS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>|<[^>]+>").unwrap());
static DATE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{4}-\d{2}-\d{2}\b|\b\d{1,2}/\d{1,2}/\d{2,4}\b|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.? \d{1,2}(?:st|nd|rd|th)?,? \d{4}\b").unwrap());

fn uniq(v: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

fn found(v: Vec<String>, what: &str) -> Result<String, String> {
    if v.is_empty() { Ok(format!("No {what} found.")) } else { Ok(format!("{} {what}:\n{}", v.len(), v.join("\n"))) }
}

pub fn html_unescape(t: &str) -> String {
    let mut out = t.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'");
    static NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"&#(x?)([0-9A-Fa-f]+);").unwrap());
    out = NUM
        .replace_all(&out, |c: &regex::Captures| {
            let v = if &c[1] == "x" { u32::from_str_radix(&c[2], 16) } else { c[2].parse() };
            v.ok().and_then(char::from_u32).map(String::from).unwrap_or_default()
        })
        .to_string();
    out.replace("&amp;", "&")
}

fn syllables(word: &str) -> usize {
    let w = word.to_lowercase();
    let mut count = 0;
    let mut prev_vowel = false;
    for c in w.chars() {
        let v = "aeiouy".contains(c);
        if v && !prev_vowel {
            count += 1;
        }
        prev_vowel = v;
    }
    if w.ends_with('e') && count > 1 && !w.ends_with("le") {
        count -= 1;
    }
    count.max(1)
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Line diff via longest common subsequence: "- old" / "+ new" / "  same".
pub fn line_diff(a: &str, b: &str) -> String {
    let x: Vec<&str> = a.lines().collect();
    let y: Vec<&str> = b.lines().collect();
    let (n, m) = (x.len(), y.len());
    let mut l = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if x[i] == y[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if x[i] == y[j] {
            out.push(format!("  {}", x[i]));
            i += 1;
            j += 1;
        } else if l[i + 1][j] >= l[i][j + 1] {
            out.push(format!("- {}", x[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", y[j]));
            j += 1;
        }
    }
    out.extend(x[i..].iter().map(|l| format!("- {l}")));
    out.extend(y[j..].iter().map(|l| format!("+ {l}")));
    out.join("\n")
}

pub fn markdown_to_html(md: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(md, pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, parser);
    out
}

pub fn defs() -> Vec<Action> {
    vec![
        local("text_stats", C, "Count characters, words, lines, sentences and paragraphs", &[T],
            |v| {
                let t = s(v, "text")?;
                let sentences = t.split(['.', '!', '?']).filter(|x| !x.trim().is_empty()).count();
                let paragraphs = t.split("\n\n").filter(|x| !x.trim().is_empty()).count();
                Ok(format!("characters: {}\nwords: {}\nlines: {}\nsentences: {sentences}\nparagraphs: {paragraphs}", t.chars().count(), words_of(t).len(), t.lines().count()))
            },
            (json!({"text": "One two. Three!\n\nFour"}), "words: 4")),
        local("text_upper", C, "Convert to UPPER CASE", &[T], |v| Ok(s(v, "text")?.to_uppercase()), (json!({"text": "hi"}), "HI")),
        local("text_lower", C, "Convert to lower case", &[T], |v| Ok(s(v, "text")?.to_lowercase()), (json!({"text": "Hi"}), "hi")),
        local("text_title_case", C, "Convert To Title Case", &[T],
            |v| Ok(s(v, "text")?.split(' ').map(cap).collect::<Vec<_>>().join(" ")),
            (json!({"text": "the quick fox"}), "The Quick Fox")),
        local("text_sentence_case", C, "Convert to Sentence case", &[T],
            |v| { let t = s(v, "text")?.to_lowercase(); Ok(cap(&t)) },
            (json!({"text": "HELLO THERE"}), "Hello there")),
        local("text_snake_case", C, "Convert to snake_case", &[T], |v| Ok(split_words_for_case(s(v, "text")?).join("_")), (json!({"text": "Hello World-wide"}), "hello_world_wide")),
        local("text_kebab_case", C, "Convert to kebab-case", &[T], |v| Ok(split_words_for_case(s(v, "text")?).join("-")), (json!({"text": "helloWorld"}), "hello-world")),
        local("text_camel_case", C, "Convert to camelCase", &[T],
            |v| { let w = split_words_for_case(s(v, "text")?); Ok(w.iter().enumerate().map(|(i, x)| if i == 0 { x.clone() } else { cap(x) }).collect()) },
            (json!({"text": "make it camel"}), "makeItCamel")),
        local("text_pascal_case", C, "Convert to PascalCase", &[T], |v| Ok(split_words_for_case(s(v, "text")?).iter().map(|x| cap(x)).collect()), (json!({"text": "make it pascal"}), "MakeItPascal")),
        local("text_slugify", C, "Make a URL slug", &[T],
            |v| {
                let t: String = s(v, "text")?.to_lowercase().chars().map(|c| match c { 'à'|'á'|'â'|'ä'|'ã'|'å' => 'a', 'è'|'é'|'ê'|'ë' => 'e', 'ì'|'í'|'î'|'ï' => 'i', 'ò'|'ó'|'ô'|'ö'|'õ' => 'o', 'ù'|'ú'|'û'|'ü' => 'u', 'ñ' => 'n', 'ç' => 'c', c => c }).collect();
                Ok(t.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-"))
            },
            (json!({"text": "Café & Crème: Day 1!"}), "cafe-creme-day-1")),
        local("text_trim", C, "Trim whitespace at both ends of every line", &[T],
            |v| Ok(s(v, "text")?.lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string()),
            (json!({"text": "  a  \n  b "}), "a\nb")),
        local("text_reverse", C, "Reverse the characters", &[T], |v| Ok(s(v, "text")?.chars().rev().collect()), (json!({"text": "abc"}), "cba")),
        local("text_dedupe_lines", C, "Remove duplicate lines, keeping first occurrences", &[T],
            |v| { let mut seen = HashSet::new(); Ok(s(v, "text")?.lines().filter(|l| seen.insert(*l)).collect::<Vec<_>>().join("\n")) },
            (json!({"text": "a\nb\na"}), "a\nb")),
        local("text_sort_lines", C, "Sort lines (alphabetically, numerically, or reversed)", &[T, ("numeric", "boolean", "Sort as numbers", false), ("reverse", "boolean", "Descending", false)],
            |v| {
                let mut lines: Vec<&str> = s(v, "text")?.lines().collect();
                if b_or(v, "numeric", false) {
                    lines.sort_by(|a, b| a.trim().parse::<f64>().unwrap_or(f64::MAX).partial_cmp(&b.trim().parse::<f64>().unwrap_or(f64::MAX)).unwrap_or(std::cmp::Ordering::Equal));
                } else {
                    lines.sort_by_key(|l| l.to_lowercase());
                }
                if b_or(v, "reverse", false) { lines.reverse(); }
                Ok(lines.join("\n"))
            },
            (json!({"text": "10\n9\n100", "numeric": true}), "9\n10\n100")),
        local("text_number_lines", C, "Prefix every line with its number", &[T],
            |v| Ok(s(v, "text")?.lines().enumerate().map(|(i, l)| format!("{:>4}  {l}", i + 1)).collect::<Vec<_>>().join("\n")),
            (json!({"text": "a\nb"}), "   2  b")),
        local("text_remove_blank_lines", C, "Remove empty lines", &[T],
            |v| Ok(s(v, "text")?.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")),
            (json!({"text": "a\n\n\nb"}), "a\nb")),
        local("text_truncate", C, "Cut text to a maximum length, adding an ellipsis", &[T, ("max", "integer", "Maximum characters", true)],
            |v| { let t = s(v, "text")?; let m = n_or(v, "max", 100.0) as usize; Ok(if t.chars().count() <= m { t.into() } else { format!("{}…", t.chars().take(m.saturating_sub(1)).collect::<String>()) }) },
            (json!({"text": "abcdefgh", "max": 4}), "abc…")),
        local("text_wrap", C, "Wrap text to a line width", &[T, ("width", "integer", "Line width (default 80)", false)],
            |v| {
                let width = n_or(v, "width", 80.0) as usize;
                let mut out = Vec::new();
                for para in s(v, "text")?.split('\n') {
                    let mut line = String::new();
                    for w in para.split_whitespace() {
                        if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > width { out.push(std::mem::take(&mut line)); }
                        if !line.is_empty() { line.push(' '); }
                        line.push_str(w);
                    }
                    out.push(line);
                }
                Ok(out.join("\n"))
            },
            (json!({"text": "aaa bbb ccc", "width": 7}), "aaa bbb\nccc")),
        local("text_indent", C, "Indent every line", &[T, ("spaces", "integer", "Spaces (default 2)", false)],
            |v| { let p = " ".repeat(n_or(v, "spaces", 2.0) as usize); Ok(s(v, "text")?.lines().map(|l| format!("{p}{l}")).collect::<Vec<_>>().join("\n")) },
            (json!({"text": "a", "spaces": 4}), "    a")),
        local("text_find_replace", C, "Replace every occurrence of a phrase", &[T, ("find", "string", "Text to find", true), ("replace", "string", "Replacement", true), ("ignore_case", "boolean", "Case-insensitive", false)],
            |v| {
                let (t, f, r) = (s(v, "text")?, s(v, "find")?, s(v, "replace")?);
                if f.is_empty() { return Err("\"find\" is empty".into()); }
                if b_or(v, "ignore_case", false) {
                    let re = Regex::new(&format!("(?i){}", regex::escape(f))).map_err(|e| e.to_string())?;
                    Ok(re.replace_all(t, regex::NoExpand(r)).to_string())
                } else { Ok(t.replace(f, r)) }
            },
            (json!({"text": "Cat cat", "find": "cat", "replace": "dog", "ignore_case": true}), "dog dog")),
        local("regex_find", C, "Find all regex matches (with capture groups)", &[T, ("pattern", "string", "Regular expression", true)],
            |v| {
                let re = Regex::new(s(v, "pattern")?).map_err(|e| format!("bad pattern: {e}"))?;
                let t = s(v, "text")?;
                let rows: Vec<String> = re.captures_iter(t).take(500).map(|c| {
                    let groups: Vec<String> = c.iter().skip(1).map(|g| g.map(|m| m.as_str().to_string()).unwrap_or_default()).collect();
                    if groups.is_empty() { c[0].to_string() } else { format!("{}  ⟶ {}", &c[0], groups.join(" | ")) }
                }).collect();
                found(rows, "matches")
            },
            (json!({"text": "a1 b22", "pattern": "([a-z])(\\d+)"}), "b22  ⟶ b | 22")),
        local("regex_replace", C, "Replace regex matches ($1 for groups)", &[T, ("pattern", "string", "Regular expression", true), ("replace", "string", "Replacement, $1 for group 1", true)],
            |v| { let re = Regex::new(s(v, "pattern")?).map_err(|e| format!("bad pattern: {e}"))?; Ok(re.replace_all(s(v, "text")?, s(v, "replace")?).to_string()) },
            (json!({"text": "2026-10-05", "pattern": "(\\d+)-(\\d+)-(\\d+)", "replace": "$3/$2/$1"}), "05/10/2026")),
        local("regex_test", C, "Test which lines match a regex", &[T, ("pattern", "string", "Regular expression", true)],
            |v| { let re = Regex::new(s(v, "pattern")?).map_err(|e| format!("bad pattern: {e}"))?; Ok(s(v, "text")?.lines().map(|l| format!("{} {l}", if re.is_match(l) { "✓" } else { "✗" })).collect::<Vec<_>>().join("\n")) },
            (json!({"text": "abc\n123", "pattern": "^\\d+$"}), "✓ 123")),
        local("extract_emails", C, "Pull out every email address", &[T],
            |v| found(uniq(EMAIL.find_iter(s(v, "text")?).map(|m| m.as_str().to_string()).collect()), "email addresses"),
            (json!({"text": "write a@b.co or x.y@example.org"}), "x.y@example.org")),
        local("extract_urls", C, "Pull out every URL", &[T],
            |v| found(uniq(URL_RE.find_iter(s(v, "text")?).map(|m| m.as_str().to_string()).collect()), "URLs"),
            (json!({"text": "see https://example.com/a?b=1, and http://x.org."}), "http://x.org")),
        local("extract_phone_numbers", C, "Pull out phone-number-like strings", &[T],
            |v| found(uniq(PHONE.find_iter(s(v, "text")?).map(|m| m.as_str().trim().to_string()).filter(|p| p.chars().filter(|c| c.is_ascii_digit()).count() >= 7).collect()), "phone numbers"),
            (json!({"text": "call +1 (555) 123-4567 now"}), "+1 (555) 123-4567")),
        local("extract_numbers", C, "Pull out every number", &[T],
            |v| found(NUMBER.find_iter(s(v, "text")?).map(|m| m.as_str().to_string()).collect(), "numbers"),
            (json!({"text": "3 apples cost 4.50 and -2"}), "4.50")),
        local("extract_hashtags", C, "Pull out #hashtags", &[T],
            |v| found(uniq(HASHTAG.find_iter(s(v, "text")?).map(|m| m.as_str().to_string()).collect()), "hashtags"),
            (json!({"text": "loving #rust and #MyBot"}), "#MyBot")),
        local("extract_mentions", C, "Pull out @mentions", &[T],
            |v| found(uniq(MENTION.captures_iter(s(v, "text")?).map(|c| format!("@{}", &c[1])).collect()), "mentions"),
            (json!({"text": "thanks @alice and @bob_9"}), "@bob_9")),
        local("extract_dates", C, "Pull out date-like strings", &[T],
            |v| found(uniq(DATE_RE.find_iter(s(v, "text")?).map(|m| m.as_str().to_string()).collect()), "dates"),
            (json!({"text": "due 2026-11-01 or March 3rd, 2027"}), "March 3rd, 2027")),
        local("word_frequency", C, "Most common words (ignoring common stop words)", &[T, ("top", "integer", "How many (default 20)", false)],
            |v| {
                const STOP: &[&str] = &["the","a","an","and","or","of","to","in","on","for","is","are","was","it","that","this","with","as","at","by","be","from"];
                let mut m: BTreeMap<String, usize> = BTreeMap::new();
                for w in words_of(s(v, "text")?) { let w = w.to_lowercase(); if !STOP.contains(&w.as_str()) { *m.entry(w).or_default() += 1; } }
                let mut v2: Vec<_> = m.into_iter().collect();
                v2.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                Ok(v2.into_iter().take(n_or(v, "top", 20.0) as usize).map(|(w, c)| format!("{c:>5}  {w}")).collect::<Vec<_>>().join("\n"))
            },
            (json!({"text": "the cat and the cat sat"}), "    2  cat")),
        local("text_diff", C, "Line-by-line difference between two texts", &[("a", "string", "Old text", true), ("b", "string", "New text", true)],
            |v| Ok(line_diff(s(v, "a")?, s(v, "b")?)),
            (json!({"a": "x\ny", "b": "x\nz"}), "+ z")),
        local("text_similarity", C, "Edit distance and similarity between two strings", &[("a", "string", "First", true), ("b", "string", "Second", true)],
            |v| { let (a, b) = (s(v, "a")?, s(v, "b")?); let d = levenshtein(a, b); let m = a.chars().count().max(b.chars().count()).max(1); Ok(format!("edit distance: {d}\nsimilarity: {:.0}%", 100.0 * (1.0 - d as f64 / m as f64))) },
            (json!({"a": "kitten", "b": "sitting"}), "edit distance: 3")),
        local("text_split", C, "Split text by a separator into numbered parts", &[T, ("separator", "string", "Separator (default newline)", false)],
            |v| { let sep = s_or(v, "separator", "\n"); Ok(s(v, "text")?.split(sep).enumerate().map(|(i, p)| format!("[{}] {p}", i + 1)).collect::<Vec<_>>().join("\n")) },
            (json!({"text": "a,b", "separator": ","}), "[2] b")),
        local("text_join_lines", C, "Join lines with a separator", &[T, ("separator", "string", "Separator (default \", \")", false)],
            |v| Ok(s(v, "text")?.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(s_or(v, "separator", ", "))),
            (json!({"text": "a\nb\nc"}), "a, b, c")),
        local("html_strip_tags", C, "Strip HTML tags (and scripts/styles) to plain text", &[("html", "string", "HTML", true)],
            |v| { let t = TAGS.replace_all(s(v, "html")?, " "); Ok(html_unescape(&t).split_whitespace().collect::<Vec<_>>().join(" ")) },
            (json!({"html": "<p>Hi <b>there</b></p><script>x()</script>"}), "Hi there")),
        local("html_escape", C, "Escape text for safe inclusion in HTML", &[T],
            |v| Ok(s(v, "text")?.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")),
            (json!({"text": "<a href=\"x\">"}), "&lt;a href=&quot;x&quot;&gt;")),
        local("html_unescape", C, "Turn HTML entities back into characters", &[T], |v| Ok(html_unescape(s(v, "text")?)), (json!({"text": "a &amp; b &#169; &#x41;"}), "a & b © A")),
        local("markdown_to_html", C, "Convert Markdown to HTML", &[("markdown", "string", "Markdown", true)],
            |v| Ok(markdown_to_html(s(v, "markdown")?)),
            (json!({"markdown": "# Title\n\n**bold** item"}), "<h1>Title</h1>")),
        local("markdown_toc", C, "Build a table of contents from Markdown headings", &[("markdown", "string", "Markdown", true)],
            |v| {
                let toc: Vec<String> = s(v, "markdown")?.lines().filter_map(|l| {
                    let level = l.chars().take_while(|c| *c == '#').count();
                    (level > 0 && level <= 6 && l[level..].starts_with(' ')).then(|| {
                        let title = l[level..].trim();
                        let anchor: String = title.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect::<String>().replace(' ', "-");
                        format!("{}- [{title}](#{anchor})", "  ".repeat(level - 1))
                    })
                }).collect();
                Ok(toc.join("\n"))
            },
            (json!({"markdown": "# A\n## B c\ntext"}), "  - [B c](#b-c)")),
        local("text_readability", C, "Flesch reading ease and grade level", &[T],
            |v| {
                let t = s(v, "text")?;
                let w = words_of(t);
                let sentences = t.split(['.', '!', '?']).filter(|x| !x.trim().is_empty()).count().max(1) as f64;
                let words = w.len().max(1) as f64;
                let syl: usize = w.iter().map(|x| syllables(x)).sum();
                let ease = 206.835 - 1.015 * (words / sentences) - 84.6 * (syl as f64 / words);
                let grade = 0.39 * (words / sentences) + 11.8 * (syl as f64 / words) - 15.59;
                Ok(format!("reading ease: {ease:.0} (higher is easier)\ngrade level: {:.1}", grade.max(0.0)))
            },
            (json!({"text": "The cat sat. The dog ran."}), "reading ease:")),
        local("text_lorem_ipsum", C, "Placeholder text of a given number of words", &[("words", "integer", "How many words (default 50)", false)],
            |v| {
                const L: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat";
                let w: Vec<&str> = L.split(' ').collect();
                let count = n_or(v, "words", 50.0) as usize;
                let mut out: String = (0..count).map(|i| w[i % w.len()]).collect::<Vec<_>>().join(" ");
                out = cap(&out); out.push('.');
                Ok(out)
            },
            (json!({"words": 3}), "Lorem ipsum dolor.")),
        local("text_count_occurrences", C, "Count how often a phrase appears", &[T, ("phrase", "string", "Phrase", true)],
            |v| { let p = s(v, "phrase")?; if p.is_empty() { return Err("empty phrase".into()); } Ok(num(s(v, "text")?.to_lowercase().matches(&p.to_lowercase()).count() as f64)) },
            (json!({"text": "Ha ha HA", "phrase": "ha"}), "3")),
        local("text_char_frequency", C, "Count each character", &[T],
            |v| { let mut m: BTreeMap<char, usize> = BTreeMap::new(); for c in s(v, "text")?.chars().filter(|c| !c.is_whitespace()) { *m.entry(c).or_default() += 1; } Ok(m.into_iter().map(|(c, n)| format!("{c}: {n}")).collect::<Vec<_>>().join("\n")) },
            (json!({"text": "aab"}), "a: 2")),
    ]
}
