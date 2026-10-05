//! Driving the browser: snapshots with numbered element refs, clicks, typing,
//! tabs, and the saved-login fill.
//!
//! Elements are addressed by `ref` — a number stamped onto the DOM during a
//! snapshot — never by CSS selector. Models pick "ref 12" off a list they were
//! just shown far more reliably than they invent selectors, and a stale ref
//! fails loudly instead of clicking whatever now sits in that slot.

use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::cdp::Cdp;

pub const REF_ATTR: &str = "data-mybot-ref";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    /// `  [7] input:password "Password"` lines.
    pub elements: String,
}

impl Snapshot {
    /// The snapshot line for a ref.
    pub fn line(&self, r: u32) -> Option<&str> {
        let tag = format!("[{r}]");
        self.elements.lines().map(str::trim).find(|l| l.starts_with(&tag))
    }

    /// `[7] input:password "…"` → `input:password`. Parsed, never pattern-
    /// matched: a page controls the label and can make it say anything.
    pub fn kind(&self, r: u32) -> Option<&str> {
        self.line(r)?.split_whitespace().nth(1)
    }

    pub fn render(&self) -> String {
        format!("URL: {}\nTITLE: {}\n\nINTERACTIVE ELEMENTS:\n{}\n\nPAGE TEXT:\n{}", self.url, self.title, self.elements, self.text)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub index: usize,
    pub target_id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
}

/// Fixed-string errors from the login fill. Nothing here may ever carry a
/// value, because errors become tool results.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct FillError(pub String);

struct Attached {
    target_id: String,
    session: String,
}

pub struct Browser {
    cdp: Cdp,
    current: Mutex<Option<Attached>>,
}

const SNAPSHOT_JS: &str = r#"
(({refAttr, maxChars}) => {
  for (const el of Array.from(document.querySelectorAll(`[${refAttr}]`))) el.removeAttribute(refAttr);
  const isVisible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
  };
  const label = (el) => {
    // A password field's value is never a label: without this a field with no
    // name or placeholder fell through to .value and leaked a filled password.
    const secret = el instanceof HTMLInputElement && el.type === 'password';
    const raw = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('name') ||
      el.getAttribute('title') || el.getAttribute('alt') || (el.innerText || '').trim() ||
      (secret ? '' : el.value) || '';
    return String(raw).replace(/\s+/g, ' ').trim().slice(0, 80);
  };
  const SELECTOR = 'a[href], button, input, select, textarea, [role=button], [role=link], [role=textbox], [role=checkbox], [role=tab], [role=menuitem], [contenteditable=true]';
  const lines = [];
  let ref = 0;
  for (const el of Array.from(document.querySelectorAll(SELECTOR))) {
    if (!isVisible(el)) continue;
    if (++ref > 200) break;
    el.setAttribute(refAttr, String(ref));
    const tag = el.tagName.toLowerCase();
    const kind = tag === 'input' ? `input:${el.type || 'text'}` : tag;
    const state = el.disabled ? ' (disabled)' : el.checked ? ' (checked)' : '';
    lines.push(`  [${ref}] ${kind} "${label(el)}"${state}`);
  }
  const text = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').trim();
  return {
    url: location.href,
    title: document.title,
    text: text.length > maxChars ? `${text.slice(0, maxChars)}\n…[truncated]` : text,
    elements: lines.join('\n') || '  (no interactive elements found)',
  };
})
"#;

fn key_info(key: &str) -> (String, String, i64, Option<&'static str>) {
    // (key, code, windowsVirtualKeyCode, text)
    let (k, code, vk, text): (&str, &str, i64, Option<&'static str>) = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, Some("\r")),
        "tab" => ("Tab", "Tab", 9, None),
        "escape" | "esc" => ("Escape", "Escape", 27, None),
        "backspace" => ("Backspace", "Backspace", 8, None),
        "delete" | "del" => ("Delete", "Delete", 46, None),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, None),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, None),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, None),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, None),
        "home" => ("Home", "Home", 36, None),
        "end" => ("End", "End", 35, None),
        "pageup" => ("PageUp", "PageUp", 33, None),
        "pagedown" => ("PageDown", "PageDown", 34, None),
        "space" | " " => (" ", "Space", 32, Some(" ")),
        other if other.chars().count() == 1 => {
            let c = other.chars().next().unwrap();
            let up = c.to_ascii_uppercase();
            return (c.to_string(), format!("Key{up}"), up as i64, None);
        }
        _ => (key, key, 0, None),
    };
    (k.to_string(), code.to_string(), vk, text)
}

impl Browser {
    pub async fn connect(port: u16) -> Result<Self, String> {
        let cdp = Cdp::connect(port).await?;
        let b = Self { cdp, current: Mutex::new(None) };
        b.attach_best().await?;
        Ok(b)
    }

    pub fn is_alive(&self) -> bool {
        self.cdp.is_alive()
    }

    async fn pages(&self) -> Result<Vec<Value>, String> {
        let r = self.cdp.call("Target.getTargets", json!({}), None).await?;
        Ok(r["targetInfos"].as_array().cloned().unwrap_or_default().into_iter().filter(|t| t["type"] == "page").collect())
    }

    /// Prefer the last non-blank page; open one if there are none.
    async fn attach_best(&self) -> Result<(), String> {
        let pages = self.pages().await?;
        let pick = pages
            .iter()
            .rev()
            .find(|t| t["url"].as_str().is_some_and(|u| u != "about:blank" && !u.starts_with("chrome://")))
            .or_else(|| pages.last())
            .and_then(|t| t["targetId"].as_str().map(String::from));
        let target = match pick {
            Some(t) => t,
            None => self.cdp.call("Target.createTarget", json!({"url": "about:blank"}), None).await?["targetId"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        };
        self.attach(&target).await
    }

    async fn attach(&self, target_id: &str) -> Result<(), String> {
        let r = self.cdp.call("Target.attachToTarget", json!({"targetId": target_id, "flatten": true}), None).await?;
        let session = r["sessionId"].as_str().ok_or("could not attach to the page")?.to_string();
        let _ = self.cdp.call("Page.enable", json!({}), Some(&session)).await;
        let _ = self.cdp.call("Runtime.enable", json!({}), Some(&session)).await;
        let _ = self.cdp.call("Target.activateTarget", json!({"targetId": target_id}), None).await;
        *self.current.lock().await = Some(Attached { target_id: target_id.into(), session });
        Ok(())
    }

    async fn session(&self) -> Result<String, String> {
        if let Some(a) = self.current.lock().await.as_ref() {
            return Ok(a.session.clone());
        }
        self.attach_best().await?;
        Ok(self.current.lock().await.as_ref().map(|a| a.session.clone()).unwrap_or_default())
    }

    async fn page_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let s = self.session().await?;
        match self.cdp.call(method, params.clone(), Some(&s)).await {
            Ok(v) => Ok(v),
            // The page we were on was closed (by the human, or by the site):
            // reattach to whatever is there now and try once more.
            Err(e) if e.contains("session") || e.contains("Session") || e.contains("target") => {
                *self.current.lock().await = None;
                let s = self.session().await?;
                self.cdp.call(method, params, Some(&s)).await
            }
            Err(e) => Err(e),
        }
    }

    /// Evaluate an expression in the page and return its JSON value.
    pub async fn eval(&self, expression: &str) -> Result<Value, String> {
        let r = self
            .page_call("Runtime.evaluate", json!({"expression": expression, "returnByValue": true, "awaitPromise": true}))
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            let msg = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("script error");
            return Err(msg.lines().next().unwrap_or(msg).to_string());
        }
        Ok(r["result"]["value"].clone())
    }

    /// Call a JS function `(arg) => …` with a JSON argument.
    pub async fn call_fn(&self, func: &str, arg: Value) -> Result<Value, String> {
        self.eval(&format!("({func})({arg})")).await
    }

    pub async fn url(&self) -> String {
        self.eval("location.href").await.ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
    }

    pub async fn origin(&self) -> String {
        self.eval("location.origin").await.ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
    }

    /// Give a page a moment to render without hanging on ones that never idle.
    pub async fn settle(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            if let Ok(v) = self.eval("document.readyState").await {
                if v == "complete" {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    pub async fn navigate(&self, url: &str) -> Result<(String, String), String> {
        // Bare hostnames get https://; anything with a scheme is left alone.
        let has_scheme = url.split_once(':').is_some_and(|(s, _)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()));
        let target = if has_scheme { url.to_string() } else { format!("https://{url}") };
        if target.to_ascii_lowercase().starts_with("javascript:") {
            return Err("javascript: URLs are not navigable.".into());
        }
        let r = self.page_call("Page.navigate", json!({"url": target})).await?;
        if let Some(err) = r["errorText"].as_str() {
            return Err(format!("Could not open {target}: {err}"));
        }
        self.settle().await;
        let info = self.eval("({url: location.href, title: document.title})").await?;
        Ok((info["url"].as_str().unwrap_or("").into(), info["title"].as_str().unwrap_or("").into()))
    }

    pub async fn snapshot(&self, max_text: usize) -> Result<Snapshot, String> {
        self.settle().await;
        let v = self.call_fn(SNAPSHOT_JS, json!({"refAttr": REF_ATTR, "maxChars": max_text})).await?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    /// Scroll the element into view and return its center, or an error if the ref is stale.
    async fn center(&self, r: u32) -> Result<(f64, f64, String), String> {
        let v = self
            .call_fn(
                "({attr, r}) => { const el = document.querySelector(`[${attr}=\"${r}\"]`); if (!el) return null; \
                 el.scrollIntoView({block: 'center', inline: 'center'}); const b = el.getBoundingClientRect(); \
                 return {x: b.left + b.width / 2, y: b.top + b.height / 2, text: (el.innerText || el.value && el.type !== 'password' && el.value || el.getAttribute('aria-label') || '').toString().trim().slice(0, 60)}; }",
                json!({"attr": REF_ATTR, "r": r}),
            )
            .await?;
        if v.is_null() {
            return Err(format!("No element with ref {r}. Take a fresh page_read — refs change on every snapshot."));
        }
        Ok((v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0), v["text"].as_str().unwrap_or("").to_string()))
    }

    pub async fn click_at(&self, x: f64, y: f64, button: &str, clicks: u32) -> Result<(), String> {
        self.page_call("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y})).await?;
        for n in 1..=clicks.max(1) {
            self.page_call("Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": x, "y": y, "button": button, "clickCount": n})).await?;
            self.page_call("Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": x, "y": y, "button": button, "clickCount": n})).await?;
        }
        Ok(())
    }

    pub async fn click(&self, r: u32) -> Result<String, String> {
        let (x, y, text) = self.center(r).await?;
        self.click_at(x, y, "left", 1).await?;
        self.settle().await;
        Ok(format!("clicked [{r}] {text}").trim().to_string())
    }

    pub async fn double_click(&self, r: u32) -> Result<String, String> {
        let (x, y, _) = self.center(r).await?;
        self.click_at(x, y, "left", 2).await?;
        self.settle().await;
        Ok(format!("double-clicked [{r}]"))
    }

    pub async fn right_click(&self, r: u32) -> Result<String, String> {
        let (x, y, _) = self.center(r).await?;
        self.click_at(x, y, "right", 1).await?;
        Ok(format!("right-clicked [{r}]"))
    }

    pub async fn hover(&self, r: u32) -> Result<String, String> {
        let (x, y, _) = self.center(r).await?;
        self.page_call("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y})).await?;
        Ok(format!("hovering over [{r}]"))
    }

    /// Focus a field and select its contents so typing replaces them.
    async fn focus_select(&self, r: u32) -> Result<(), String> {
        let ok = self
            .call_fn(
                "({attr, r}) => { const el = document.querySelector(`[${attr}=\"${r}\"]`); if (!el) return false; \
                 el.scrollIntoView({block: 'center'}); el.focus(); \
                 if (typeof el.select === 'function') el.select(); \
                 else if (el.isContentEditable) { const s = getSelection(); const rg = document.createRange(); rg.selectNodeContents(el); s.removeAllRanges(); s.addRange(rg); } \
                 return true; }",
                json!({"attr": REF_ATTR, "r": r}),
            )
            .await?;
        if ok != json!(true) {
            return Err(format!("No element with ref {r}. Take a fresh page_read — refs change on every snapshot."));
        }
        Ok(())
    }

    async fn insert_text(&self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return self.key("Delete").await;
        }
        self.page_call("Input.insertText", json!({"text": text})).await.map(|_| ())
    }

    pub async fn type_text(&self, r: u32, text: &str, submit: bool) -> Result<String, String> {
        self.focus_select(r).await?;
        self.insert_text(text).await?;
        if submit {
            self.key("Enter").await?;
            self.settle().await;
        }
        Ok(format!("typed into [{r}]{}", if submit { " and pressed Enter" } else { "" }))
    }

    /// Press a key, optionally with modifiers: "Enter", "Control+a", "Shift+Tab".
    pub async fn key(&self, combo: &str) -> Result<(), String> {
        let mut modifiers = 0;
        let parts: Vec<&str> = combo.split('+').collect();
        let (mods, key) = parts.split_at(parts.len() - 1);
        for m in mods {
            modifiers |= match m.to_ascii_lowercase().as_str() {
                "alt" | "option" => 1,
                "control" | "ctrl" => 2,
                "meta" | "cmd" | "command" => 4,
                "shift" => 8,
                _ => 0,
            };
        }
        let (k, code, vk, text) = key_info(key[0]);
        let mut down = json!({"type": if text.is_some() && modifiers == 0 { "keyDown" } else { "rawKeyDown" }, "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers});
        if let (Some(t), 0) = (text, modifiers) {
            down["text"] = json!(t);
        }
        self.page_call("Input.dispatchKeyEvent", down).await?;
        self.page_call("Input.dispatchKeyEvent", json!({"type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers})).await?;
        Ok(())
    }

    pub async fn scroll(&self, down: bool, amount: f64) -> Result<String, String> {
        let size = self.eval("({w: innerWidth, h: innerHeight})").await?;
        let (x, y) = (size["w"].as_f64().unwrap_or(800.0) / 2.0, size["h"].as_f64().unwrap_or(600.0) / 2.0);
        self.page_call("Input.dispatchMouseEvent", json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": if down { amount } else { -amount }})).await?;
        tokio::time::sleep(Duration::from_millis(250)).await;
        Ok(format!("scrolled {}", if down { "down" } else { "up" }))
    }

    pub async fn screenshot_png(&self) -> Result<Vec<u8>, String> {
        let r = self.page_call("Page.captureScreenshot", json!({"format": "png"})).await?;
        base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or("")).map_err(|e| e.to_string())
    }

    pub async fn pdf(&self) -> Result<Vec<u8>, String> {
        let r = self.page_call("Page.printToPDF", json!({"printBackground": true})).await?;
        base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or("")).map_err(|e| e.to_string())
    }

    pub async fn history_step(&self, delta: i64) -> Result<String, String> {
        let h = self.page_call("Page.getNavigationHistory", json!({})).await?;
        let idx = h["currentIndex"].as_i64().unwrap_or(0) + delta;
        let entries = h["entries"].as_array().cloned().unwrap_or_default();
        let Some(e) = entries.get(idx.max(0) as usize).filter(|_| idx >= 0) else {
            return Err(if delta < 0 { "There is no earlier page.".into() } else { "There is no later page.".into() });
        };
        self.page_call("Page.navigateToHistoryEntry", json!({"entryId": e["id"]})).await?;
        self.settle().await;
        Ok(format!("now on {}", self.url().await))
    }

    pub async fn reload(&self) -> Result<String, String> {
        self.page_call("Page.reload", json!({})).await?;
        self.settle().await;
        Ok(format!("reloaded {}", self.url().await))
    }

    pub async fn tabs(&self) -> Result<Vec<Tab>, String> {
        let current = self.current.lock().await.as_ref().map(|a| a.target_id.clone());
        Ok(self
            .pages()
            .await?
            .into_iter()
            .enumerate()
            .map(|(i, t)| Tab {
                index: i,
                target_id: t["targetId"].as_str().unwrap_or("").into(),
                url: t["url"].as_str().unwrap_or("").into(),
                title: t["title"].as_str().unwrap_or("").into(),
                active: current.as_deref() == t["targetId"].as_str(),
            })
            .collect())
    }

    pub async fn new_tab(&self, url: &str) -> Result<String, String> {
        let r = self.cdp.call("Target.createTarget", json!({"url": "about:blank"}), None).await?;
        let id = r["targetId"].as_str().ok_or("could not open a tab")?.to_string();
        self.attach(&id).await?;
        if !url.is_empty() {
            self.navigate(url).await?;
        }
        Ok(format!("opened a new tab{}", if url.is_empty() { String::new() } else { format!(" on {}", self.url().await) }))
    }

    pub async fn switch_tab(&self, index: usize) -> Result<String, String> {
        let tabs = self.tabs().await?;
        let t = tabs.get(index).ok_or_else(|| format!("No tab {index}. There are {} tabs.", tabs.len()))?;
        self.attach(&t.target_id).await?;
        Ok(format!("switched to tab {index}: {}", t.url))
    }

    pub async fn close_tab(&self, index: usize) -> Result<String, String> {
        let tabs = self.tabs().await?;
        let t = tabs.get(index).ok_or_else(|| format!("No tab {index}."))?;
        if tabs.len() == 1 {
            return Err("That is the only tab; it stays open.".into());
        }
        self.cdp.call("Target.closeTarget", json!({"targetId": t.target_id}), None).await?;
        // The target list lags the close; wait so the next listing is true.
        for _ in 0..40 {
            if !self.pages().await?.iter().any(|p| p["targetId"] == t.target_id.as_str()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if t.active {
            *self.current.lock().await = None;
            self.attach_best().await?;
        }
        Ok(format!("closed tab {index}"))
    }

    pub async fn clear_viewport(&self) -> Result<(), String> {
        self.page_call("Emulation.clearDeviceMetricsOverride", json!({})).await.map(|_| ())
    }

    /// Attach a file (a path inside the computer) to an <input type=file>.
    pub async fn set_file_input(&self, r: u32, path: &str) -> Result<(), String> {
        let obj = self
            .page_call(
                "Runtime.evaluate",
                json!({"expression": format!("document.querySelector('[{REF_ATTR}=\"{r}\"]')"), "returnByValue": false}),
            )
            .await?;
        let id = obj["result"]["objectId"].as_str().ok_or_else(|| format!("No element with ref {r}."))?.to_string();
        self.page_call("DOM.setFileInputFiles", json!({"files": [path], "objectId": id})).await.map(|_| ())
    }

    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<(), String> {
        self.page_call("Emulation.setDeviceMetricsOverride", json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false})).await.map(|_| ())
    }

    /// The saved-login fill. Called only by fill_login, after the human said yes.
    ///
    /// Everything is re-checked against the live DOM at the moment of filling,
    /// not trusted from the snapshot the request was made from — the human may
    /// have taken minutes, and the page may have moved meanwhile:
    ///   - the top-level page must still be on `origin`
    ///   - the password ref must be a real <input type=password>
    ///   - the username ref must be a text / email / tel / search input
    pub async fn fill_credential(
        &self,
        origin: &str,
        password_ref: Option<u32>,
        username_ref: Option<u32>,
        username: &str,
        password: &str,
        submit: bool,
    ) -> Result<bool, FillError> {
        if password_ref.is_none() && username_ref.is_none() {
            return Err(FillError("Give the ref of the password field, the username field, or both.".into()));
        }
        if self.origin().await != origin {
            return Err(FillError(format!("The page is no longer on {origin}. Nothing was filled.")));
        }
        let kind = |r: u32| async move {
            self.call_fn(
                "({attr, r}) => { const el = document.querySelector(`[${attr}=\"${r}\"]`); if (!el) return null; \
                 return el instanceof HTMLInputElement ? (el.type || 'text') : el.tagName.toLowerCase(); }",
                json!({"attr": REF_ATTR, "r": r}),
            )
            .await
            .ok()
            .and_then(|v| v.as_str().map(String::from))
        };
        if let Some(r) = password_ref {
            match kind(r).await.as_deref() {
                None => return Err(FillError(format!("No element with ref {r}. Take a fresh page_read. Nothing was filled."))),
                Some("password") => {}
                Some(_) => return Err(FillError(format!("Ref {r} is not a password field. Nothing was filled."))),
            }
        }
        if let Some(r) = username_ref {
            match kind(r).await.as_deref() {
                None => return Err(FillError(format!("No element with ref {r}. Take a fresh page_read. Nothing was filled."))),
                Some("text" | "email" | "tel" | "search") => {}
                Some(_) => return Err(FillError(format!("Ref {r} is not a username or email field. Nothing was filled."))),
            }
        }
        // Original errors are dropped on purpose: they could quote the call,
        // and the call carried the password.
        let fail = |_| FillError("The browser would not accept the login in those fields. Nothing more was attempted.".into());
        if let Some(r) = username_ref {
            self.focus_select(r).await.map_err(fail)?;
            self.insert_text(username).await.map_err(fail)?;
        }
        if let Some(r) = password_ref {
            self.focus_select(r).await.map_err(fail)?;
            self.insert_text(password).await.map_err(fail)?;
        }
        if submit {
            self.key("Enter").await.map_err(fail)?;
            self.settle().await;
        }
        Ok(submit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_kind_is_parsed_not_matched() {
        let s = Snapshot { elements: "  [1] input:email \"Email\"\n  [2] input:text \"input:password\"\n  [3] input:password \"\"".into(), ..Default::default() };
        assert_eq!(s.kind(1), Some("input:email"));
        assert_eq!(s.kind(2), Some("input:text"));
        assert_eq!(s.kind(3), Some("input:password"));
        assert_eq!(s.kind(9), None);
    }

    #[test]
    fn keys() {
        assert_eq!(key_info("Enter"), ("Enter".into(), "Enter".into(), 13, Some("\r")));
        assert_eq!(key_info("a").2, 'A' as i64);
    }
}
