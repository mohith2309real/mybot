//! HTTP plumbing shared by the adapters: one client, retries, error bodies.

use std::time::Duration;

use once_cell::sync::Lazy;
use serde_json::Value;

use super::ProviderError;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        // No overall timeout: a long agent turn streams for minutes. A stalled
        // stream is caught by the read timeout instead.
        .read_timeout(Duration::from_secs(300))
        .user_agent(concat!("mybot/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
});

const MAX_RETRIES: u32 = 2;

fn retryable(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500..=599)
}

/// Send, retrying rate limits, overloads and connection failures with
/// backoff — but only before any response body has been read, so a retry can
/// never duplicate output the caller already showed.
pub async fn send(
    provider: &'static str,
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, ProviderError> {
    let mut attempt = 0;
    loop {
        let result = build().send().await;
        match result {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                if retryable(status) && attempt < MAX_RETRIES {
                    attempt += 1;
                    let wait = retry_after.map(Duration::from_secs).unwrap_or_else(|| backoff(attempt));
                    tokio::time::sleep(wait.min(Duration::from_secs(30))).await;
                    continue;
                }
                let body = resp.text().await.unwrap_or_default();
                return Err(ProviderError::Http { provider, status, message: error_message(&body) });
            }
            Err(e) => {
                if attempt < MAX_RETRIES && (e.is_connect() || e.is_timeout()) {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                return Err(ProviderError::Network(format!("{provider}: {e}")));
            }
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * 2u64.pow(attempt))
}

/// Pull a readable message out of whatever error JSON the provider sent.
pub fn error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        for path in [&["error", "message"][..], &["message"][..], &["error"][..]] {
            let mut cur = &v;
            let mut ok = true;
            for k in path {
                match cur.get(k) {
                    Some(n) => cur = n,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if let Some(s) = cur.as_str() {
                    return s.to_string();
                }
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() { "no response body".into() } else { trimmed.chars().take(300).collect() }
}

pub fn base(url: &Option<String>, default: &str) -> String {
    url.as_deref().unwrap_or(default).trim_end_matches('/').to_string()
}
