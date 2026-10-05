//! Gemini, over `streamGenerateContent?alt=sse`.
//!
//!  - Roles are `user` / `model`.
//!  - A `functionResponse` is keyed by function *name*, so the originating
//!    call is looked up to recover it. Ids are often absent; a synthetic one
//!    (`gemini_call_N`) is minted and never sent back as if it were real.
//!  - Model turns are replayed exactly as received, because Gemini 3 attaches
//!    a `thoughtSignature` to function-call parts and rejects a history that
//!    drops it.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::http::{self, CLIENT};
use super::{EventSink, Provider, ProviderConfig, ProviderError};
use crate::model::*;
use crate::sse;

pub const DEFAULT_MODEL: &str = "gemini-3.6-flash";
const SYNTHETIC_PREFIX: &str = "gemini_call_";

pub struct Gemini {
    cfg: ProviderConfig,
}

impl Gemini {
    pub fn new(cfg: ProviderConfig) -> Self {
        Self { cfg }
    }

    fn base(&self) -> String {
        http::base(&self.cfg.base_url, "https://generativelanguage.googleapis.com")
    }
}

pub(crate) fn request_body(messages: &[Message], opts: &ChatOptions) -> Value {
    let (system, rest) = split_system(messages, opts.system.as_deref());
    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(rest.iter().map(|m| to_content(m, messages)).collect()));
    if let Some(s) = system {
        body.insert("systemInstruction".into(), json!({"parts": [{"text": s}]}));
    }
    let mut config = Map::new();
    if let Some(n) = opts.max_tokens {
        config.insert("maxOutputTokens".into(), json!(n));
    }
    config.insert("thinkingConfig".into(), json!({"includeThoughts": true}));
    body.insert("generationConfig".into(), Value::Object(config));
    if !opts.tools.is_empty() {
        let decls: Vec<Value> = opts
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "parametersJsonSchema": t.parameters}))
            .collect();
        body.insert("tools".into(), json!([{"functionDeclarations": decls}]));
    }
    Value::Object(body)
}

fn to_content(m: &Message, all: &[Message]) -> Value {
    if m.role == Role::Assistant {
        if let Some(raw) = m.raw.as_ref().filter(|r| r.provider == "gemini") {
            return json!({"role": "model", "parts": raw.content});
        }
    }
    let parts: Vec<Value> = m
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } if text.is_empty() => None,
            Part::Text { text } => Some(json!({"text": text})),
            Part::ToolCall { id, name, input } => {
                let mut fc = json!({"name": name, "args": input});
                if !id.starts_with(SYNTHETIC_PREFIX) {
                    fc["id"] = json!(id);
                }
                Some(json!({"functionCall": fc}))
            }
            Part::ToolResult { call_id, content, is_error } => {
                let response = if *is_error { json!({"error": content}) } else { json!({"result": content}) };
                let mut fr = json!({"name": tool_name_for_call(all, call_id), "response": response});
                if !call_id.starts_with(SYNTHETIC_PREFIX) {
                    fr["id"] = json!(call_id);
                }
                Some(json!({"functionResponse": fr}))
            }
        })
        .collect();
    let role = if m.role == Role::Assistant { "model" } else { "user" };
    json!({"role": role, "parts": parts})
}

#[derive(Default)]
pub(crate) struct Accumulator {
    pub raw_parts: Vec<Value>,
    pub parts: Vec<Part>,
    pub usage: Usage,
    pub stop: Option<StopReason>,
    pub model: String,
    seq: u32,
}

impl Accumulator {
    pub fn apply(&mut self, chunk: &Value) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        if chunk["promptFeedback"]["blockReason"].is_string() {
            self.stop = Some(StopReason::Refusal);
        }
        let cand = &chunk["candidates"][0];
        for p in cand["content"]["parts"].as_array().into_iter().flatten() {
            self.raw_parts.push(p.clone());
            if let Some(t) = p["text"].as_str() {
                if t.is_empty() {
                    continue;
                }
                if p["thought"].as_bool() == Some(true) {
                    out.push(StreamEvent::ThinkingDelta(t.to_string()));
                } else {
                    match self.parts.last_mut() {
                        Some(Part::Text { text }) => text.push_str(t),
                        _ => self.parts.push(Part::Text { text: t.to_string() }),
                    }
                    out.push(StreamEvent::TextDelta(t.to_string()));
                }
            }
            if let Some(fc) = p.get("functionCall") {
                self.seq += 1;
                let id = fc["id"].as_str().map(String::from).unwrap_or_else(|| format!("{SYNTHETIC_PREFIX}{}", self.seq));
                let name = fc["name"].as_str().unwrap_or("unknown_tool").to_string();
                let input = if fc["args"].is_object() { fc["args"].clone() } else { json!({}) };
                self.parts.push(Part::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() });
                self.stop = Some(StopReason::ToolUse);
                out.push(StreamEvent::ToolCall { id, name, input });
            }
        }
        if let Some(u) = chunk.get("usageMetadata") {
            self.usage = Usage {
                input_tokens: u["promptTokenCount"].as_u64().unwrap_or(0),
                output_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) + u["thoughtsTokenCount"].as_u64().unwrap_or(0),
            };
        }
        if let Some(v) = chunk["modelVersion"].as_str() {
            self.model = v.to_string();
        }
        match cand["finishReason"].as_str() {
            Some("MAX_TOKENS") => self.stop = Some(StopReason::MaxTokens),
            Some("SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" | "RECITATION") => self.stop = Some(StopReason::Refusal),
            _ => {}
        }
        out
    }
}

#[async_trait]
impl Provider for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    async fn chat(&self, messages: &[Message], opts: &ChatOptions, on_event: EventSink<'_>) -> Result<Completion, ProviderError> {
        let model = opts.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into());
        let body = request_body(messages, opts);
        let url = format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", self.base(), model);
        let key = self.cfg.api_key.clone();
        let resp = http::send("gemini", || CLIENT.post(&url).header("x-goog-api-key", &key).json(&body)).await?;

        let mut acc = Accumulator { model: model.clone(), ..Default::default() };
        let mut error = None;
        sse::read_events(resp.bytes_stream(), |ev| {
            let Ok(data) = serde_json::from_str::<Value>(&ev.data) else { return true };
            if let Some(msg) = data["error"]["message"].as_str() {
                error = Some(msg.to_string());
                return false;
            }
            for e in acc.apply(&data) {
                on_event(e);
            }
            true
        })
        .await
        .map_err(ProviderError::Network)?;
        if let Some(e) = error {
            return Err(ProviderError::Protocol(format!("gemini: {e}")));
        }

        Ok(Completion {
            stop: acc.stop.unwrap_or(StopReason::EndTurn),
            usage: acc.usage,
            message: Message {
                role: Role::Assistant,
                parts: acc.parts,
                raw: Some(RawTurn { provider: "gemini".into(), content: Value::Array(acc.raw_parts) }),
            },
            model: acc.model,
            refusal_category: None,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let url = format!("{}/v1beta/models?pageSize=1000", self.base());
        let key = self.cfg.api_key.clone();
        let resp = http::send("gemini", || CLIENT.get(&url).header("x-goog-api-key", &key)).await?;
        let v: Value = resp.json().await.map_err(|e| ProviderError::Protocol(e.to_string()))?;
        let mut out: Vec<String> = v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_none_or(|a| a.iter().any(|x| x == "generateContent"))
            })
            .filter_map(|m| m["name"].as_str().map(|n| n.trim_start_matches("models/").to_string()))
            .collect();
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_response_by_name_and_signature_replay() {
        let mut acc = Accumulator::default();
        acc.apply(&json!({"candidates":[{"content":{"role":"model","parts":[
            {"text":"thinking…","thought":true},
            {"functionCall":{"name":"page_read","args":{}},"thoughtSignature":"SIG"}
        ]}}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"thoughtsTokenCount":3}}));
        assert_eq!(acc.usage.output_tokens, 5);
        let call_id = match &acc.parts[0] { Part::ToolCall { id, .. } => id.clone(), _ => panic!() };
        assert!(call_id.starts_with(SYNTHETIC_PREFIX));

        let assistant = Message { role: Role::Assistant, parts: acc.parts.clone(), raw: Some(RawTurn { provider: "gemini".into(), content: Value::Array(acc.raw_parts.clone()) }) };
        let msgs = vec![
            Message::user("go"),
            assistant,
            Message::tool_results(vec![Part::ToolResult { call_id: call_id.clone(), content: "ok".into(), is_error: false }]),
        ];
        let b = request_body(&msgs, &ChatOptions::default());
        assert_eq!(b["contents"][1]["role"], "model");
        assert_eq!(b["contents"][1]["parts"][1]["thoughtSignature"], "SIG");
        let fr = &b["contents"][2]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "page_read");
        assert!(fr.get("id").is_none(), "a synthetic id is never sent as real");
    }
}
