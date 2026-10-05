//! The agent loop: model → tool calls → results → model, until the bot calls
//! `task_done`, stops calling tools, or hits the iteration cap.
//!
//! The loop is append-only: earlier turns are never edited, because providers
//! bind reasoning to the exact conversation that produced it. Everything is
//! written to the task log as it happens, so a run is auditable afterwards.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::cancel::Cancel;
use crate::db::Db;
use crate::model::*;
use crate::providers::{Provider, ProviderError};
use crate::providers::INVALID_JSON_KEY;

pub const BASE_SYSTEM: &str = "You are a bot operating a real computer with a browser, a shell, and a filesystem.

The computer is shared with the other bots on this account: same /workspace, same browser
profile, same logged-in sessions. Anything you sign into stays signed in for everyone.
Leave the machine in a sane state.

How to work:
- Call page_read before acting on a page, and again after anything that changes it.
  Element refs are only valid until the next page_read.
- Prefer the shell for file and data work; prefer the browser for the web.
- Many more actions exist than the core tools listed: call actions_search with what you
  need (\"resize image\", \"parse csv\", \"open new tab\") and then action_run.
- Work in /workspace. It persists across runs.
- When you are finished, call task_done with a summary. Do not stop without it.

Boundaries — these are hard limits, not preferences:
- Never type passwords, one-time codes, 2FA codes, or card details yourself.
- To sign in, call fill_login with the refs of the password and username fields. If the
  human saved a login for that exact site, they are asked to approve and MyBot fills it
  in — you never see the password. If not, the run pauses for them.
- One-time codes, 2FA and card fields always pause: a human opens a live view of your
  browser, completes that one step, and hands control back. When that happens, do not
  retry the step they just did — call page_read and carry on.
- If the human declines a sign-in, do not ask again in the same task.
- Call request_human whenever you judge a step needs a person. That is a normal move,
  not a failure, and it is better than guessing or giving up.";

/// A run stopped at a security boundary or by the bot's own request.
#[derive(Debug, Clone, PartialEq)]
pub struct Pause {
    /// captcha | two_factor | password | payment | always_confirm
    pub kind: String,
    /// One line for the human.
    pub reason: String,
    /// Where the boundary was hit.
    pub url: String,
    /// Page-level (acknowledgeable) vs element/model-level.
    pub page_level: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    /// The bot signalled completion with this summary.
    pub done: Option<String>,
    /// The action did not happen; a human is needed first.
    pub pause: Option<Pause>,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self { content: content.into(), ..Default::default() }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: true, ..Default::default() }
    }
    pub fn paused(prefix: impl Into<String>, pause: Pause) -> Self {
        let prefix = prefix.into();
        Self {
            content: format!("{prefix}Paused — this needs a human ({}): {}", pause.kind, pause.reason),
            pause: Some(pause),
            ..Default::default()
        }
    }
}

/// Who is running, on whose behalf.
#[derive(Clone)]
pub struct RunCtx {
    pub bot: String,
    pub task_id: Option<String>,
    pub instruction: String,
    pub cancel: Cancel,
}

/// The tool surface a run gets.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn tools(&self) -> Vec<ToolDef>;
    async fn run(&self, name: &str, input: &Value, ctx: &RunCtx) -> ToolOutcome;
    /// A human dealt with this boundary; don't stop again for the same page.
    fn acknowledge(&self, _pause: &Pause) {}
    /// Start of a run: no consent carries over from a previous task.
    fn reset(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// The human did the step.
    Done,
    /// The human declined the step — it will never happen.
    Skipped,
    TimedOut,
    Cancelled,
}

/// How a paused run waits for its human.
#[async_trait]
pub trait Handoff: Send + Sync {
    async fn wait(&self, task_id: Option<&str>, pause: &Pause, timeout: Duration, cancel: &Cancel) -> Resume;
}

/// Everything a run reports as it goes.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Started { instruction: String },
    TextDelta(String),
    ThinkingDelta(String),
    Thinking(String),
    Assistant(String),
    ToolCall { name: String, input: Value },
    ToolResult { name: String, content: String, is_error: bool },
    Paused(Pause),
    Resumed { skipped: bool },
    Finished { status: RunStatus, summary: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Done,
    MaxIterations,
    Refused,
    Error,
    PausedTimeout,
    Cancelled,
}

impl RunStatus {
    pub fn task_status(self) -> &'static str {
        match self {
            RunStatus::Done => "done",
            RunStatus::Cancelled => "cancelled",
            _ => "failed",
        }
    }
}

pub struct RunOptions {
    pub provider: Arc<dyn Provider>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    /// BASE_SYSTEM plus policy, persona and skill text, assembled by the caller.
    pub system: String,
    /// Earlier turns with this bot, so follow-ups have context.
    pub history: Vec<Message>,
    pub instruction: String,
    pub max_iterations: usize,
    pub pause_timeout: Duration,
    pub ctx: RunCtx,
    pub db: Option<Db>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub status: RunStatus,
    pub summary: String,
    pub iterations: usize,
    pub usage: Usage,
    pub model: String,
}

pub async fn run_task(
    opts: RunOptions,
    tools: &dyn ToolHost,
    handoff: &dyn Handoff,
    on_event: &(dyn Fn(AgentEvent) + Send + Sync),
) -> RunResult {
    let ctx = opts.ctx.clone();
    let task_id = ctx.task_id.clone();
    let log = |kind: &str, text: &str| {
        if let (Some(db), Some(t)) = (&opts.db, &task_id) {
            let _ = db.append_log(t, kind, text);
        }
    };
    let finish = |status: RunStatus, summary: String, iterations: usize, usage: Usage, model: String| {
        log(match status { RunStatus::Done => "done", RunStatus::Cancelled => "cancelled", RunStatus::Refused => "refusal", _ => "error" }, &summary);
        if let (Some(db), Some(t)) = (&opts.db, &task_id) {
            let _ = db.finish_task(t, status.task_status(), &summary, usage.input_tokens, usage.output_tokens);
        }
        on_event(AgentEvent::Finished { status, summary: summary.clone() });
        RunResult { status, summary, iterations, usage, model }
    };

    tools.reset();
    if let (Some(db), Some(t)) = (&opts.db, &task_id) {
        let _ = db.set_task_status(t, "running");
    }
    log("user", &opts.instruction);
    on_event(AgentEvent::Started { instruction: opts.instruction.clone() });

    let mut messages = opts.history.clone();
    messages.push(Message::user(opts.instruction.clone()));
    let mut usage = Usage::default();
    let mut served = opts.model.clone().unwrap_or_else(|| opts.provider.default_model().to_string());
    let chat_opts = ChatOptions {
        model: opts.model.clone(),
        system: Some(opts.system.clone()),
        tools: tools.tools(),
        max_tokens: Some(32_000),
        effort: opts.effort,
    };

    for i in 1..=opts.max_iterations.max(1) {
        if ctx.cancel.is_cancelled() {
            return finish(RunStatus::Cancelled, "Stopped by you.".into(), i, usage, served);
        }

        let sink = |e: StreamEvent| match e {
            StreamEvent::TextDelta(t) => on_event(AgentEvent::TextDelta(t)),
            StreamEvent::ThinkingDelta(t) => on_event(AgentEvent::ThinkingDelta(t)),
            StreamEvent::ToolCall { .. } => {}
        };
        let turn = tokio::select! {
            r = opts.provider.chat(&messages, &chat_opts, &sink) => r,
            _ = ctx.cancel.cancelled() => {
                return finish(RunStatus::Cancelled, "Stopped by you.".into(), i, usage, served);
            }
        };
        let completion = match turn {
            Ok(c) => c,
            Err(e) => return finish(RunStatus::Error, describe(&e), i, usage, served),
        };
        usage += completion.usage;
        if !completion.model.is_empty() {
            served = completion.model.clone();
        }

        let text = completion.message.text();
        if !text.trim().is_empty() {
            log("assistant", text.trim());
            on_event(AgentEvent::Assistant(text.trim().to_string()));
        }

        if completion.stop == StopReason::Refusal {
            let why = match &completion.refusal_category {
                Some(c) => format!("The model declined this task ({c})."),
                None => "The model declined this task.".to_string(),
            };
            return finish(RunStatus::Refused, why, i, usage, served);
        }

        let calls: Vec<(String, String, Value)> =
            completion.message.tool_calls().map(|(a, b, c)| (a.to_string(), b.to_string(), c.clone())).collect();

        if calls.is_empty() {
            let summary = if text.trim().is_empty() { "(no output)".to_string() } else { text.trim().to_string() };
            if completion.stop == StopReason::MaxTokens {
                return finish(RunStatus::Error, format!("The reply was cut off at the output limit. {summary}"), i, usage, served);
            }
            return finish(RunStatus::Done, summary, i, usage, served);
        }

        messages.push(completion.message.clone());

        // Cut off mid-call: an input can parse as a valid-looking prefix of
        // what was meant, so none of these run.
        if completion.stop == StopReason::MaxTokens {
            let results = calls
                .iter()
                .map(|(id, _, _)| Part::ToolResult {
                    call_id: id.clone(),
                    content: "Not run: your output hit the length limit while writing this call. Redo it with shorter input.".into(),
                    is_error: true,
                })
                .collect();
            messages.push(Message::tool_results(results));
            continue;
        }

        let mut results = Vec::new();
        let mut paused_this_turn = false;
        for (c, (id, name, input)) in calls.iter().enumerate() {
            log("tool_call", &format!("{name} {input}"));
            on_event(AgentEvent::ToolCall { name: name.clone(), input: input.clone() });

            let outcome = if let Some(raw) = input.get(INVALID_JSON_KEY) {
                ToolOutcome::err(json!({ "INVALID_JSON": raw }).to_string())
            } else {
                tokio::select! {
                    o = tools.run(name, input, &ctx) => o,
                    _ = ctx.cancel.cancelled() => ToolOutcome::err("Stopped by you."),
                }
            };

            if let Some(pause) = outcome.pause.clone() {
                let label = format!("{}: {}", pause.kind, pause.reason);
                log("pause", &format!("{label} — {}", pause.url));
                if let (Some(db), Some(t)) = (&opts.db, &task_id) {
                    let _ = db.pause_task(t, &label);
                }
                on_event(AgentEvent::Paused(pause.clone()));

                let resume = handoff.wait(task_id.as_deref(), &pause, opts.pause_timeout, &ctx.cancel).await;
                let note = match resume {
                    Resume::Done => {
                        tools.acknowledge(&pause);
                        "A human opened the live view and completed that step. Do NOT redo it and do NOT re-enter \
                         anything they entered. Call page_read to see where the page is now, then continue from there."
                    }
                    Resume::Skipped => {
                        tools.acknowledge(&pause);
                        "A human DECLINED to do that step, so it will not happen. Do NOT ask for it again and do NOT \
                         attempt it yourself. Continue with anything still possible without it; if the task cannot be \
                         finished without that step, stop and say so plainly."
                    }
                    Resume::TimedOut => "No human came in time, so the run stopped here.",
                    Resume::Cancelled => "The task was cancelled while it was waiting for a human.",
                };

                results.push(Part::ToolResult { call_id: id.clone(), content: format!("{}\n\n{note}", outcome.content), is_error: false });
                // Anything else queued this turn was planned against a page a
                // human has since been driving — never run it blind.
                for (skipped_id, _, _) in &calls[c + 1..] {
                    results.push(Part::ToolResult {
                        call_id: skipped_id.clone(),
                        content: "Not run: the run paused for a human first. Call page_read, then redo this only if still needed.".into(),
                        is_error: false,
                    });
                }
                messages.push(Message::tool_results(std::mem::take(&mut results)));
                paused_this_turn = true;

                match resume {
                    Resume::Done | Resume::Skipped => {
                        if let (Some(db), Some(t)) = (&opts.db, &task_id) {
                            let _ = db.set_task_status(t, "running");
                        }
                        log("resume", if resume == Resume::Skipped { "human skipped this step" } else { "human handed control back" });
                        on_event(AgentEvent::Resumed { skipped: resume == Resume::Skipped });
                        break;
                    }
                    Resume::TimedOut => return finish(RunStatus::PausedTimeout, note.into(), i, usage, served),
                    Resume::Cancelled => return finish(RunStatus::Cancelled, note.into(), i, usage, served),
                }
            }

            log(if outcome.is_error { "tool_error" } else { "tool_result" }, &format!("{name} -> {}", preview(&outcome.content)));
            on_event(AgentEvent::ToolResult { name: name.clone(), content: outcome.content.clone(), is_error: outcome.is_error });
            results.push(Part::ToolResult { call_id: id.clone(), content: outcome.content.clone(), is_error: outcome.is_error });

            if let Some(summary) = outcome.done {
                // Still send the results so the transcript stays well-formed.
                for (rest_id, _, _) in &calls[c + 1..] {
                    results.push(Part::ToolResult { call_id: rest_id.clone(), content: "Not run: the task was already marked done.".into(), is_error: false });
                }
                messages.push(Message::tool_results(results));
                return finish(RunStatus::Done, summary, i, usage, served);
            }
        }
        if !paused_this_turn {
            messages.push(Message::tool_results(results));
        }
    }

    finish(
        RunStatus::MaxIterations,
        format!("Stopped after {} steps without calling task_done.", opts.max_iterations),
        opts.max_iterations,
        usage,
        served,
    )
}

fn describe(e: &ProviderError) -> String {
    e.to_string()
}

pub fn preview(s: &str) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 300 { format!("{}…", flat.chars().take(300).collect::<String>()) } else { flat }
}

/// Earlier tasks with a bot as conversation history: instruction, then what it
/// reported back. Enough for "continue" or "now the other one" to mean something.
pub fn history_from_tasks(tasks: &[crate::db::Task]) -> Vec<Message> {
    let mut out = Vec::new();
    for t in tasks {
        let Some(summary) = t.summary.as_deref().filter(|s| !s.trim().is_empty()) else { continue };
        out.push(Message::user(t.instruction.clone()));
        out.push(Message::assistant(summary.to_string()));
    }
    out
}
