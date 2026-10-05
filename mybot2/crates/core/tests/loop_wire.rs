//! The agent loop against a stub HTTP server speaking each provider's SSE.
//! No API keys, no network: this checks the real request bytes, the real
//! stream parsing, and the loop's turn-taking together.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mybot_core::agent::*;
use mybot_core::cancel::Cancel;
use mybot_core::db::Db;
use mybot_core::model::*;
use mybot_core::providers::{self, ProviderConfig};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve `responses` in order (one per request) as SSE; record each request body.
async fn stub(responses: Vec<String>) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        for body in responses {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // Read headers + body (content-length).
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = find(&buf, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    while buf.len() < pos + 4 + len {
                        let n = sock.read(&mut tmp).await.unwrap();
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let req_body: Value = serde_json::from_slice(&buf[pos + 4..pos + 4 + len]).unwrap_or(Value::Null);
                    seen2.lock().unwrap().push((head, req_body));
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.shutdown().await.ok();
        }
    });
    (format!("http://{addr}"), seen)
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap_or("message"), e)).collect()
}

struct FakeTools {
    calls: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl ToolHost for FakeTools {
    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef { name: "page_read".into(), description: "read".into(), parameters: json!({"type":"object","properties":{}}) },
            ToolDef { name: "task_done".into(), description: "done".into(), parameters: json!({"type":"object","properties":{"summary":{"type":"string"}}}) },
        ]
    }
    async fn run(&self, name: &str, input: &Value, _ctx: &RunCtx) -> ToolOutcome {
        self.calls.lock().unwrap().push((name.to_string(), input.clone()));
        match name {
            "page_read" => ToolOutcome::ok("URL: https://example.com\nHeading: Example Domain"),
            "task_done" => ToolOutcome { content: "Task marked complete.".into(), done: Some(input["summary"].as_str().unwrap_or("").into()), ..Default::default() },
            _ => ToolOutcome::err("unknown"),
        }
    }
}

struct NoHuman;
#[async_trait]
impl Handoff for NoHuman {
    async fn wait(&self, _: Option<&str>, _: &Pause, _: Duration, _: &Cancel) -> Resume {
        Resume::TimedOut
    }
}

fn opts(provider: Arc<dyn providers::Provider>, db: &Db, task: &str) -> RunOptions {
    RunOptions {
        provider,
        model: None,
        effort: Some(Effort::High),
        system: BASE_SYSTEM.into(),
        history: vec![],
        instruction: "What is the heading on example.com?".into(),
        max_iterations: 5,
        pause_timeout: Duration::from_secs(1),
        ctx: RunCtx { bot: "scout".into(), task_id: Some(task.into()), instruction: String::new(), cancel: Cancel::new() },
        db: Some(db.clone()),
    }
}

#[tokio::test]
async fn anthropic_loop_replays_thinking_and_finishes() {
    let turn1 = sse(&[
        json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":100}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Read the page first."}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"page_read","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}),
        json!({"type":"message_stop"}),
    ]);
    let turn2 = sse(&[
        json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":150}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"task_done","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"summary\":\"It says Example Domain.\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":15}}),
        json!({"type":"message_stop"}),
    ]);
    let (url, seen) = stub(vec![turn1, turn2]).await;
    let provider: Arc<dyn providers::Provider> =
        Arc::from(providers::make("anthropic", ProviderConfig { api_key: "sk-test".into(), base_url: Some(url) }).unwrap());

    let db = Db::in_memory().unwrap();
    let bot = db.create_bot("scout", "anthropic", "claude-opus-5-5", None).unwrap();
    let task = db.create_task(&bot.id, "What is the heading on example.com?").unwrap();
    let tools = FakeTools { calls: Mutex::new(vec![]) };
    let events = Mutex::new(Vec::new());
    let result = run_task(opts(provider, &db, &task.id), &tools, &NoHuman, &|e| events.lock().unwrap().push(e)).await;

    assert_eq!(result.status, RunStatus::Done);
    assert_eq!(result.summary, "It says Example Domain.");
    assert_eq!(result.usage, Usage { input_tokens: 250, output_tokens: 35 });
    assert_eq!(tools.calls.lock().unwrap().len(), 2);

    let seen = seen.lock().unwrap();
    let (head1, body1) = &seen[0];
    assert!(head1.contains("x-api-key: sk-test"));
    assert!(head1.contains("anthropic-version: 2023-06-01"));
    assert!(head1.contains("anthropic-beta: server-side-fallback-2026-07-01"));
    assert_eq!(body1["model"], "claude-opus-5-5");
    assert_eq!(body1["output_config"]["effort"], "high");
    assert_eq!(body1["stream"], true);

    // The second request replays turn one exactly, thinking signature included,
    // and answers the tool call in a single user turn.
    let (_, body2) = &seen[1];
    let msgs = body2["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["content"][0], json!({"type":"thinking","thinking":"Read the page first.","signature":"sig-abc"}));
    assert_eq!(msgs[1]["content"][1]["id"], "toolu_1");
    assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_1");

    let t = db.task(&task.id).unwrap().unwrap();
    assert_eq!(t.status, "done");
    assert_eq!(t.output_tokens, 35);
    let kinds: Vec<String> = db.log(&task.id).unwrap().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds, ["user", "tool_call", "tool_result", "tool_call", "tool_result", "done"]);
    assert!(events.lock().unwrap().contains(&AgentEvent::ThinkingDelta("Read the page first.".into())));
}

#[tokio::test]
async fn anthropic_refusal_and_cut_off_tool_calls_never_run() {
    let refusal = sse(&[
        json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":5}}}),
        json!({"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}},"usage":{"output_tokens":0}}),
        json!({"type":"message_stop"}),
    ]);
    let cut_off = sse(&[
        json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":5}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_x","name":"page_read","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":9}}),
        json!({"type":"message_stop"}),
    ]);
    let then_done = sse(&[
        json!({"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":5}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done."}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
        json!({"type":"message_stop"}),
    ]);
    let (url, seen) = stub(vec![refusal, cut_off, then_done]).await;
    let provider: Arc<dyn providers::Provider> =
        Arc::from(providers::make("anthropic", ProviderConfig { api_key: "k".into(), base_url: Some(url) }).unwrap());
    let db = Db::in_memory().unwrap();
    let bot = db.create_bot("scout", "anthropic", "claude-opus-5-5", None).unwrap();

    let t1 = db.create_task(&bot.id, "x").unwrap();
    let tools = FakeTools { calls: Mutex::new(vec![]) };
    let r = run_task(opts(provider.clone(), &db, &t1.id), &tools, &NoHuman, &|_| {}).await;
    assert_eq!(r.status, RunStatus::Refused);
    assert!(r.summary.contains("cyber"));

    let t2 = db.create_task(&bot.id, "y").unwrap();
    let r = run_task(opts(provider, &db, &t2.id), &tools, &NoHuman, &|_| {}).await;
    assert_eq!(r.status, RunStatus::Done);
    assert!(tools.calls.lock().unwrap().is_empty(), "a call cut off at max_tokens must not run");
    let (_, body) = &seen.lock().unwrap()[2];
    let last = body["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["content"][0]["is_error"], true);
}

#[tokio::test]
async fn openai_loop_uses_call_id() {
    let turn1 = sse(&[
        json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_A","name":"page_read","arguments":"{}"}}),
        json!({"type":"response.completed","response":{"model":"gpt-5.6","usage":{"input_tokens":10,"output_tokens":5}}}),
    ]);
    let turn2 = sse(&[
        json!({"type":"response.output_text.delta","delta":"Example Domain"}),
        json!({"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Example Domain"}]}}),
        json!({"type":"response.completed","response":{"model":"gpt-5.6","usage":{"input_tokens":12,"output_tokens":3}}}),
    ]);
    let (url, seen) = stub(vec![turn1, turn2]).await;
    let provider: Arc<dyn providers::Provider> =
        Arc::from(providers::make("openai", ProviderConfig { api_key: "sk".into(), base_url: Some(url) }).unwrap());
    let db = Db::in_memory().unwrap();
    let bot = db.create_bot("g", "openai", "gpt-5.6", None).unwrap();
    let task = db.create_task(&bot.id, "x").unwrap();
    let tools = FakeTools { calls: Mutex::new(vec![]) };
    let r = run_task(opts(provider, &db, &task.id), &tools, &NoHuman, &|_| {}).await;
    assert_eq!(r.status, RunStatus::Done);
    assert_eq!(r.summary, "Example Domain");
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.contains("POST /responses"));
    assert!(seen[0].0.to_ascii_lowercase().contains("authorization: bearer sk"));
    let input = seen[1].1["input"].as_array().unwrap();
    let out = input.iter().find(|i| i["type"] == "function_call_output").unwrap();
    assert_eq!(out["call_id"], "call_A");
}

#[tokio::test]
async fn gemini_loop_keys_results_by_name() {
    let turn1 = format!(
        "data: {}\n\n",
        json!({"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"page_read","args":{}},"thoughtSignature":"TS"}]}}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":1}})
    );
    let turn2 = format!(
        "data: {}\n\n",
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Example Domain"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":6,"candidatesTokenCount":2}})
    );
    let (url, seen) = stub(vec![turn1, turn2]).await;
    let provider: Arc<dyn providers::Provider> =
        Arc::from(providers::make("gemini", ProviderConfig { api_key: "gk".into(), base_url: Some(url) }).unwrap());
    let db = Db::in_memory().unwrap();
    let bot = db.create_bot("gm", "gemini", "gemini-3.6-flash", None).unwrap();
    let task = db.create_task(&bot.id, "x").unwrap();
    let tools = FakeTools { calls: Mutex::new(vec![]) };
    let r = run_task(opts(provider, &db, &task.id), &tools, &NoHuman, &|_| {}).await;
    assert_eq!(r.status, RunStatus::Done);
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.contains(":streamGenerateContent?alt=sse"));
    assert!(seen[0].0.contains("x-goog-api-key: gk"));
    let contents = seen[1].1["contents"].as_array().unwrap();
    assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "TS");
    assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "page_read");
}
