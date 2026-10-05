//! OpenAI, over the Responses API (not chat.completions).
//!
//! Responses streams typed semantic events: `response.output_text.delta` for
//! text, `response.output_item.done` carrying a `function_call` for tools.
//! A tool result must reference the call's `call_id` — `item.id` is a
//! different identifier and will not round-trip. Arguments arrive as a JSON
//! string and are parsed here.
//!
//! The same adapter drives a ChatGPT account through a local `openai-oauth`
//! proxy: it is the same API at a different base URL with no key.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::http::{self, CLIENT};
use super::{EventSink, Provider, ProviderConfig, ProviderError};
use crate::model::*;
use crate::sse;

pub const DEFAULT_MODEL: &str = "gpt-5.6";

pub struct OpenAi {
    cfg: ProviderConfig,
}

impl OpenAi {
    pub fn new(cfg: ProviderConfig) -> Self {
        Self { cfg }
    }

    fn base(&self) -> String {
        http::base(&self.cfg.base_url, "https://api.openai.com/v1")
    }
}

fn reasoning_model(model: &str) -> bool {
    model.starts_with("gpt-5") || model.starts_with('o')
}

pub(crate) fn request_body(messages: &[Message], opts: &ChatOptions) -> Value {
    let (system, rest) = split_system(messages, opts.system.as_deref());
    let model = opts.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    if let Some(s) = system {
        body.insert("instructions".into(), json!(s));
    }
    if let Some(n) = opts.max_tokens {
        body.insert("max_output_tokens".into(), json!(n));
    }
    if let (Some(e), true) = (opts.effort, reasoning_model(&model)) {
        let effort = match e {
            Effort::Low => "low",
            Effort::Medium => "medium",
            _ => "high",
        };
        body.insert("reasoning".into(), json!({"effort": effort}));
    }
    if !opts.tools.is_empty() {
        let tools: Vec<Value> = opts
            .tools
            .iter()
            .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.parameters, "strict": false}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("input".into(), Value::Array(rest.iter().flat_map(to_items).collect()));
    body.insert("stream".into(), json!(true));
    Value::Object(body)
}

/// One neutral message can be several Responses input items.
fn to_items(m: &Message) -> Vec<Value> {
    let mut items = Vec::new();
    let mut text = String::new();
    for p in &m.parts {
        match p {
            Part::Text { text: t } => text.push_str(t),
            Part::ToolCall { id, name, input } => items.push(json!({
                "type": "function_call",
                "call_id": id,
                "name": name,
                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
            })),
            Part::ToolResult { call_id, content, .. } => {
                items.push(json!({"type": "function_call_output", "call_id": call_id, "output": content}))
            }
        }
    }
    if !text.is_empty() {
        let item = if m.role == Role::Assistant {
            json!({"role": "assistant", "content": [{"type": "output_text", "text": text}]})
        } else {
            json!({"role": "user", "content": [{"type": "input_text", "text": text}]})
        };
        items.insert(0, item);
    }
    items
}

#[derive(Default)]
pub(crate) struct Accumulator {
    pub parts: Vec<Part>,
    pub usage: Usage,
    pub stop: Option<StopReason>,
    pub model: String,
    pub error: Option<String>,
    pub done: bool,
}

impl Accumulator {
    pub fn apply(&mut self, ev: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        match ev["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => out.push(StreamEvent::TextDelta(ev["delta"].as_str().unwrap_or("").into())),
            "response.reasoning_summary_text.delta" => {
                out.push(StreamEvent::ThinkingDelta(ev["delta"].as_str().unwrap_or("").into()))
            }
            "response.output_item.done" => {
                let item = &ev["item"];
                match item["type"].as_str() {
                    Some("function_call") => {
                        let id = item["call_id"].as_str().or(item["id"].as_str()).unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        let input = parse_args(&item["arguments"]);
                        self.parts.push(Part::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() });
                        self.stop = Some(StopReason::ToolUse);
                        out.push(StreamEvent::ToolCall { id, name, input });
                    }
                    Some("message") => {
                        let text: String = item["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|c| c["type"] == "output_text")
                            .filter_map(|c| c["text"].as_str())
                            .collect();
                        if !text.is_empty() {
                            self.parts.push(Part::Text { text });
                        }
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.incomplete" => {
                let r = &ev["response"];
                if let Some(m) = r["model"].as_str() {
                    self.model = m.to_string();
                }
                self.usage = Usage {
                    input_tokens: r["usage"]["input_tokens"].as_u64().unwrap_or(0),
                    output_tokens: r["usage"]["output_tokens"].as_u64().unwrap_or(0),
                };
                match r["incomplete_details"]["reason"].as_str() {
                    Some("max_output_tokens") => self.stop = Some(StopReason::MaxTokens),
                    Some("content_filter") => self.stop = Some(StopReason::Refusal),
                    _ => {}
                }
                self.done = true;
            }
            "response.failed" | "error" => {
                self.error = Some(
                    ev["response"]["error"]["message"]
                        .as_str()
                        .or(ev["message"].as_str())
                        .or(ev["error"]["message"].as_str())
                        .unwrap_or("openai stream failed")
                        .to_string(),
                );
            }
            _ => {}
        }
        out
    }
}

fn parse_args(raw: &Value) -> Value {
    match raw {
        Value::String(s) if s.trim().is_empty() => json!({}),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) => v,
            _ => json!({ super::anthropic::INVALID_JSON_KEY: s }),
        },
        Value::Object(_) => raw.clone(),
        _ => json!({}),
    }
}

#[async_trait]
impl Provider for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    async fn chat(&self, messages: &[Message], opts: &ChatOptions, on_event: EventSink<'_>) -> Result<Completion, ProviderError> {
        let body = request_body(messages, opts);
        let url = format!("{}/responses", self.base());
        let key = self.cfg.api_key.clone();
        let resp = http::send("openai", || {
            let rb = CLIENT.post(&url).json(&body);
            if key.is_empty() { rb } else { rb.bearer_auth(&key) }
        })
        .await?;

        let mut acc = Accumulator { model: body["model"].as_str().unwrap_or(DEFAULT_MODEL).into(), ..Default::default() };
        sse::read_events(resp.bytes_stream(), |ev| {
            let Ok(data) = serde_json::from_str::<Value>(&ev.data) else { return true };
            for e in acc.apply(&data) {
                on_event(e);
            }
            acc.error.is_none() && !acc.done
        })
        .await
        .map_err(ProviderError::Network)?;

        if let Some(err) = acc.error {
            return Err(ProviderError::Protocol(format!("openai: {err}")));
        }
        Ok(Completion {
            stop: acc.stop.unwrap_or(StopReason::EndTurn),
            usage: acc.usage,
            message: Message { role: Role::Assistant, parts: acc.parts, raw: None },
            model: acc.model,
            refusal_category: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let url = format!("{}/models", self.base());
        let key = self.cfg.api_key.clone();
        let resp = http::send("openai", || {
            let rb = CLIENT.get(&url);
            if key.is_empty() { rb } else { rb.bearer_auth(&key) }
        })
        .await?;
        let v: Value = resp.json().await.map_err(|e| ProviderError::Protocol(e.to_string()))?;
        let mut out: Vec<String> =
            v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)).collect();
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_and_call_ids() {
        let msgs = vec![
            Message::user("go"),
            Message { role: Role::Assistant, parts: vec![Part::Text { text: "ok".into() }, Part::ToolCall { id: "call_1".into(), name: "t".into(), input: json!({"a": 1}) }], raw: None },
            Message::tool_results(vec![Part::ToolResult { call_id: "call_1".into(), content: "done".into(), is_error: false }]),
        ];
        let b = request_body(&msgs, &ChatOptions { system: Some("s".into()), effort: Some(Effort::Max), ..Default::default() });
        assert_eq!(b["instructions"], "s");
        assert_eq!(b["reasoning"]["effort"], "high");
        let input = b["input"].as_array().unwrap();
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["arguments"], "{\"a\":1}");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
    }

    #[test]
    fn stream_uses_call_id_not_item_id() {
        let mut acc = Accumulator::default();
        acc.apply(&json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc_item","call_id":"call_real","name":"x","arguments":"{\"q\":2}"}}));
        acc.apply(&json!({"type":"response.completed","response":{"model":"gpt-5.6","usage":{"input_tokens":3,"output_tokens":4}}}));
        assert_eq!(acc.parts[0], Part::ToolCall { id: "call_real".into(), name: "x".into(), input: json!({"q": 2}) });
        assert_eq!(acc.usage.output_tokens, 4);
        assert!(acc.done);
    }
}
