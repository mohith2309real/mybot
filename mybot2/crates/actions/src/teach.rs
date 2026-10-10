//! Teach a task: the human demonstrates a workflow in the bot's own browser
//! (through the live view), and a model turns the recording into a skill.
//!
//! Secrets never leave the page. The recorder classifies and redacts inside
//! the browser, so a password, one-time code or card number is never handed
//! to MyBot at all; a second pass here re-checks every action before it is
//! stored or shown to a model. Over-redaction is the deliberate failure mode.
//!
//! A recording stops by itself after ten minutes.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mybot_computer::browser::Browser;
use mybot_core::db::{Db, Recording, SkillRow};
use mybot_core::model::{ChatOptions, Message, Part, ToolDef};
use mybot_core::providers::Provider;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_RECORDING: Duration = Duration::from_secs(10 * 60);

const BINDING: &str = "__mybotTeachEmit";
const STATE_KEY: &str = "__mybotTeach";

/// Field hints that mean "whatever is typed here is a secret". Shared with the
/// in-page recorder so there is one definition of sensitive, and applied
/// again here as defence in depth.
const SENSITIVE_HINT: &str = r"pass(word|code|phrase)|passwd|\bpwd\b|\botp\b|one[-_ ]?time|2fa|\bmfa\b|two[-_ ]?factor|auth(entication)?[-_ ]?code|verif(y|ication)[-_ ]?code|security[-_ ]?code|confirm(ation)?[-_ ]?code|sms[-_ ]?code|\bcvv\b|\bcvc\b|\bcsc\b|card[-_ ]?number|cardnum|credit[-_ ]?card|\bpin\b|secret|\btoken\b|api[-_ ]?key|private[-_ ]?key|seed[-_ ]?phrase|mnemonic|\bssn\b|social[-_ ]?security|routing[-_ ]?number|account[-_ ]?number|\biban\b|passport|security[-_ ]?answer";

/// Query-string keys that carry credentials. A `method=get` form puts every
/// field in the URL, so short field names (`pw`, `cc`) matter as much as labels.
const SENSITIVE_PARAM_ALIASES: &str = "pw|pass|passwd|pwd|cc|ccnum|ccnumber|card|cardno|cvv|cvc|csc|pin|ssn|otp|totp|mfa|code|token|secret|key|sig|signature|session|auth|credential|state|nonce";

static HINT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(&format!("(?i){SENSITIVE_HINT}")).unwrap());
static PARAM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(&format!("(?i)(^|[_-])({SENSITIVE_PARAM_ALIASES})([_-]|$)|{SENSITIVE_HINT}")).unwrap());
static HASH_TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(access_token|id_token|[?&#]code=)").unwrap());
static MARKER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^<redacted:[a-z-]+>$").unwrap());
static TOKENISH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9_-]{24,}$").unwrap());

/// Runs in the page. `__CONFIG__` is replaced with the binding, state key and
/// pattern. Idempotent: a second install re-arms the existing state.
const RECORDER_JS: &str = r#"(function (cfg) {
  var w = window;
  var already = w[cfg.stateKey];
  if (already && already.installed) { already.on = true; return; }
  var state = { installed: true, on: true, queue: [], seq: 0, docId: Math.random().toString(36).slice(2, 10) };
  w[cfg.stateKey] = state;
  var sensitive = new RegExp(cfg.sensitivePattern, 'i');
  var lastValue = new WeakMap();
  var pendingEl = null, pendingTimer = 0, lastScrollAt = 0, lastScrollY = 0;
  var clean = function (s) { return (s || '').slice(0, 400).replace(/\s+/g, ' ').trim().slice(0, 120); };
  var emit = function (a) {
    if (!state.on) return;
    a.id = state.docId + ':' + (++state.seq);
    a.at = new Date().toISOString();
    a.url = location.href;
    var send = w[cfg.binding];
    if (typeof send === 'function') {
      try { send(JSON.stringify(a)); return; } catch (e) { /* fall through to the queue */ }
    }
    state.queue.push(a);
    if (state.queue.length > 500) state.queue.splice(0, state.queue.length - 500);
  };
  var labelOf = function (el) {
    var fromLabel = el.labels && el.labels.length ? el.labels[0].innerText : '';
    var ref = el.getAttribute('aria-labelledby');
    var referenced = ref ? document.getElementById(ref.split(' ')[0]) : null;
    var tag = el.tagName.toLowerCase();
    var type = (el.type || '').toLowerCase();
    // A button's value is its caption; every other control's value is user data.
    var caption = tag === 'button' || (tag === 'input' && ['submit', 'button', 'reset'].indexOf(type) >= 0) ? (el.value || '') : '';
    var text = (tag === 'input' || tag === 'textarea' || tag === 'select') ? '' : el.innerText;
    return clean(el.getAttribute('aria-label')) || clean(fromLabel) || clean(referenced ? referenced.innerText : '') ||
      clean(el.getAttribute('placeholder')) || clean(el.getAttribute('title')) || clean(el.getAttribute('alt')) ||
      clean(text) || clean(caption) || clean(el.getAttribute('name')) || '';
  };
  var selectorOf = function (el) {
    var esc = function (s) { return s.replace(/["\\]/g, '\\$&'); };
    var tag = el.tagName.toLowerCase();
    var id = el.getAttribute('id');
    if (id && !/\d{4,}|:r[0-9a-z]+:/i.test(id)) return '#' + id;
    var testId = el.getAttribute('data-testid') || el.getAttribute('data-test-id');
    if (testId) return '[data-testid="' + esc(testId) + '"]';
    var name = el.getAttribute('name');
    if (name) return tag + '[name="' + esc(name) + '"]';
    var aria = el.getAttribute('aria-label');
    if (aria) return tag + '[aria-label="' + esc(aria) + '"]';
    var parts = [], node = el;
    for (var depth = 0; node && depth < 4; depth++) {
      var current = node, part = current.tagName.toLowerCase(), parent = current.parentElement;
      if (parent) {
        var sibs = Array.prototype.filter.call(parent.children, function (c) { return c.tagName === current.tagName; });
        if (sibs.length > 1) part += ':nth-of-type(' + (sibs.indexOf(current) + 1) + ')';
      }
      parts.unshift(part);
      node = parent;
    }
    return parts.join(' > ');
  };
  var sensitivityOf = function (el) {
    var type = (el.type || '').toLowerCase();
    if (type === 'password') return 'password';
    var auto = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (auto.indexOf('password') >= 0) return 'password';
    if (auto.indexOf('one-time-code') >= 0) return 'one-time-code';
    if (auto.indexOf('cc-number') >= 0) return 'card-number';
    if (auto.indexOf('cc-csc') >= 0) return 'card-security-code';
    if (auto.indexOf('cc-') === 0) return 'card-detail';
    var hint = [el.getAttribute('name'), el.getAttribute('id'), el.getAttribute('placeholder'), el.getAttribute('aria-label'),
      el.getAttribute('title'), el.getAttribute('data-testid'), labelOf(el)].join(' ');
    if (sensitive.test(hint)) return 'sensitive';
    var inputMode = (el.getAttribute('inputmode') || '').toLowerCase();
    var maxLen = typeof el.maxLength === 'number' && el.maxLength > 0 ? el.maxLength : 0;
    if ((inputMode === 'numeric' || type === 'tel' || type === 'number') && maxLen > 0 && maxLen <= 8) return 'short-numeric';
    return '';
  };
  var describe = function (el) {
    var t = { description: labelOf(el), tag: el.tagName.toLowerCase(), selector: selectorOf(el) };
    if (el.type) t.type = String(el.type).toLowerCase();
    return t;
  };
  var readValue = function (el) {
    var reason = sensitivityOf(el);
    if (reason) return { value: '<redacted:' + reason + '>', redacted: true, reason: reason };
    var raw = el.value !== undefined && el.value !== null ? String(el.value) : (el.innerText || '');
    if (/^\d{12,}$/.test(raw.replace(/[\s-]/g, ''))) return { value: '<redacted:long-number>', redacted: true, reason: 'long-number' };
    return { value: raw.slice(0, 200), redacted: false, reason: '' };
  };
  var isTextEntry = function (el) {
    var tag = el.tagName.toLowerCase();
    if (tag === 'textarea' || tag === 'select') return true;
    if (tag === 'input') return ['submit', 'button', 'reset', 'file', 'image'].indexOf((el.type || 'text').toLowerCase()) < 0;
    return el.isContentEditable === true;
  };
  var flushPending = function () {
    if (pendingTimer) { clearTimeout(pendingTimer); pendingTimer = 0; }
    var el = pendingEl;
    pendingEl = null;
    if (!el || !el.isConnected) return;
    var r = readValue(el);
    if (!r.value || lastValue.get(el) === r.value) return;
    lastValue.set(el, r.value);
    var a = { kind: el.tagName.toLowerCase() === 'select' ? 'select' : 'input', target: describe(el), value: r.value };
    if (r.redacted) { a.redacted = true; a.detail = 'redacted (' + r.reason + ')'; }
    emit(a);
  };
  var noteInput = function (el) {
    if (pendingEl && pendingEl !== el) flushPending();
    pendingEl = el;
    if (pendingTimer) clearTimeout(pendingTimer);
    pendingTimer = setTimeout(flushPending, 900);
  };
  var CLICKABLE = 'a,button,input,select,textarea,label,summary,option,[role=button],[role=link],[role=tab],[role=menuitem],[role=option],[role=checkbox],[contenteditable=true]';
  document.addEventListener('click', function (ev) {
    var raw = ev.target;
    if (!raw || !raw.tagName) return;
    var el = (raw.closest && raw.closest(CLICKABLE)) || raw;
    flushPending();
    var a = { kind: 'click', target: describe(el) };
    var href = el.getAttribute && el.getAttribute('href');
    if (href) a.detail = 'href ' + href.slice(0, 200);
    emit(a);
  }, true);
  document.addEventListener('input', function (ev) { var el = ev.target; if (el && el.tagName && isTextEntry(el)) noteInput(el); }, true);
  document.addEventListener('change', function (ev) {
    var el = ev.target;
    if (!el || !el.tagName || !isTextEntry(el)) return;
    pendingEl = el;
    flushPending();
  }, true);
  document.addEventListener('focusout', function (ev) { if (ev.target === pendingEl) flushPending(); }, true);
  document.addEventListener('submit', function (ev) { flushPending(); emit({ kind: 'submit', target: ev.target ? describe(ev.target) : undefined }); }, true);
  document.addEventListener('keydown', function (ev) {
    // Named keys only, never characters: keystrokes would rebuild a password.
    if (['Enter', 'Escape', 'Tab'].indexOf(ev.key) < 0) return;
    if (ev.key === 'Enter') flushPending();
    var el = ev.target;
    emit({ kind: 'key', detail: ev.key, target: el && el.tagName ? describe(el) : undefined });
  }, true);
  window.addEventListener('scroll', function () {
    var now = Date.now();
    if (now - lastScrollAt < 1000) return;
    lastScrollAt = now;
    var y = window.scrollY;
    if (Math.abs(y - lastScrollY) < 40) return;
    var dir = y > lastScrollY ? 'down' : 'up';
    lastScrollY = y;
    emit({ kind: 'scroll', detail: dir + ' to ' + Math.round(y) + 'px' });
  }, true);
  if (window.top === window) {
    var navigated = function (detail) { emit({ kind: 'navigate', detail: detail, target: { description: clean(document.title) || location.href, tag: 'document' } }); };
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', function () { navigated('load'); }, { once: true });
    else navigated('load');
    ['pushState', 'replaceState'].forEach(function (name) {
      var orig = history[name];
      history[name] = function () { var out = orig.apply(this, arguments); setTimeout(function () { navigated('in-page'); }, 0); return out; };
    });
    window.addEventListener('popstate', function () { setTimeout(function () { navigated('back/forward'); }, 0); });
  }
})(__CONFIG__)"#;

fn recorder_source() -> String {
    let cfg = json!({"binding": BINDING, "stateKey": STATE_KEY, "sensitivePattern": SENSITIVE_HINT});
    RECORDER_JS.replace("__CONFIG__", &cfg.to_string())
}

fn off_source() -> String {
    format!("(function(){{var s=window[{k}]; if (s) s.on=false;}})()", k = json!(STATE_KEY))
}

fn drain_source() -> String {
    format!("(function(){{var s=window[{k}]; if(!s||!s.queue) return []; var o=s.queue.slice(); s.queue.length=0; return o;}})()", k = json!(STATE_KEY))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub description: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedAction {
    pub id: String,
    pub at: String,
    /// navigate | click | input | select | submit | key | scroll
    pub kind: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ActionTarget>,
    /// Typed or selected text; a `<redacted:…>` marker when sensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl RecordedAction {
    /// One line for the log the human watches and the model reads.
    pub fn line(&self) -> String {
        let what = self.target.as_ref().map(|t| {
            let d = if t.description.is_empty() { t.selector.clone().unwrap_or_default() } else { format!("\"{}\"", t.description) };
            match &t.r#type {
                Some(ty) if t.tag == "input" => format!("{} {ty} {d}", t.tag),
                _ => format!("{} {d}", t.tag),
            }
        });
        let mut s = match self.kind.as_str() {
            "navigate" => format!("navigate → {}", self.url),
            "input" | "select" => format!("{} {} = {}", self.kind, what.unwrap_or_default(), self.value.clone().unwrap_or_default()),
            _ => format!("{} {}", self.kind, what.unwrap_or_default()),
        };
        if let Some(d) = &self.detail
            && (self.kind != "navigate" || d != "load") {
                s.push_str(&format!(" ({d})"));
            }
        s.trim().to_string()
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn cut(x: &str, n: usize) -> String {
    x.chars().take(n).collect()
}

/// Second-pass cleaning of what the page sent. Anything malformed is dropped.
pub fn sanitize(raw: &Value) -> Option<RecordedAction> {
    let id = raw.get("id")?.as_str()?.to_string();
    let kind = raw.get("kind")?.as_str()?.to_string();
    if !["navigate", "click", "input", "select", "submit", "key", "scroll"].contains(&kind.as_str()) {
        return None;
    }
    let target = raw.get("target").filter(|t| t.is_object()).map(|t| ActionTarget {
        description: cut(&s(&t["description"]), 120),
        tag: Some(cut(&s(&t["tag"]), 20)).filter(|x| !x.is_empty()).unwrap_or_else(|| "unknown".into()),
        r#type: t.get("type").map(s).filter(|x| !x.is_empty()).map(|x| cut(&x, 20)),
        selector: t.get("selector").map(s).filter(|x| !x.is_empty()).map(|x| cut(&x, 200)),
    });
    let mut a = RecordedAction {
        id,
        at: raw.get("at").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        kind,
        url: scrub_url(&s(raw.get("url").unwrap_or(&Value::Null))),
        target,
        value: None,
        redacted: false,
        detail: raw.get("detail").map(s).filter(|x| !x.is_empty()).map(|x| cut(&x, 200)),
    };
    if let Some(v) = raw.get("value") {
        let (value, red) = redact_value(&s(v), a.target.as_ref(), raw.get("redacted").and_then(|r| r.as_bool()).unwrap_or(false));
        a.value = Some(value);
        if red {
            a.redacted = true;
            a.detail.get_or_insert_with(|| "redacted".into());
        }
    }
    Some(a)
}

/// Decide redaction again from the metadata alone: a value that arrives in the
/// clear on a sensitive-looking field means the page's check was bypassed.
pub fn redact_value(value: &str, target: Option<&ActionTarget>, already: bool) -> (String, bool) {
    if already {
        return if value.starts_with("<redacted:") { (value.to_string(), true) } else { ("<redacted:sensitive>".into(), true) };
    }
    if MARKER_RE.is_match(value) {
        return (value.to_string(), true);
    }
    let hint = target
        .map(|t| [Some(t.description.clone()), t.r#type.clone(), t.selector.clone()].into_iter().flatten().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if target.and_then(|t| t.r#type.as_deref()) == Some("password") || HINT_RE.is_match(&hint) {
        return ("<redacted:sensitive>".into(), true);
    }
    let digits: String = value.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if digits.len() >= 12 && digits.chars().all(|c| c.is_ascii_digit()) {
        return ("<redacted:long-number>".into(), true);
    }
    if TOKENISH_RE.is_match(value) && value.chars().any(|c| c.is_ascii_digit()) && value.chars().any(|c| c.is_ascii_alphabetic()) {
        return ("<redacted:secret-like>".into(), true);
    }
    (cut(value, 200), false)
}

/// Redirect URLs routinely carry a credential in the query string or fragment.
pub fn scrub_url(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if raw.len() >= 5 && raw[..5].eq_ignore_ascii_case("data:") {
        return if raw.chars().count() > 200 { format!("{}…", cut(raw, 200)) } else { raw.to_string() };
    }
    let Ok(mut url) = url::Url::parse(raw) else { return cut(raw, 500) };
    let mut touched = false;
    let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if pairs.iter().any(|(k, _)| PARAM_RE.is_match(k)) {
        touched = true;
        let mut q = url.query_pairs_mut();
        q.clear();
        for (k, v) in &pairs {
            q.append_pair(k, if PARAM_RE.is_match(k) { "REDACTED" } else { v });
        }
    }
    if url.fragment().is_some_and(|f| HASH_TOKEN_RE.is_match(&format!("#{f}"))) {
        url.set_fragment(Some("REDACTED"));
        touched = true;
    }
    cut(if touched { url.as_str() } else { raw }, 500)
}

/// A recording in progress, on one bot's browser.
pub struct Recorder {
    pub label: String,
    started: Instant,
    actions: Arc<Mutex<Vec<RecordedAction>>>,
    browser: Arc<Browser>,
    script_id: String,
    listener: tokio::task::JoinHandle<()>,
}

impl Recorder {
    pub async fn start(browser: Arc<Browser>, label: &str) -> Result<Self, String> {
        let label = label.trim();
        if label.is_empty() {
            return Err("A recording needs a name — it becomes the draft skill's name.".into());
        }
        let actions: Arc<Mutex<Vec<RecordedAction>>> = Arc::default();
        let mut events = browser.events();
        let sink = actions.clone();
        let listener = tokio::spawn(async move {
            let mut seen = HashSet::new();
            loop {
                match events.recv().await {
                    Ok(e) if e.method == "Runtime.bindingCalled" && e.params["name"] == BINDING => {
                        let Some(payload) = e.params["payload"].as_str() else { continue };
                        let Ok(raw) = serde_json::from_str::<Value>(payload) else { continue };
                        if let Some(a) = sanitize(&raw)
                            && seen.insert(a.id.clone()) {
                                sink.lock().unwrap().push(a);
                            }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        });
        let script_id = match browser.arm(BINDING, &recorder_source()).await {
            Ok(id) => id,
            Err(e) => {
                listener.abort();
                return Err(format!("Could not attach the recorder to the browser ({e}). Is the computer up?"));
            }
        };
        Ok(Self { label: label.to_string(), started: Instant::now(), actions, browser, script_id, listener })
    }

    pub fn count(&self) -> usize {
        self.actions.lock().unwrap().len()
    }

    pub fn actions(&self) -> Vec<RecordedAction> {
        self.actions.lock().unwrap().clone()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn remaining(&self) -> Duration {
        MAX_RECORDING.saturating_sub(self.started.elapsed())
    }

    pub fn expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Stop, collect anything still queued in the page, and store the log.
    pub async fn stop(self, db: &Db) -> Result<Recording, String> {
        if let Ok(Value::Array(batch)) = self.browser.eval(&drain_source()).await {
            let mut v = self.actions.lock().unwrap();
            let have: HashSet<String> = v.iter().map(|a| a.id.clone()).collect();
            v.extend(batch.iter().filter_map(sanitize).filter(|a| !have.contains(&a.id)));
        }
        self.browser.disarm(BINDING, &self.script_id, &off_source()).await;
        self.listener.abort();
        let mut actions = self.actions.lock().unwrap().clone();
        actions.sort_by(|a, b| a.at.cmp(&b.at));
        let json = serde_json::to_string(&actions).map_err(|e| e.to_string())?;
        db.add_recording(&self.label, &json).map_err(|e| e.to_string())
    }
}

pub fn actions_of(rec: &Recording) -> Vec<RecordedAction> {
    serde_json::from_str(&rec.steps).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Compiling a recording into a skill
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillDraft {
    pub name: String,
    /// When a bot should reach for this skill.
    pub trigger: String,
    pub steps: Vec<String>,
    pub safety_rules: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

const COMPILE_SYSTEM: &str = "You turn a recorded browser demonstration into a reusable Skill for an AI teammate that drives a real browser.

The log is what a human actually did, in order. Write the skill so a competent agent could repeat the workflow on a different day with different data — describe intent (\"open the orders page\", \"search for the customer\"), not coordinates or replay steps. Selectors are context, not instructions.

Rules:
- Values shown as <redacted:...> were secrets (passwords, one-time codes, card details). You do not know them and must never guess. Any step that needed one must instead say to use a saved login (fill_login) if there is one, or to stop and hand control to the human.
- Note which concrete values in the demonstration are inputs that will change each run.
- Ignore incidental noise: stray scrolls, mis-clicks the human corrected, focus changes that led nowhere.
- Safety rules must reflect what this workflow actually touches. For anything irreversible or public (sending, posting, purchasing, paying, deleting, accepting terms) write a rule of the form \"always-confirm: <the action>\".

Call emit_skill with the draft.";

fn emit_tool() -> ToolDef {
    ToolDef {
        name: "emit_skill".into(),
        description: "Return the compiled skill draft.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Short kebab-case skill name."},
                "trigger": {"type": "string", "description": "One sentence: when a bot should use this skill."},
                "steps": {"type": "array", "items": {"type": "string"}, "description": "Ordered steps."},
                "safety_rules": {"type": "array", "items": {"type": "string"}, "description": "Boundaries for this skill; `always-confirm: …` for anything irreversible or public."},
                "notes": {"type": "string", "description": "Optional caveats."}
            },
            "required": ["name", "trigger", "steps", "safety_rules"]
        }),
    }
}

/// The log as the model sees it, capped.
pub fn format_actions(actions: &[RecordedAction], max: usize) -> String {
    let mut out: Vec<String> = actions.iter().take(max).enumerate().map(|(i, a)| format!("{}. {}", i + 1, a.line())).collect();
    if actions.len() > max {
        out.push(format!("… {} more actions not shown", actions.len() - max));
    }
    out.join("\n")
}

/// Ask a model for a draft. Nothing is saved: the human reviews it first.
pub async fn compile(provider: &dyn Provider, model: Option<String>, label: &str, actions: &[RecordedAction]) -> Result<SkillDraft, String> {
    if actions.is_empty() {
        return Err(format!("\"{label}\" captured no actions. Was the demonstration done in the bot's browser, in the live view?"));
    }
    let prompt = format!("Demonstration: {label}\nActions recorded: {}\n\nAction log:\n{}", actions.len(), format_actions(actions, 400));
    let opts = ChatOptions { model, system: Some(COMPILE_SYSTEM.into()), tools: vec![emit_tool()], max_tokens: Some(8000), effort: None };
    let done = provider.chat(&[Message::user(prompt)], &opts, &|_| {}).await.map_err(|e| e.to_string())?;
    let raw = done
        .message
        .parts
        .iter()
        .find_map(|p| match p {
            Part::ToolCall { name, input, .. } if name == "emit_skill" => Some(input.clone()),
            _ => None,
        })
        .or_else(|| extract_json(&done.message.text()))
        .ok_or_else(|| format!("The model did not return a usable draft. It said: {}", cut(done.message.text().trim(), 400)))?;
    normalize_draft(&raw, label)
}

/// The first `{…}` object in free text (some models answer in prose + JSON).
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

pub fn normalize_draft(raw: &Value, fallback_name: &str) -> Result<SkillDraft, String> {
    let list = |v: &Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().map(s).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default()
    };
    let steps = list(raw.get("steps").unwrap_or(&Value::Null));
    if steps.is_empty() {
        return Err("The draft has no steps.".into());
    }
    let name = raw.get("name").map(s).map(|n| mybot_catalog::agent_skill::slug(&n)).filter(|n| !n.is_empty()).unwrap_or_else(|| mybot_catalog::agent_skill::slug(fallback_name));
    Ok(SkillDraft {
        name: if name.is_empty() { "taught-skill".into() } else { name },
        trigger: raw.get("trigger").map(s).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("When asked to: {fallback_name}")),
        steps,
        safety_rules: list(raw.get("safety_rules").or(raw.get("safetyRules")).unwrap_or(&Value::Null)),
        notes: raw.get("notes").map(s).filter(|n| !n.trim().is_empty()),
    })
}

/// The skill body; safety rules are stored separately.
pub fn instructions(d: &SkillDraft) -> String {
    let mut lines = vec![format!("Use this when: {}", d.trigger), String::new(), "Steps:".into()];
    for (i, st) in d.steps.iter().enumerate() {
        lines.push(format!("{}. {st}", i + 1));
    }
    if let Some(n) = d.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        lines.push(String::new());
        lines.push(format!("Notes: {}", n.trim()));
    }
    lines.join("\n")
}

pub fn render_draft(d: &SkillDraft) -> String {
    let mut out = format!("Name: {}\n\n{}\n\nSafety rules:", d.name, instructions(d));
    for r in &d.safety_rules {
        out.push_str(&format!("\n- {r}"));
    }
    out
}

/// Save a reviewed draft as a `taught` skill, linked to its recording.
pub fn save(db: &Db, recording_id: Option<&str>, d: &SkillDraft) -> Result<SkillRow, String> {
    let taken = |n: &str| mybot_catalog::skill(n).is_some() || db.skill(n).ok().flatten().is_some();
    let mut name = d.name.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{}-{n}", d.name);
        n += 1;
        if n > 100 {
            return Err(format!("Too many skills already named \"{}\".", d.name));
        }
    }
    let row = db
        .add_skill(&name, "taught", "taught", &cut(&d.trigger, 300), &instructions(d), &d.safety_rules.join("\n"))
        .map_err(|e| e.to_string())?;
    if let Some(id) = recording_id {
        let _ = db.set_recording_skill(id, &row.id);
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_on_sensitive_fields_are_redacted_again() {
        let t = ActionTarget { description: "One-time code".into(), tag: "input".into(), r#type: Some("text".into()), selector: None };
        assert_eq!(redact_value("123456", Some(&t), false), ("<redacted:sensitive>".into(), true));
        let p = ActionTarget { description: "x".into(), tag: "input".into(), r#type: Some("password".into()), selector: None };
        assert!(redact_value("hunter2", Some(&p), false).1);
        assert_eq!(redact_value("4111 1111 1111 1111", None, false).0, "<redacted:long-number>");
        assert_eq!(redact_value("sk_live_abcdefghijklmnop1234567", None, false).0, "<redacted:secret-like>");
        assert_eq!(redact_value("plain words", None, false), ("plain words".into(), false));
        assert_eq!(redact_value("cleartext", None, true).0, "<redacted:sensitive>", "a page claim of redaction wins");
    }

    #[test]
    fn urls_lose_credentials() {
        let u = scrub_url("https://a.example/login?user=me&pw=hunter2&cc=4111&q=shoes");
        assert!(!u.contains("hunter2") && !u.contains("4111"), "{u}");
        assert!(u.contains("q=shoes") && u.contains("user=me"), "{u}");
        assert!(scrub_url("https://a.example/cb#access_token=abc").ends_with("#REDACTED"));
        assert_eq!(scrub_url("https://a.example/plain"), "https://a.example/plain");
    }

    #[test]
    fn page_payloads_are_checked() {
        let a = sanitize(&json!({"id": "d:1", "kind": "input", "url": "https://a.example/?token=x", "value": "123456789012345",
            "target": {"description": "Amount", "tag": "input", "type": "text"}}))
        .unwrap();
        assert_eq!(a.value.as_deref(), Some("<redacted:long-number>"));
        assert!(a.url.contains("token=REDACTED"));
        assert!(sanitize(&json!({"id": "d:2", "kind": "eval"})).is_none(), "unknown kinds are dropped");
        assert!(sanitize(&json!({"kind": "click"})).is_none());
    }

    #[test]
    fn drafts_normalise_and_save() {
        let d = normalize_draft(&json!({"name": "Check Orders!", "trigger": "t", "steps": ["a", " ", "b"], "safety_rules": ["always-confirm: refund"]}), "x").unwrap();
        assert_eq!(d.name, "check-orders");
        assert_eq!(d.steps, vec!["a", "b"]);
        assert!(normalize_draft(&json!({"steps": []}), "x").is_err());
        let db = Db::in_memory().unwrap();
        let a = save(&db, None, &d).unwrap();
        let b = save(&db, None, &d).unwrap();
        assert_eq!(a.source, "taught");
        assert_eq!(b.name, "check-orders-2");
        assert!(a.instructions.contains("1. a") && a.safety_rules.contains("refund"));
        assert!(render_draft(&d).contains("Safety rules:"));
    }

    #[test]
    fn recorder_script_is_configured() {
        let src = recorder_source();
        assert!(src.contains(BINDING) && !src.contains("__CONFIG__"));
        assert!(extract_json("Here: {\"a\": 1} done").is_some());
    }
}
