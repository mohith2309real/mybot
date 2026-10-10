//! Claude, over the Messages API with SSE streaming (raw HTTP: there is no
//! official Rust SDK).
//!
//! Things this adapter does on purpose, so they don't get "simplified" away:
//!  - Adaptive thinking with `display: "summarized"`. The default display is
//!    "omitted", which streams empty thinking and makes a working bot look
//!    stalled. `budget_tokens` is a 400 on current models.
//!  - Effort goes in `output_config.effort`. Claude Opus 5.5 defaults to
//!    `medium`, so agent runs set it explicitly.
//!  - The assistant turn is kept exactly as received (thinking blocks and their
//!    signatures included) and replayed unchanged. Thinking blocks are bound to
//!    the conversation; rebuilding the turn from neutral parts would drop them.
//!  - A refusal is HTTP 200 with `stop_reason: "refusal"`. Server-side
//!    fallbacks (`fallbacks: "default"`) are on for the models that take them,
//!    so a decline mid-run is re-served instead of stranding an unattended bot.
//!    After a mid-output fallback, blocks before the last `fallback` marker are
//!    replayed as text only, and tool calls from before it are never run.
//!  - Tools stream eagerly, so tool input is parsed strictly at block end; a
//!    malformed input becomes an INVALID_JSON error result, never a run.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::http::{self, CLIENT};
use super::{EventSink, Provider, ProviderConfig, ProviderError};
use crate::model::*;
use crate::sse;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Marker key on a tool call whose streamed input was not valid JSON.
pub const INVALID_JSON_KEY: &str = "__invalid_json__";

pub struct Anthropic {
    cfg: ProviderConfig,
}

impl Anthropic {
    pub fn new(cfg: ProviderConfig) -> Self {
        Self { cfg }
    }

    fn base(&self) -> String {
        http::base(&self.cfg.base_url, "https://api.anthropic.com")
    }
}

/// Models that take adaptive thinking and `output_config.effort`. Haiku 4.5
/// and older generations do not.
fn adaptive(model: &str) -> bool {
    !(model.contains("haiku") || model.starts_with("claude-3") || model.contains("-4-5") || model.contains("-4-1") || model.ends_with("-4-0") || model == "claude-opus-4" || model == "claude-sonnet-4")
}

/// Models that accept `fallbacks: "default"`.
fn takes_fallbacks(model: &str) -> bool {
    matches!(model, "claude-fable-5-1" | "claude-fable-5" | "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5")
}

pub(crate) fn request_body(messages: &[Message], opts: &ChatOptions) -> Value {
    let (system, rest) = split_system(messages, opts.system.as_deref());
    let model = opts.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("max_tokens".into(), json!(opts.max_tokens.unwrap_or(64_000)));
    body.insert("stream".into(), json!(true));
    if adaptive(&model) {
        body.insert("thinking".into(), json!({"type": "adaptive", "display": "summarized"}));
        if let Some(e) = opts.effort {
            body.insert("output_config".into(), json!({"effort": e.as_str()}));
        }
    }
    if let Some(s) = system {
        body.insert("system".into(), json!(s));
    }
    if !opts.tools.is_empty() {
        let tools: Vec<Value> = opts
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                    "eager_input_streaming": true,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("messages".into(), Value::Array(rest.iter().map(to_wire).collect()));
    if takes_fallbacks(&model) {
        body.insert("fallbacks".into(), json!("default"));
    }
    Value::Object(body)
}

fn to_wire(m: &Message) -> Value {
    if m.role == Role::Assistant {
        if let Some(raw) = m.raw.as_ref().filter(|r| r.provider == "anthropic") {
            return json!({"role": "assistant", "content": raw.content});
        }
    }
    let blocks: Vec<Value> = m
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } if text.is_empty() => None,
            Part::Text { text } => Some(json!({"type": "text", "text": text})),
            Part::ToolCall { id, name, input } => {
                Some(json!({"type": "tool_use", "id": id, "name": name, "input": clean_input(input)}))
            }
            Part::ToolResult { call_id, content, is_error } => {
                let mut b = json!({"type": "tool_result", "tool_use_id": call_id, "content": content});
                if *is_error {
                    b["is_error"] = json!(true);
                }
                Some(b)
            }
        })
        .collect();
    let role = if m.role == Role::Assistant { "assistant" } else { "user" };
    json!({"role": role, "content": blocks})
}

fn clean_input(input: &Value) -> Value {
    match input {
        Value::Object(o) if o.contains_key(INVALID_JSON_KEY) => json!({}),
        Value::Object(_) => input.clone(),
        _ => json!({}),
    }
}

/// Accumulates one streamed message.
#[derive(Default)]
pub(crate) struct Accumulator {
    blocks: Vec<Value>,
    partial_json: Vec<String>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub refusal_category: Option<String>,
    pub usage: Usage,
    pub error: Option<String>,
}

impl Accumulator {
    fn slot(&mut self, index: usize) {
        while self.blocks.len() <= index {
            self.blocks.push(Value::Null);
            self.partial_json.push(String::new());
        }
    }

    /// Apply one SSE event. Returns the stream events it produced.
    pub fn apply(&mut self, data: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        match data["type"].as_str().unwrap_or("") {
            "message_start" => {
                let m = &data["message"];
                if let Some(model) = m["model"].as_str() {
                    self.model = model.to_string();
                }
                self.usage.input_tokens = token_count(&m["usage"]);
            }
            "content_block_start" => {
                let i = data["index"].as_u64().unwrap_or(0) as usize;
                self.slot(i);
                let mut block = data["content_block"].clone();
                if block["type"] == "tool_use" {
                    block["input"] = json!({});
                }
                self.blocks[i] = block;
            }
            "content_block_delta" => {
                let i = data["index"].as_u64().unwrap_or(0) as usize;
                self.slot(i);
                let d = &data["delta"];
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or("");
                        append(&mut self.blocks[i], "text", t);
                        out.push(StreamEvent::TextDelta(t.to_string()));
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or("");
                        append(&mut self.blocks[i], "thinking", t);
                        if !t.is_empty() {
                            out.push(StreamEvent::ThinkingDelta(t.to_string()));
                        }
                    }
                    "signature_delta" => {
                        append(&mut self.blocks[i], "signature", d["signature"].as_str().unwrap_or(""));
                    }
                    "input_json_delta" => {
                        self.partial_json[i].push_str(d["partial_json"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let i = data["index"].as_u64().unwrap_or(0) as usize;
                self.slot(i);
                if self.blocks[i]["type"] == "tool_use" {
                    let raw = std::mem::take(&mut self.partial_json[i]);
                    let input = if raw.trim().is_empty() {
                        json!({})
                    } else {
                        match serde_json::from_str::<Value>(&raw) {
                            Ok(v @ Value::Object(_)) => v,
                            _ => json!({ INVALID_JSON_KEY: raw }),
                        }
                    };
                    self.blocks[i]["input"] = input;
                }
            }
            "message_delta" => {
                if let Some(r) = data["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
                if let Some(c) = data["delta"]["stop_details"]["category"].as_str() {
                    self.refusal_category = Some(c.to_string());
                }
                let out_tokens = data["usage"]["output_tokens"].as_u64();
                if let Some(n) = out_tokens {
                    self.usage.output_tokens = n;
                }
                if let Some(n) = data["usage"]["input_tokens"].as_u64() {
                    if n > 0 {
                        self.usage.input_tokens = n
                            + data["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0)
                            + data["usage"]["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    }
                }
            }
            "error" => {
                self.error = Some(
                    data["error"]["message"].as_str().unwrap_or("the stream reported an error").to_string(),
                );
            }
            _ => {}
        }
        out
    }

    /// The turn to replay, applying the fallback rule, and the neutral parts.
    pub fn finish(self) -> (Value, Vec<Part>) {
        let blocks: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        let boundary = blocks.iter().rposition(|b| b["type"] == "fallback");

        let mut echo = Vec::new();
        for (i, b) in blocks.into_iter().enumerate() {
            let ty = b["type"].as_str().unwrap_or("").to_string();
            if ty == "fallback" {
                continue;
            }
            let before = boundary.is_some_and(|f| i < f);
            if before && ty != "text" {
                continue;
            }
            if ty == "text" && b["text"].as_str().is_none_or(str::is_empty) {
                continue;
            }
            echo.push(b);
        }

        let mut parts = Vec::new();
        for b in &echo {
            match b["type"].as_str() {
                Some("text") => {
                    let t = b["text"].as_str().unwrap_or("").to_string();
                    match parts.last_mut() {
                        Some(Part::Text { text }) => text.push_str(&t),
                        _ => parts.push(Part::Text { text: t }),
                    }
                }
                Some("tool_use") => parts.push(Part::ToolCall {
                    id: b["id"].as_str().unwrap_or("").to_string(),
                    name: b["name"].as_str().unwrap_or("").to_string(),
                    input: b["input"].clone(),
                }),
                _ => {}
            }
        }

        // The replayed copy must carry a valid object even where the stream
        // was malformed; the neutral part keeps the marker for the loop.
        let echo: Vec<Value> = echo
            .into_iter()
            .map(|mut b| {
                if b["type"] == "tool_use" {
                    b["input"] = clean_input(&b["input"]);
                }
                b
            })
            .collect();
        (Value::Array(echo), parts)
    }
}

fn append(block: &mut Value, key: &str, s: &str) {
    let cur = block[key].as_str().unwrap_or("").to_string();
    block[key] = json!(cur + s);
}

fn token_count(u: &Value) -> u64 {
    u["input_tokens"].as_u64().unwrap_or(0)
        + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
        + u["cache_creation_input_tokens"].as_u64().unwrap_or(0)
}

fn map_stop(r: Option<&str>) -> StopReason {
    match r {
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("refusal") => StopReason::Refusal,
        _ => StopReason::EndTurn,
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    async fn chat(
        &self,
        messages: &[Message],
        opts: &ChatOptions,
        on_event: EventSink<'_>,
    ) -> Result<Completion, ProviderError> {
        let body = request_body(messages, opts);
        let model = body["model"].as_str().unwrap_or(DEFAULT_MODEL).to_string();
        let url = format!("{}/v1/messages", self.base());
        let key = self.cfg.api_key.clone();
        let fallbacks = body.get("fallbacks").is_some();

        let resp = http::send("anthropic", || {
            let mut rb = CLIENT
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&body);
            if fallbacks {
                rb = rb.header("anthropic-beta", FALLBACK_BETA);
            }
            rb
        })
        .await?;

        let mut acc = Accumulator { model: model.clone(), ..Default::default() };
        sse::read_events(resp.bytes_stream(), |ev| {
            if ev.data.is_empty() {
                return true;
            }
            let Ok(data) = serde_json::from_str::<Value>(&ev.data) else { return true };
            for e in acc.apply(&data) {
                on_event(e);
            }
            acc.error.is_none() && data["type"] != "message_stop"
        })
        .await
        .map_err(ProviderError::Network)?;

        if let Some(err) = acc.error.take() {
            return Err(ProviderError::Protocol(format!("anthropic: {err}")));
        }

        let stop = map_stop(acc.stop_reason.as_deref());
        let usage = acc.usage;
        let served = acc.model.clone();
        let category = acc.refusal_category.clone();
        let (echo, parts) = acc.finish();

        for p in &parts {
            if let Part::ToolCall { id, name, input } = p {
                on_event(StreamEvent::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() });
            }
        }

        Ok(Completion {
            stop,
            usage,
            message: Message {
                role: Role::Assistant,
                parts,
                raw: Some(RawTurn { provider: "anthropic".into(), content: echo }),
            },
            model: served,
            refusal_category: category,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut url = format!("{}/v1/models?limit=100", self.base());
            if let Some(a) = &after {
                url.push_str(&format!("&after_id={a}"));
            }
            let key = self.cfg.api_key.clone();
            let resp = http::send("anthropic", || {
                CLIENT.get(&url).header("x-api-key", &key).header("anthropic-version", API_VERSION)
            })
            .await?;
            let v: Value = resp.json().await.map_err(|e| ProviderError::Protocol(e.to_string()))?;
            for m in v["data"].as_array().into_iter().flatten() {
                if let Some(id) = m["id"].as_str() {
                    out.push(id.to_string());
                }
            }
            if v["has_more"].as_bool() != Some(true) {
                break;
            }
            after = v["last_id"].as_str().map(String::from);
            if after.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_shape() {
        let opts = ChatOptions {
            system: Some("sys".into()),
            tools: vec![ToolDef { name: "t".into(), description: "d".into(), parameters: json!({"type": "object", "properties": {}}) }],
            effort: Some(Effort::High),
            ..Default::default()
        };
        let b = request_body(&[Message::user("hi")], &opts);
        assert_eq!(b["model"], DEFAULT_MODEL);
        assert_eq!(b["thinking"], json!({"type": "adaptive", "display": "summarized"}));
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["tools"][0]["eager_input_streaming"], true);
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        assert!(b.get("temperature").is_none());
        assert!(b["thinking"].get("budget_tokens").is_none());

        let haiku = request_body(&[Message::user("hi")], &ChatOptions { model: Some("claude-haiku-4-5".into()), effort: Some(Effort::High), ..Default::default() });
        assert!(haiku.get("thinking").is_none() && haiku.get("output_config").is_none() && haiku.get("fallbacks").is_none());
    }

    fn feed(acc: &mut Accumulator, events: &[Value]) -> Vec<StreamEvent> {
        events.iter().flat_map(|e| acc.apply(e)).collect()
    }

    #[test]
    fn stream_keeps_thinking_signature_and_parses_tools() {
        let mut acc = Accumulator::default();
        let evs = feed(&mut acc, &[
            json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":10}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Plan."}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIG"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Let me look."}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"page_read","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"u"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"rl\":\"x\"}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_2","name":"bad","input":{}}}),
            json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"a\": \"unterminated"}}),
            json!({"type":"content_block_stop","index":3}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}),
        ]);
        assert!(evs.contains(&StreamEvent::ThinkingDelta("Plan.".into())));
        assert!(evs.contains(&StreamEvent::TextDelta("Let me look.".into())));
        assert_eq!(acc.usage, Usage { input_tokens: 10, output_tokens: 42 });
        let (echo, parts) = acc.finish();
        assert_eq!(echo[0], json!({"type":"thinking","thinking":"Plan.","signature":"SIG"}));
        assert_eq!(echo[2]["input"], json!({"url": "x"}));
        assert_eq!(echo[3]["input"], json!({}), "replayed copy must be a valid object");
        assert_eq!(parts[0], Part::Text { text: "Let me look.".into() });
        assert!(matches!(&parts[2], Part::ToolCall { input, .. } if input.get(INVALID_JSON_KEY).is_some()));
    }

    #[test]
    fn mid_output_fallback_replays_text_only_before_boundary() {
        let mut acc = Accumulator::default();
        feed(&mut acc, &[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"x","signature":"s"}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"partial"}}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"declined","name":"bash","input":{}}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"content_block_start","index":3,"content_block":{"type":"fallback","from":{"model":"a"},"to":{"model":"b"}}}),
            json!({"type":"content_block_start","index":4,"content_block":{"type":"thinking","thinking":"y","signature":"t"}}),
            json!({"type":"content_block_start","index":5,"content_block":{"type":"tool_use","id":"kept","name":"page_read","input":{}}}),
            json!({"type":"content_block_stop","index":5}),
        ]);
        let (echo, parts) = acc.finish();
        let types: Vec<&str> = echo.as_array().unwrap().iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["text", "thinking", "tool_use"]);
        let calls: Vec<&str> = parts.iter().filter_map(|p| match p { Part::ToolCall { id, .. } => Some(id.as_str()), _ => None }).collect();
        assert_eq!(calls, ["kept"], "a tool call from before the fallback must never run");
    }

    #[test]
    fn replays_raw_turn_verbatim() {
        let raw = json!([{"type":"thinking","thinking":"t","signature":"s"},{"type":"tool_use","id":"a","name":"n","input":{"k":1}}]);
        let m = Message { role: Role::Assistant, parts: vec![], raw: Some(RawTurn { provider: "anthropic".into(), content: raw.clone() }) };
        assert_eq!(to_wire(&m)["content"], raw);
        // Another provider's raw turn is ignored.
        let other = Message { role: Role::Assistant, parts: vec![Part::Text { text: "hi".into() }], raw: Some(RawTurn { provider: "gemini".into(), content: json!([]) }) };
        assert_eq!(to_wire(&other)["content"], json!([{"type":"text","text":"hi"}]));
    }
}
