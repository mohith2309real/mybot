//! The computer: one Docker container per conversation, one desktop (Linux
//! user, X display, browser, live view) per bot inside it.
//!
//! Same image, names and ports as MyBot 1.x — a conversation started in one
//! version opens in the other. The Dockerfile and its scripts are compiled
//! into this binary, so the app can build the image with nothing else on disk.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub const IMAGE: &str = "mybot-desktop:0.2";
pub const AGENTD_PORT: u16 = 7000;
pub const DEFAULT_CONVERSATION: &str = "default";

const DOCKERFILE: &str = include_str!("../../../../docker/Dockerfile");
const AGENTD: &str = include_str!("../../../../docker/agentd.py");
const DESKTOPCTL: &str = include_str!("../../../../docker/desktopctl.sh");
const CDP_RELAY: &str = include_str!("../../../../docker/cdp-relay.py");

#[derive(Debug, Clone, thiserror::Error)]
pub enum ComputerError {
    #[error("Docker is not running. Start Docker Desktop, then try again.")]
    NoDocker,
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, ComputerError>;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

async fn run(program: &str, args: &[&str], timeout: Duration) -> ExecResult {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ExecResult { code: 127, stdout: String::new(), stderr: e.to_string() },
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => ExecResult {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Ok(Err(e)) => ExecResult { code: 127, stdout: String::new(), stderr: e.to_string() },
        Err(_) => ExecResult { code: 124, stdout: String::new(), stderr: format!("[timed out after {}s]", timeout.as_secs()) },
    }
}

async fn docker(args: &[&str]) -> ExecResult {
    run("docker", args, Duration::from_secs(120)).await
}

pub fn container_name(conversation: &str) -> String {
    let clean: String = conversation.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    format!("mybot-conv-{}", clean.to_lowercase())
}

/// Bot names become Unix usernames inside the container.
pub fn normalise_bot(bot: &str) -> String {
    let mut n: String = bot
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    n = n.trim_start_matches('-').chars().take(31).collect();
    if n.is_empty() { "bot".into() } else { n }
}

pub async fn docker_available() -> bool {
    docker(&["info", "--format", "{{.ServerVersion}}"]).await.code == 0
}

pub async fn image_exists() -> bool {
    docker(&["image", "inspect", IMAGE]).await.code == 0
}

/// Write the embedded build context to ~/.mybot/docker-build and build.
pub async fn build_image(on_line: &(dyn Fn(String) + Send + Sync)) -> Result<()> {
    let dir: PathBuf = mybot_vault::home().join("docker-build");
    std::fs::create_dir_all(&dir).map_err(|e| ComputerError::Failed(e.to_string()))?;
    for (name, body) in [("Dockerfile", DOCKERFILE), ("agentd.py", AGENTD), ("desktopctl.sh", DESKTOPCTL), ("cdp-relay.py", CDP_RELAY)] {
        std::fs::write(dir.join(name), body).map_err(|e| ComputerError::Failed(e.to_string()))?;
    }
    let mut child = Command::new("docker")
        .args(["build", "-t", IMAGE, "."])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ComputerError::Failed(format!("docker build: {e}")))?;
    let stderr = child.stderr.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut a = BufReader::new(stdout).lines();
    let mut b = BufReader::new(stderr).lines();
    loop {
        tokio::select! {
            l = a.next_line() => match l { Ok(Some(l)) => on_line(l), _ => break },
            l = b.next_line() => match l { Ok(Some(l)) => on_line(l), _ => {} },
        }
    }
    while let Ok(Some(l)) = b.next_line().await {
        on_line(l);
    }
    let status = child.wait().await.map_err(|e| ComputerError::Failed(e.to_string()))?;
    if !status.success() {
        return Err(ComputerError::Failed(format!("docker build failed ({status})")));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Stopped,
    Absent,
}

pub async fn state(conversation: &str) -> State {
    let r = docker(&["inspect", "-f", "{{.State.Running}}", &container_name(conversation)]).await;
    if r.code != 0 {
        State::Absent
    } else if r.stdout.trim() == "true" {
        State::Running
    } else {
        State::Stopped
    }
}

async fn host_port(name: &str, container_port: u16) -> Option<u16> {
    let r = docker(&["port", name, &container_port.to_string()]).await;
    if r.code != 0 {
        return None;
    }
    r.stdout.lines().next()?.rsplit(':').next()?.trim().parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Computer {
    pub conversation: String,
    pub name: String,
    pub agentd_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desktop {
    pub bot: String,
    pub user: String,
    pub display: u32,
    /// Host port of the desktop's noVNC/websockify endpoint (the live view).
    pub view_port: u16,
    /// Host port for CDP, once a browser runs on this desktop.
    pub cdp_port: Option<u16>,
}

impl Desktop {
    /// The WebSocket the native live view speaks VNC over.
    pub fn vnc_ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/websockify", self.view_port)
    }
    /// The same desktop in a browser, for "open in a window".
    pub fn web_url(&self) -> String {
        format!("http://127.0.0.1:{}/vnc.html?autoconnect=1&resize=scale&reconnect=1", self.view_port)
    }
}

/// Start (or attach to) a conversation's container.
pub async fn ensure(conversation: &str, on_line: &(dyn Fn(String) + Send + Sync)) -> Result<Computer> {
    if !docker_available().await {
        return Err(ComputerError::NoDocker);
    }
    if !image_exists().await {
        on_line("Building the desktop image (first run — a few minutes)…".into());
        build_image(on_line).await?;
    }
    let name = container_name(conversation);
    match state(conversation).await {
        State::Absent => {
            let ws = format!("{name}-ws:/workspace");
            let homes = format!("{name}-homes:/home");
            let label = format!("mybot.conversation={conversation}");
            let agentd = format!("127.0.0.1::{AGENTD_PORT}");
            let mut args: Vec<String> = vec![
                "run".into(), "-d".into(), "--name".into(), name.clone(),
                "--label".into(), "mybot=conversation".into(), "--label".into(), label,
                "-p".into(), agentd,
            ];
            // Desktops are created later, but ports can only be published now:
            // publish a band up front.
            for d in 1..=8u16 {
                args.push("-p".into());
                args.push(format!("127.0.0.1::{}", 6080 + d));
                args.push("-p".into());
                args.push(format!("127.0.0.1::{}", 9322 + d));
            }
            args.extend(["-v".into(), ws, "-v".into(), homes, "--shm-size".into(), "1g".into(), IMAGE.into()]);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = docker(&refs).await;
            if r.code != 0 {
                return Err(ComputerError::Failed(format!("docker run failed: {}", r.stderr.trim())));
            }
        }
        State::Stopped => {
            let r = docker(&["start", &name]).await;
            if r.code != 0 {
                return Err(ComputerError::Failed(format!("docker start failed: {}", r.stderr.trim())));
            }
        }
        State::Running => {}
    }
    let agentd_port = wait_for_agentd(&name).await?;
    Ok(Computer { conversation: conversation.into(), name, agentd_port })
}

async fn wait_for_agentd(name: &str) -> Result<u16> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut last = "no attempt".to_string();
    while tokio::time::Instant::now() < deadline {
        if let Some(port) = host_port(name, AGENTD_PORT).await {
            match reqwest::Client::new()
                .get(format!("http://127.0.0.1:{port}/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => return Ok(port),
                Ok(r) => last = format!("HTTP {}", r.status()),
                Err(e) => last = e.to_string(),
            }
        } else {
            last = "port not published yet".into();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(ComputerError::Failed(format!("the computer did not start within 90s ({last})")))
}

async fn agentd_post(c: &Computer, path: &str, body: Value) -> Result<Value> {
    let r = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}{path}", c.agentd_port))
        .json(&body)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| ComputerError::Failed(e.to_string()))?;
    let ok = r.status().is_success();
    let text = r.text().await.unwrap_or_default();
    if !ok {
        return Err(ComputerError::Failed(format!("{path}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| ComputerError::Failed(e.to_string()))
}

/// This bot's own desktop, with a browser on it.
pub async fn desktop(c: &Computer, bot: &str) -> Result<Desktop> {
    let name = normalise_bot(bot);
    let info = agentd_post(c, "/desktops", json!({"name": name})).await?;
    let web_port = info["webPort"].as_u64().unwrap_or(0) as u16;
    let view_port = host_port(&c.name, web_port)
        .await
        .ok_or_else(|| ComputerError::Failed(format!("desktop {} is outside the published port band", info["display"])))?;
    let b = agentd_post(c, "/browser", json!({"name": name})).await?;
    let cdp_inner = b["cdpPort"].as_u64().unwrap_or(0) as u16;
    let cdp = host_port(&c.name, cdp_inner).await.ok_or_else(|| ComputerError::Failed("CDP port is outside the published band".into()))?;
    crate::cdp::wait_for_cdp(cdp, Duration::from_secs(45)).await.map_err(ComputerError::Failed)?;
    Ok(Desktop {
        bot: bot.into(),
        user: info["user"].as_str().unwrap_or("").into(),
        display: info["display"].as_u64().unwrap_or(0) as u32,
        view_port,
        cdp_port: Some(cdp),
    })
}

pub async fn list_desktops(c: &Computer) -> Value {
    match reqwest::get(format!("http://127.0.0.1:{}/desktops", c.agentd_port)).await {
        Ok(r) => r.json::<Value>().await.map(|v| v["desktops"].clone()).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

/// A shell command inside the conversation's container, in /workspace.
pub async fn exec(conversation: &str, command: &str, as_bot: Option<&str>, timeout: Duration) -> ExecResult {
    let name = container_name(conversation);
    let user = as_bot.map(|b| format!("bot-{}", normalise_bot(b)));
    let mut args = vec!["exec", "-w", "/workspace", name.as_str()];
    if let Some(u) = &user {
        args.extend(["sudo", "-u", u.as_str()]);
    }
    args.extend(["bash", "-lc", command]);
    run("docker", &args, timeout).await
}

/// Write bytes to a path inside the container, streamed over stdin (no
/// argument-length limit, nothing interpolated into a shell string).
pub async fn write_file(conversation: &str, path: &str, bytes: &[u8]) -> ExecResult {
    use tokio::io::AsyncWriteExt;
    let name = container_name(conversation);
    let script = "mkdir -p \"$(dirname -- \"$1\")\" && cat > \"$1\" && wc -c < \"$1\"";
    let mut child = match Command::new("docker")
        .args(["exec", "-i", "-w", "/workspace", &name, "bash", "-c", script, "write", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ExecResult { code: 127, stdout: String::new(), stderr: e.to_string() },
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(bytes).await;
    }
    match tokio::time::timeout(Duration::from_secs(120), child.wait_with_output()).await {
        Ok(Ok(out)) => ExecResult {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Ok(Err(e)) => ExecResult { code: 127, stdout: String::new(), stderr: e.to_string() },
        Err(_) => ExecResult { code: 124, stdout: String::new(), stderr: "timed out".into() },
    }
}

/// Push one file into the container (host → /workspace/inbox).
pub async fn deliver_file(c: &Computer, bot: &str, filename: &str, contents: &[u8]) -> Result<Value> {
    use base64::Engine;
    agentd_post(
        c,
        "/files",
        json!({"name": normalise_bot(bot), "path": filename, "contentBase64": base64::engine::general_purpose::STANDARD.encode(contents)}),
    )
    .await
}

pub async fn down(conversation: &str, remove: bool) {
    let name = container_name(conversation);
    docker(&["stop", &name]).await;
    if remove {
        docker(&["rm", &name]).await;
    }
}

/// Drop every signed-in browser session for a conversation; keeps /workspace.
pub async fn reset_profiles(conversation: &str) {
    down(conversation, true).await;
    docker(&["volume", "rm", &format!("{}-homes", container_name(conversation))]).await;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub docker: bool,
    pub image: bool,
    pub conversations: Vec<(String, State)>,
}

pub async fn status() -> Status {
    let docker_ok = docker_available().await;
    if !docker_ok {
        return Status { docker: false, image: false, conversations: vec![] };
    }
    let r = docker(&["ps", "-a", "--filter", "label=mybot=conversation", "--format", "{{.Label \"mybot.conversation\"}}\t{{.State}}"]).await;
    let conversations = r
        .stdout
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(c, s)| (c.to_string(), if s == "running" { State::Running } else { State::Stopped }))
        .collect();
    Status { docker: true, image: image_exists().await, conversations }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(container_name("default"), "mybot-conv-default");
        assert_eq!(container_name("A b/c-123"), "mybot-conv-abc123");
        assert_eq!(normalise_bot("Web Researcher!"), "web-researcher-");
        assert_eq!(normalise_bot("---"), "bot");
        assert!(DOCKERFILE.contains("agentd.py"));
    }
}
