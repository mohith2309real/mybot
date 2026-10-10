//! Provider-neutral conversation types.
//!
//! Every adapter converts to and from these, so the agent loop, the tools,
//! skills and routines never see a provider's wire shape.
//!
//! One deliberate exception: [`Message::raw`]. Claude's thinking blocks (with
//! their signatures) and Gemini's thought signatures have to be sent back
//! *unchanged* on the next request, or the provider rejects the turn or loses
//! its reasoning. Rebuilding them from neutral parts would change them, so the
//! adapter that produced a turn keeps its exact content here and replays it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    /// A tool the model wants run. Only ever produced by a model.
    ToolCall { id: String, name: String, input: Value },
    /// The result of running one, sent back on the next turn.
    ToolResult {
        call_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

/// A provider's own content for an assistant turn, replayed verbatim to that
/// same provider and ignored by the others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTurn {
    pub provider: String,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawTurn>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, parts: vec![Part::Text { text: text.into() }], raw: None }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, parts: vec![Part::Text { text: text.into() }], raw: None }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self { role: Role::System, parts: vec![Part::Text { text: text.into() }], raw: None }
    }

    pub fn tool_results(results: Vec<Part>) -> Self {
        Self { role: Role::User, parts: results, raw: None }
    }

    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn tool_calls(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.parts.iter().filter_map(|p| match p {
            Part::ToolCall { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema, `{"type":"object","properties":{…}}`.
    pub parameters: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, o: Self) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
    }
}

/// What a provider streams while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// A complete tool call, once its arguments have fully arrived.
    ToolCall { id: String, name: String, input: Value },
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub stop: StopReason,
    pub usage: Usage,
    /// The assistant turn to append to history.
    pub message: Message,
    /// Which model actually served it (can differ under a fallback).
    pub model: String,
    /// Refusal category, when the provider gives one.
    pub refusal_category: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    #[default]
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "low" => Effort::Low,
            "medium" => Effort::Medium,
            "high" => Effort::High,
            "xhigh" => Effort::Xhigh,
            "max" => Effort::Max,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub model: Option<String>,
    pub system: Option<String>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: Option<u32>,
    pub effort: Option<Effort>,
}

/// Leading system messages are concatenated: providers take system out-of-band.
pub fn split_system(messages: &[Message], explicit: Option<&str>) -> (Option<String>, Vec<Message>) {
    let mut systems: Vec<String> = explicit.into_iter().map(String::from).collect();
    let mut rest = Vec::new();
    for m in messages {
        if m.role == Role::System {
            systems.push(m.text());
        } else {
            rest.push(m.clone());
        }
    }
    let system = (!systems.is_empty()).then(|| systems.join("\n\n"));
    (system, rest)
}

/// Gemini keys a tool result by function *name*; recover it from the call id.
pub fn tool_name_for_call(messages: &[Message], call_id: &str) -> String {
    for m in messages {
        for p in &m.parts {
            if let Part::ToolCall { id, name, .. } = p {
                if id == call_id {
                    return name.clone();
                }
            }
        }
    }
    "unknown_tool".into()
}
