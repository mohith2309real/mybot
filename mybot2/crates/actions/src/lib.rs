//! The action library: everything a bot can do beyond its core tools.
//!
//! A model handed 200 tools at once picks worse and costs more on every turn,
//! so the bot sees a small core set plus two tools — `actions_search` and
//! `action_run` — and looks up the rest when it needs them.
//!
//! Kinds of action:
//!   - Local: pure computation in this process (text, data, maths, dates,
//!     hashing). No filesystem, no network.
//!   - Shell: a command run inside the bot's container, in /workspace.
//!   - Browser: driven over CDP, through the same boundary checks as the core
//!     browser tools.
//!   - Meta: about the run itself (notes, memory, the skill and site library).
//!
//! Every Local action carries a runnable example that the test suite executes,
//! so "200+ actions" means 200+ actions that are checked, not a list of names.

mod browser_actions;
mod local_crypto;
mod local_data;
mod local_math;
mod local_text;
mod local_time;
mod meta;
mod shell;
pub mod skills_store;
pub mod toolbox;

use mybot_core::policy::Capability;
use once_cell::sync::Lazy;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Local,
    Shell,
    Browser,
    Meta,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Local => "local",
            Kind::Shell => "computer",
            Kind::Browser => "browser",
            Kind::Meta => "meta",
        }
    }
}

pub type LocalFn = fn(&Value) -> Result<String, String>;

#[derive(Clone)]
pub enum Handler {
    Local(LocalFn),
    /// Builds the bash command to run in /workspace.
    Shell(LocalFn),
    /// Dispatched by name in `browser_actions::run`.
    Browser,
    /// Dispatched by name in `toolbox`.
    Meta,
}

#[derive(Clone)]
pub struct Action {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub params: Value,
    pub handler: Handler,
    /// Needs a human's OK under the run's policy unless the instruction granted it.
    pub capability: Option<Capability>,
    /// Local actions: an input and a substring the output must contain.
    pub example: Option<(Value, &'static str)>,
}

impl Action {
    pub fn kind(&self) -> Kind {
        match self.handler {
            Handler::Local(_) => Kind::Local,
            Handler::Shell(_) => Kind::Shell,
            Handler::Browser => Kind::Browser,
            Handler::Meta => Kind::Meta,
        }
    }

    /// One line for search results: name(params) — description.
    pub fn signature(&self) -> String {
        let props = self.params["properties"].as_object();
        let required: Vec<&str> = self.params["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let args: Vec<String> = props
            .into_iter()
            .flatten()
            .map(|(k, v)| {
                let ty = v["type"].as_str().unwrap_or("any");
                if required.contains(&k.as_str()) { format!("{k}: {ty}") } else { format!("{k}?: {ty}") }
            })
            .collect();
        format!("{}({}) — {}", self.name, args.join(", "), self.description)
    }
}

/// Param spec: (name, JSON type, description, required).
pub type P = (&'static str, &'static str, &'static str, bool);

pub fn schema(params: &[P]) -> Value {
    let mut props = Map::new();
    let mut req = Vec::new();
    for (name, ty, desc, required) in params {
        let mut p = json!({"type": ty, "description": desc});
        if *ty == "array" {
            p["items"] = json!({});
        }
        props.insert((*name).into(), p);
        if *required {
            req.push(json!(name));
        }
    }
    json!({"type": "object", "properties": props, "required": req})
}

pub(crate) fn local(name: &'static str, category: &'static str, description: &'static str, params: &[P], f: LocalFn, example: (Value, &'static str)) -> Action {
    Action { name, category, description, params: schema(params), handler: Handler::Local(f), capability: None, example: Some(example) }
}

pub(crate) fn shell(name: &'static str, category: &'static str, description: &'static str, params: &[P], f: LocalFn) -> Action {
    Action { name, category, description, params: schema(params), handler: Handler::Shell(f), capability: None, example: None }
}

pub(crate) fn browser(name: &'static str, description: &'static str, params: &[P]) -> Action {
    Action { name, category: "Browser", description, params: schema(params), handler: Handler::Browser, capability: None, example: None }
}

pub(crate) fn meta(name: &'static str, category: &'static str, description: &'static str, params: &[P]) -> Action {
    Action { name, category, description, params: schema(params), handler: Handler::Meta, capability: None, example: None }
}

pub(crate) fn with_capability(mut a: Action, c: Capability) -> Action {
    a.capability = Some(c);
    a
}

// --- argument helpers --------------------------------------------------------

pub(crate) fn s<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v.get(k).and_then(Value::as_str).ok_or_else(|| format!("missing \"{k}\" (text)"))
}

pub(crate) fn s_or<'a>(v: &'a Value, k: &str, d: &'a str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or(d)
}

pub(crate) fn n(v: &Value, k: &str) -> Result<f64, String> {
    match v.get(k) {
        Some(Value::Number(x)) => x.as_f64().ok_or_else(|| format!("\"{k}\" is not a number")),
        Some(Value::String(t)) => t.trim().parse().map_err(|_| format!("\"{k}\" is not a number")),
        _ => Err(format!("missing \"{k}\" (number)")),
    }
}

pub(crate) fn n_or(v: &Value, k: &str, d: f64) -> f64 {
    n(v, k).unwrap_or(d)
}

pub(crate) fn b_or(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}

pub(crate) fn list(v: &Value, k: &str) -> Result<Vec<Value>, String> {
    match v.get(k) {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(t)) => match serde_json::from_str::<Value>(t) {
            Ok(Value::Array(a)) => Ok(a),
            _ => Ok(t.split(',').map(|x| json!(x.trim())).collect()),
        },
        _ => Err(format!("missing \"{k}\" (list)")),
    }
}

pub(crate) fn numbers(v: &Value, k: &str) -> Result<Vec<f64>, String> {
    list(v, k)?
        .iter()
        .map(|x| match x {
            Value::Number(n) => n.as_f64().ok_or_else(|| "not a number".to_string()),
            Value::String(t) => t.trim().parse::<f64>().map_err(|_| format!("\"{t}\" is not a number")),
            _ => Err("not a number".into()),
        })
        .collect()
}

/// Format a float without a trailing ".0" for whole numbers.
pub(crate) fn num(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        let s = format!("{x:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// --- the registry ------------------------------------------------------------

static REGISTRY: Lazy<Vec<Action>> = Lazy::new(|| {
    let mut all = Vec::new();
    all.extend(browser_actions::defs());
    all.extend(shell::defs());
    all.extend(local_text::defs());
    all.extend(local_data::defs());
    all.extend(local_crypto::defs());
    all.extend(local_math::defs());
    all.extend(local_time::defs());
    all.extend(meta::defs());
    all
});

pub fn all() -> &'static [Action] {
    &REGISTRY
}

pub fn get(name: &str) -> Option<&'static Action> {
    REGISTRY.iter().find(|a| a.name == name.trim())
}

pub fn categories() -> Vec<(&'static str, usize)> {
    let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
    for a in REGISTRY.iter() {
        *m.entry(a.category).or_default() += 1;
    }
    m.into_iter().collect()
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// Rank actions for a free-text need ("resize image", "csv to json").
pub fn search(query: &str, category: Option<&str>, limit: usize) -> Vec<&'static Action> {
    let q = words(query);
    let mut scored: Vec<(usize, &Action)> = REGISTRY
        .iter()
        .filter(|a| category.is_none_or(|c| a.category.eq_ignore_ascii_case(c)))
        .map(|a| {
            let name = words(&a.name.replace('_', " "));
            let desc = words(&format!("{} {}", a.description, a.category));
            let score: usize = q
                .iter()
                .map(|w| {
                    if name.iter().any(|n| n == w) {
                        6
                    } else if name.iter().any(|n| n.starts_with(w.as_str()) || w.starts_with(n.as_str()) && n.len() > 2) {
                        3
                    } else if desc.iter().any(|d| d == w) {
                        2
                    } else if desc.iter().any(|d| d.starts_with(w.as_str()) && w.len() > 2) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score, a)
        })
        .filter(|(sc, _)| q.is_empty() || *sc > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(b.1.name)));
    scored.into_iter().take(limit).map(|(_, a)| a).collect()
}

/// Run a Local action. (Shell/Browser/Meta need the toolbox.)
pub fn run_local(name: &str, args: &Value) -> Result<String, String> {
    match get(name).map(|a| &a.handler) {
        Some(Handler::Local(f)) => f(args),
        Some(_) => Err(format!("{name} needs the computer; call it through action_run")),
        None => Err(format!("No action named {name}. Use actions_search.")),
    }
}

/// Quote a string for bash.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_200_unique_well_formed_actions() {
        let all = all();
        assert!(all.len() >= 200, "only {} actions", all.len());
        let mut seen = std::collections::HashSet::new();
        for a in all {
            assert!(seen.insert(a.name), "duplicate action {}", a.name);
            assert!(a.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "bad name {}", a.name);
            assert!(!a.description.is_empty(), "{}", a.name);
            assert_eq!(a.params["type"], "object", "{}", a.name);
            if matches!(a.handler, Handler::Local(_)) {
                assert!(a.example.is_some(), "{} has no example", a.name);
            }
        }
    }

    #[test]
    fn every_local_example_runs() {
        let mut failures = Vec::new();
        for a in all() {
            if let (Handler::Local(f), Some((input, expect))) = (&a.handler, &a.example) {
                match f(input) {
                    Ok(out) if out.contains(expect) => {}
                    Ok(out) => failures.push(format!("{}: expected {expect:?} in {out:?}", a.name)),
                    Err(e) => failures.push(format!("{}: error {e}", a.name)),
                }
            }
        }
        assert!(failures.is_empty(), "{} failing examples:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn search_finds_the_obvious_one() {
        for (q, want) in [
            ("csv to json", "csv_to_json"),
            ("sha256 hash", "hash_sha256"),
            ("open new tab", "tab_new"),
            ("days between dates", "date_diff"),
            ("convert units", "unit_convert"),
            ("extract emails", "extract_emails"),
            ("download a file", "download_url"),
            ("take a screenshot", "page_screenshot"),
        ] {
            let got: Vec<&str> = search(q, None, 5).iter().map(|a| a.name).collect();
            assert!(got.contains(&want), "{q:?} → {got:?}");
        }
    }

    #[test]
    fn shell_commands_quote_their_inputs() {
        let a = get("file_read").unwrap();
        let Handler::Shell(f) = a.handler else { panic!() };
        let cmd = f(&json!({"path": "notes'; rm -rf / #.txt"})).unwrap();
        assert!(cmd.contains(r"'notes'\''; rm -rf / #.txt'"), "{cmd}");
    }
}
