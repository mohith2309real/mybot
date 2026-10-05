//! The built-in library, compiled into the binary.
//!
//! Skills are hand-written instructions — never fetched, never imported from a
//! registry. (In February 2026, 12–29% of a public agent-skill registry turned
//! out to be malware; a skill is instructions a model follows with a real
//! browser and shell, so importing one is running a stranger's code.)
//!
//! Site profiles say where a site lives and where you sign in, so the Add-login
//! screen can suggest the exact origin a saved login will be bound to.

use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

const SKILL_FILES: &[&str] = &[
    include_str!("../data/skills/research.toml"),
    include_str!("../data/skills/writing.toml"),
    include_str!("../data/skills/email.toml"),
    include_str!("../data/skills/calendar.toml"),
    include_str!("../data/skills/shopping.toml"),
    include_str!("../data/skills/travel.toml"),
    include_str!("../data/skills/finance.toml"),
    include_str!("../data/skills/files.toml"),
    include_str!("../data/skills/data.toml"),
    include_str!("../data/skills/developer.toml"),
    include_str!("../data/skills/social.toml"),
    include_str!("../data/skills/jobs.toml"),
    include_str!("../data/skills/learning.toml"),
    include_str!("../data/skills/home.toml"),
    include_str!("../data/skills/news.toml"),
    include_str!("../data/skills/health.toml"),
    include_str!("../data/skills/productivity.toml"),
    include_str!("../data/skills/web.toml"),
];

const SITES_FILE: &str = include_str!("../data/sites.toml");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    pub instructions: String,
    /// One rule per entry. `always-confirm: …` forces a human check.
    #[serde(default)]
    pub safety: Vec<String>,
    /// Site keys this skill usually works on.
    #[serde(default)]
    pub sites: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    pub key: String,
    pub name: String,
    pub category: String,
    pub url: String,
    /// The sign-in page; its origin is what a saved login is bound to.
    pub login: String,
    #[serde(default)]
    pub two_step: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Deserialize)]
struct SkillFile {
    skill: Vec<Skill>,
}

#[derive(Deserialize)]
struct SiteFile {
    sites: Vec<Site>,
}

static SKILLS: Lazy<Vec<Skill>> = Lazy::new(|| {
    let mut all = Vec::new();
    for f in SKILL_FILES {
        let parsed: SkillFile = toml::from_str(f).expect("built-in skills parse");
        for mut s in parsed.skill {
            s.instructions = s.instructions.trim().to_string();
            all.push(s);
        }
    }
    all
});

static SITES: Lazy<Vec<Site>> = Lazy::new(|| toml::from_str::<SiteFile>(SITES_FILE).expect("built-in sites parse").sites);

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

pub fn skills() -> &'static [Skill] {
    &SKILLS
}

pub fn skill(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim()))
}

pub fn skill_categories() -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for s in SKILLS.iter() {
        *m.entry(s.category.as_str()).or_insert(0) += 1;
    }
    m
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(String::from).collect()
}

/// Rank by how many query words appear in the name (weighted), category and
/// description. Good enough for a few hundred entries, and predictable.
pub fn search_skills(query: &str, limit: usize) -> Vec<&'static Skill> {
    let q = words(query);
    if q.is_empty() {
        return SKILLS.iter().take(limit).collect();
    }
    let mut scored: Vec<(usize, &Skill)> = SKILLS
        .iter()
        .filter_map(|s| {
            let name = words(&s.name);
            let rest = words(&format!("{} {} {}", s.category, s.description, s.instructions));
            let score: usize = q
                .iter()
                .map(|w| {
                    if name.iter().any(|n| n == w) {
                        5
                    } else if name.iter().any(|n| n.starts_with(w.as_str())) {
                        3
                    } else if rest.iter().any(|n| n == w || n.starts_with(w.as_str())) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score > 0).then_some((score, s))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, s)| s).collect()
}

/// The rules a skill marks `always-confirm`, and the rest.
pub fn safety_rules(rules: &[String]) -> (Vec<String>, Vec<String>) {
    let mut confirm = Vec::new();
    let mut other = Vec::new();
    for r in rules {
        let t = r.trim();
        if let Some(rest) = t.strip_prefix("always-confirm:").or_else(|| t.strip_prefix("always-confirm")) {
            confirm.push(rest.trim().to_string());
        } else if !t.is_empty() {
            other.push(t.to_string());
        }
    }
    (confirm, other)
}

/// Substitute `{{input}}` placeholders. Inputs with no value are left visible
/// as `{{input}}` so the bot can see what it was not told — and ask.
pub fn render_instructions(instructions: &str, args: &BTreeMap<String, String>) -> String {
    let mut out = instructions.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

/// The full prompt handed to a run: instructions, then the skill's rules.
pub fn render_skill(skill: &Skill, args: &BTreeMap<String, String>) -> String {
    let mut text = format!("Skill: {}\n\n{}", skill.name, render_instructions(&skill.instructions, args));
    let missing: Vec<&str> = skill.inputs.iter().filter(|i| !args.contains_key(*i)).map(String::as_str).collect();
    if !missing.is_empty() {
        text.push_str(&format!(
            "\n\nNot given: {}. If the task cannot be done sensibly without them, call request_human and ask.",
            missing.join(", ")
        ));
    }
    let (confirm, other) = safety_rules(&skill.safety);
    if !confirm.is_empty() || !other.is_empty() {
        text.push_str("\n\nSafety rules for this skill:");
        for r in &other {
            text.push_str(&format!("\n- {r}"));
        }
        for r in &confirm {
            text.push_str(&format!("\n- Always stop and get a human to confirm before: {r}"));
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Sites
// ---------------------------------------------------------------------------

pub fn sites() -> &'static [Site] {
    &SITES
}

pub fn site(key: &str) -> Option<&'static Site> {
    SITES.iter().find(|s| s.key.eq_ignore_ascii_case(key.trim()))
}

pub fn site_categories() -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for s in SITES.iter() {
        *m.entry(s.category.as_str()).or_insert(0) += 1;
    }
    m
}

fn origin_of(u: &str) -> Option<String> {
    url::Url::parse(u).ok().map(|u| u.origin().ascii_serialization())
}

fn host_of(u: &str) -> Option<String> {
    url::Url::parse(u).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
}

/// Search by name, key, category or domain.
pub fn search_sites(query: &str, limit: usize) -> Vec<&'static Site> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return SITES.iter().take(limit).collect();
    }
    let qhost = host_of(&if q.contains("://") { q.clone() } else { format!("https://{q}") }).unwrap_or_default();
    let mut scored: Vec<(usize, &Site)> = SITES
        .iter()
        .filter_map(|s| {
            let name = s.name.to_lowercase();
            let score = if s.key == q || name == q {
                10
            } else if !qhost.is_empty() && (host_of(&s.url).as_deref() == Some(&qhost) || host_of(&s.login).as_deref() == Some(&qhost)) {
                9
            } else if name.starts_with(&q) || s.key.starts_with(&q) {
                6
            } else if name.contains(&q) {
                4
            } else if s.category.to_lowercase().contains(&q) || s.url.contains(&q) {
                2
            } else {
                0
            };
            (score > 0).then_some((score, s))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, s)| s).collect()
}

/// The profile for a page the bot is on, by sign-in origin or site origin.
pub fn site_for_origin(origin: &str) -> Option<&'static Site> {
    SITES
        .iter()
        .find(|s| origin_of(&s.login).as_deref() == Some(origin))
        .or_else(|| SITES.iter().find(|s| origin_of(&s.url).as_deref() == Some(origin)))
}

/// Where to sign in for whatever the human typed ("github", "amazon.co.uk").
pub fn suggest_login_url(input: &str) -> Option<&'static str> {
    search_sites(input, 1).first().map(|s| s.login.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_least_200_of_each_and_well_formed() {
        assert!(skills().len() >= 200, "{} skills", skills().len());
        assert!(sites().len() >= 200, "{} sites", sites().len());

        let mut names = std::collections::HashSet::new();
        for s in skills() {
            assert!(names.insert(s.name.to_lowercase()), "duplicate skill {}", s.name);
            assert!(!s.instructions.is_empty() && !s.description.is_empty(), "{}", s.name);
            for i in &s.inputs {
                assert!(s.instructions.contains(&format!("{{{{{i}}}}}")), "{}: input {i} unused", s.name);
            }
            for k in &s.sites {
                assert!(site(k).is_some(), "{}: unknown site {k}", s.name);
            }
        }
        let mut keys = std::collections::HashSet::new();
        for s in sites() {
            assert!(keys.insert(s.key.clone()), "duplicate site {}", s.key);
            for u in [&s.url, &s.login] {
                assert!(u.starts_with("https://") && url::Url::parse(u).is_ok(), "{}: {u}", s.key);
            }
        }
    }

    #[test]
    fn money_and_messages_always_confirm() {
        // Every built-in skill that sends, posts, buys or pays carries an
        // always-confirm rule for it.
        for s in skills() {
            let text = s.instructions.to_lowercase();
            let (confirm, _) = safety_rules(&s.safety);
            for (verb, what) in [("press send", "send"), ("before pressing post", "publish"), ("checkout", "checkout")] {
                if text.contains(verb) {
                    assert!(!confirm.is_empty(), "{} mentions {what} without an always-confirm rule", s.name);
                }
            }
        }
    }

    #[test]
    fn search_and_render() {
        assert_eq!(search_skills("inbox triage", 3)[0].name, "Inbox triage");
        assert!(search_skills("flight", 5).iter().any(|s| s.name == "Flight search"));
        let s = skill("summarize a web page").unwrap();
        let mut args = BTreeMap::new();
        args.insert("url".to_string(), "https://example.com".to_string());
        let r = render_skill(s, &args);
        assert!(r.contains("Open https://example.com"));
        assert!(!r.contains("Not given"));
        let r = render_skill(skill("Send a prepared email").unwrap(), &BTreeMap::new());
        assert!(r.contains("Not given: to, subject, body_file"));
        assert!(r.contains("Always stop and get a human to confirm before: sending the email"));
    }

    #[test]
    fn sites_lookup() {
        assert_eq!(search_sites("github", 1)[0].key, "github");
        assert_eq!(search_sites("amazon.co.uk", 1)[0].key, "amazon-uk");
        assert_eq!(suggest_login_url("gmail"), Some("https://accounts.google.com/"));
        assert_eq!(site_for_origin("https://github.com").unwrap().key, "github");
        assert!(site_categories().len() >= 10);
    }
}
