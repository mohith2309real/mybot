//! What the window shows for each bot's conversation, fed by run events
//! (from the runtime) and rebuilt from the database when a thread is opened.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mybot_core::agent::{AgentEvent, Pause, RunStatus};
use mybot_core::db::Db;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    You(String),
    Bot(String),
    Thinking(String),
    Step { name: String, input: String, result: Option<(String, bool)> },
    Paused { pause: Pause, task_id: Option<String> },
    Note(String),
    Done(String),
    Problem(String),
}

#[derive(Debug, Clone, Default)]
pub struct Thread {
    pub items: Vec<Item>,
    pub live_text: String,
    pub live_thinking: String,
    pub loaded: bool,
    /// The run is waiting on the human, for this task.
    pub waiting: Option<(String, Pause)>,
    pub working: bool,
    pub task_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct Threads(pub Arc<Mutex<HashMap<String, Thread>>>);

impl Threads {
    pub fn with<R>(&self, bot_id: &str, f: impl FnOnce(&mut Thread) -> R) -> R {
        let mut m = self.0.lock().unwrap();
        f(m.entry(bot_id.to_string()).or_default())
    }

    /// Apply one run event to a bot's thread.
    pub fn apply(&self, bot_id: &str, e: AgentEvent) {
        self.with(bot_id, |t| {
            let flush_thinking = |t: &mut Thread| {
                let th = std::mem::take(&mut t.live_thinking);
                if !th.trim().is_empty() {
                    t.items.push(Item::Thinking(th.trim().to_string()));
                }
            };
            match e {
                AgentEvent::Started { .. } => {
                    t.working = true;
                }
                AgentEvent::TextDelta(s) => t.live_text.push_str(&s),
                AgentEvent::ThinkingDelta(s) => t.live_thinking.push_str(&s),
                AgentEvent::Thinking(s) => t.items.push(Item::Thinking(s)),
                AgentEvent::Assistant(s) => {
                    flush_thinking(t);
                    t.live_text.clear();
                    match s.strip_prefix("▸ ") {
                        Some(note) => t.items.push(Item::Note(note.to_string())),
                        None => t.items.push(Item::Bot(s)),
                    }
                }
                AgentEvent::ToolCall { name, input } => {
                    flush_thinking(t);
                    t.live_text.clear();
                    t.items.push(Item::Step { name, input: compact(&input), result: None });
                }
                AgentEvent::ToolResult { name, content, is_error } => {
                    if let Some(Item::Step { result, .. }) = t.items.iter_mut().rev().find(|i| matches!(i, Item::Step { name: n, result: None, .. } if *n == name)) {
                        *result = Some((content, is_error));
                    }
                }
                AgentEvent::Paused(p) => {
                    let task = t.task_id.clone();
                    t.items.push(Item::Paused { pause: p.clone(), task_id: task.clone() });
                    if let Some(task) = task {
                        t.waiting = Some((task, p));
                    }
                }
                AgentEvent::Resumed { skipped } => {
                    t.waiting = None;
                    t.items.push(Item::Note(if skipped { "You skipped that step. The bot carries on without it.".into() } else { "Handed back to the bot.".into() }));
                }
                AgentEvent::Finished { status, summary } => {
                    flush_thinking(t);
                    t.live_text.clear();
                    t.waiting = None;
                    t.working = false;
                    let already = matches!(t.items.last(), Some(Item::Bot(b)) if b.trim() == summary.trim());
                    match status {
                        RunStatus::Done if !already => t.items.push(Item::Done(summary)),
                        RunStatus::Done => {}
                        RunStatus::Cancelled => t.items.push(Item::Note(summary)),
                        _ => t.items.push(Item::Problem(summary)),
                    }
                }
            }
        });
    }

    /// Rebuild a thread from stored tasks and logs (first open, or after restart).
    pub fn load(&self, db: &Db, bot_id: &str) {
        let tasks = db.tasks_for(bot_id, 30).unwrap_or_default();
        let mut items = Vec::new();
        for task in &tasks {
            items.push(Item::You(task.instruction.clone()));
            for e in db.log(&task.id).unwrap_or_default() {
                match e.kind.as_str() {
                    "assistant" => items.push(Item::Bot(e.text)),
                    "thinking" => items.push(Item::Thinking(e.text)),
                    "tool_call" => {
                        let (name, input) = e.text.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((e.text.clone(), String::new()));
                        items.push(Item::Step { name, input, result: None });
                    }
                    "tool_result" | "tool_error" => {
                        let content = e.text.split_once(" -> ").map(|(_, b)| b.to_string()).unwrap_or(e.text.clone());
                        if let Some(Item::Step { result, .. }) = items.iter_mut().rev().find(|i| matches!(i, Item::Step { result: None, .. })) {
                            *result = Some((content, e.kind == "tool_error"));
                        }
                    }
                    "pause" => items.push(Item::Note(format!("Stopped for you: {}", e.text))),
                    "resume" => items.push(Item::Note(e.text)),
                    "done" => {
                        if !matches!(items.last(), Some(Item::Bot(b)) if b.trim() == e.text.trim()) {
                            items.push(Item::Done(e.text));
                        }
                    }
                    "error" | "refusal" => items.push(Item::Problem(e.text)),
                    "cancelled" => items.push(Item::Note(e.text)),
                    _ => {}
                }
            }
        }
        self.with(bot_id, |t| {
            t.items = items;
            t.loaded = true;
        });
    }
}

/// `{"url":"https://…"}` → `url: https://…`, short.
pub fn compact(v: &Value) -> String {
    let s = match v {
        Value::Object(o) if o.len() == 1 => {
            let (k, x) = o.iter().next().unwrap();
            format!("{k}: {}", x.as_str().map(String::from).unwrap_or_else(|| x.to_string()))
        }
        Value::Object(o) if o.is_empty() => String::new(),
        _ => v.to_string(),
    };
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 140 { format!("{}…", flat.chars().take(140).collect::<String>()) } else { flat }
}

/// A human-friendly verb for a tool step.
pub fn verb(name: &str) -> &'static str {
    match name {
        "page_read" => "Read the page",
        "page_navigate" => "Opened",
        "page_click" => "Clicked",
        "page_type" => "Typed",
        "fill_login" => "Signed in with a saved login",
        "page_scroll" => "Scrolled",
        "bash" => "Ran",
        "actions_search" => "Looked for an action",
        "action_run" => "Used",
        "request_human" => "Asked you",
        "task_done" => "Finished",
        _ => "Step",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_build_a_readable_thread() {
        let th = Threads::default();
        th.with("b", |t| t.task_id = Some("t1".into()));
        th.apply("b", AgentEvent::Started { instruction: "x".into() });
        th.apply("b", AgentEvent::ThinkingDelta("plan".into()));
        th.apply("b", AgentEvent::ToolCall { name: "page_navigate".into(), input: json!({"url": "https://a.example"}) });
        th.apply("b", AgentEvent::ToolResult { name: "page_navigate".into(), content: "Opened".into(), is_error: false });
        th.apply("b", AgentEvent::Paused(Pause::new("captcha", "solve it", "https://a.example", true)));
        assert!(th.with("b", |t| t.waiting.is_some()));
        th.apply("b", AgentEvent::Resumed { skipped: false });
        th.apply("b", AgentEvent::Assistant("All done.".into()));
        th.apply("b", AgentEvent::Finished { status: RunStatus::Done, summary: "All done.".into() });
        let items = th.with("b", |t| t.items.clone());
        assert_eq!(items[0], Item::Thinking("plan".into()));
        assert_eq!(items[1], Item::Step { name: "page_navigate".into(), input: "url: https://a.example".into(), result: Some(("Opened".into(), false)) });
        assert!(matches!(items[2], Item::Paused { .. }));
        assert_eq!(items.last(), Some(&Item::Bot("All done.".into())), "no duplicate summary");
        assert!(!th.with("b", |t| t.working || t.waiting.is_some()));
    }
}
