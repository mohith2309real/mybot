//! Browser actions beyond the core page tools.
//!
//! Deliberately absent: running arbitrary JavaScript and typing raw key
//! events into whatever has focus. Either would walk straight around the
//! password, 2FA and card-field boundaries (and could read a filled password
//! back out of the page). Everything here works through refs, and the toolbox
//! runs the same boundary checks before any of them touch an element.

use mybot_computer::browser::{Browser, REF_ATTR};
use serde_json::{Value, json};

use crate::{Action, b_or, browser, n, n_or, s, s_or};

const REF: (&str, &str, &str, bool) = ("ref", "integer", "Element ref from page_read", true);

pub fn defs() -> Vec<Action> {
    vec![
        browser("page_back", "Go back one page in history", &[]),
        browser("page_forward", "Go forward one page in history", &[]),
        browser("page_reload", "Reload the current page", &[]),
        browser("tab_list", "List open tabs", &[]),
        browser("tab_new", "Open a new tab (optionally at a URL)", &[("url", "string", "URL to open", false)]),
        browser("tab_switch", "Switch to a tab by index from tab_list", &[("index", "integer", "Tab index", true)]),
        browser("tab_close", "Close a tab by index", &[("index", "integer", "Tab index", true)]),
        browser("page_hover", "Hover the mouse over an element", &[REF]),
        browser("page_double_click", "Double-click an element", &[REF]),
        browser("page_right_click", "Right-click an element", &[REF]),
        browser("page_press_key", "Press a navigation key: Enter, Tab, Escape, ArrowDown, PageDown, Home, End…", &[("key", "string", "Key name (combos like Shift+Tab allowed)", true)]),
        browser("page_select_option", "Choose an option in a <select> by its visible text or value", &[REF, ("option", "string", "Option text or value", true)]),
        browser("page_set_checkbox", "Tick or untick a checkbox / radio", &[REF, ("checked", "boolean", "Desired state", true)]),
        browser("page_clear_field", "Empty a text field", &[REF]),
        browser("page_focus", "Move keyboard focus to an element", &[REF]),
        browser("page_url", "The current page's URL", &[]),
        browser("page_title", "The current page's title", &[]),
        browser("page_text", "All visible text of the page (up to N characters)", &[("max_chars", "integer", "Default 20000", false)]),
        browser("page_links", "Every link on the page: text and URL", &[("same_site", "boolean", "Only links on this site", false)]),
        browser("page_images", "Images on the page: src, alt text and size", &[]),
        browser("page_headings", "The page's heading outline (h1–h6)", &[]),
        browser("page_tables", "Tables on the page as CSV", &[("index", "integer", "Only this table (0-based)", false)]),
        browser("page_forms", "Forms and their fields (names, types, labels — never values of password fields)", &[]),
        browser("page_meta", "Meta tags, canonical URL and JSON-LD structured data", &[]),
        browser("page_find_text", "Find text on the page: matching snippets and the nearest element refs", &[("text", "string", "Text to find", true)]),
        browser("page_wait_for_text", "Wait until text appears on the page", &[("text", "string", "Text to wait for", true), ("seconds", "integer", "Max wait (default 20)", false)]),
        browser("page_scroll_to_top", "Scroll to the top", &[]),
        browser("page_scroll_to_bottom", "Scroll to the bottom (loads lazy content)", &[]),
        browser("page_scroll_to_ref", "Scroll an element into view", &[REF]),
        browser("element_text", "The text of one element", &[REF]),
        browser("element_attribute", "An attribute of one element (href, src, aria-label…)", &[REF, ("name", "string", "Attribute name", true)]),
        browser("page_screenshot", "Save a screenshot of the page into /workspace/screenshots", &[("path", "string", "Where to save (default: timestamped)", false)]),
        browser("page_save_pdf", "Save the page as a PDF into /workspace", &[("path", "string", "Where to save (default: timestamped)", false)]),
        browser("page_set_viewport", "Emulate a screen size (e.g. 390x844 for a phone)", &[("width", "integer", "Width px", true), ("height", "integer", "Height px", true)]),
        browser("page_reset_viewport", "Back to the normal window size", &[]),
        browser("page_count_elements", "Count elements matching a CSS selector", &[("selector", "string", "CSS selector", true)]),
        browser("page_performance", "Load timing and resource counts for the page", &[]),
        browser("page_cookie_names", "Names and domains of this page's cookies (never their values)", &[]),
        browser("page_word_count", "Words of visible text on the page", &[]),
        browser("page_list_items", "Text of list items (li) on the page", &[("selector", "string", "Optional CSS selector for the list", false)]),
        browser("page_upload_file", "Attach a file from /workspace to a file input", &[REF, ("path", "string", "File path inside the computer", true)]),
        browser("page_go_to_ref_link", "Open the URL of a link ref in a new tab", &[REF]),
        browser("page_highlight_ref", "Outline an element so a watching human can see it", &[REF]),
    ]
}

fn ref_of(v: &Value) -> Result<u32, String> {
    n(v, "ref").map(|x| x as u32)
}

async fn on_ref(b: &Browser, r: u32, body: &str) -> Result<Value, String> {
    let func = format!("({{attr, r, arg}}) => {{ const el = document.querySelector(`[${{attr}}=\"${{r}}\"]`); if (!el) return {{__missing: true}}; {body} }}");
    let out = b.call_fn(&func, json!({"attr": REF_ATTR, "r": r, "arg": Value::Null})).await?;
    if out.get("__missing").is_some() {
        return Err(format!("No element with ref {r}. Take a fresh page_read — refs change on every snapshot."));
    }
    Ok(out)
}

async fn on_ref_arg(b: &Browser, r: u32, arg: Value, body: &str) -> Result<Value, String> {
    let func = format!("({{attr, r, arg}}) => {{ const el = document.querySelector(`[${{attr}}=\"${{r}}\"]`); if (!el) return {{__missing: true}}; {body} }}");
    let out = b.call_fn(&func, json!({"attr": REF_ATTR, "r": r, "arg": arg})).await?;
    if out.get("__missing").is_some() {
        return Err(format!("No element with ref {r}. Take a fresh page_read — refs change on every snapshot."));
    }
    Ok(out)
}

fn text(v: Value) -> String {
    match v {
        Value::String(s) => s,
        Value::Null => String::new(),
        o => serde_json::to_string_pretty(&o).unwrap_or_default(),
    }
}

/// Run a browser action. Screenshot/PDF bytes are returned for the toolbox
/// to write into the container.
pub enum Output {
    Text(String),
    File { default_name: String, bytes: Vec<u8>, path: Option<String> },
}

pub async fn run(name: &str, b: &Browser, v: &Value) -> Result<Output, String> {
    use Output::Text;
    Ok(match name {
        "page_back" => Text(b.history_step(-1).await?),
        "page_forward" => Text(b.history_step(1).await?),
        "page_reload" => Text(b.reload().await?),
        "tab_list" => Text(
            b.tabs().await?.iter().map(|t| format!("[{}]{} {} — {}", t.index, if t.active { " (active)" } else { "" }, t.title, t.url)).collect::<Vec<_>>().join("\n"),
        ),
        "tab_new" => Text(b.new_tab(s_or(v, "url", "")).await?),
        "tab_switch" => Text(b.switch_tab(n(v, "index")? as usize).await?),
        "tab_close" => Text(b.close_tab(n(v, "index")? as usize).await?),
        "page_hover" => Text(b.hover(ref_of(v)?).await?),
        "page_double_click" => Text(b.double_click(ref_of(v)?).await?),
        "page_right_click" => Text(b.right_click(ref_of(v)?).await?),
        "page_press_key" => {
            let key = s(v, "key")?;
            // Navigation keys only: typing characters goes through page_type,
            // where the field is checked first.
            let last = key.rsplit('+').next().unwrap_or(key);
            if last.chars().count() == 1 && !key.contains('+') {
                return Err("Use page_type to type text; page_press_key is for keys like Enter, Tab, Escape, arrows.".into());
            }
            b.key(key).await?;
            b.settle().await;
            Text(format!("pressed {key}"))
        }
        "page_select_option" => {
            let r = ref_of(v)?;
            let out = on_ref_arg(b, r, json!(s(v, "option")?), "if (el.tagName !== 'SELECT') return 'not a select'; const want = String(arg).trim().toLowerCase(); const o = Array.from(el.options).find((o) => o.value.toLowerCase() === want || o.text.trim().toLowerCase() === want) || Array.from(el.options).find((o) => o.text.trim().toLowerCase().includes(want)); if (!o) return 'no such option; options: ' + Array.from(el.options).map((o) => o.text.trim()).join(' | '); el.value = o.value; el.dispatchEvent(new Event('input', {bubbles: true})); el.dispatchEvent(new Event('change', {bubbles: true})); return 'selected ' + o.text.trim();").await?;
            Text(text(out))
        }
        "page_set_checkbox" => {
            let r = ref_of(v)?;
            let want = b_or(v, "checked", true);
            let cur = on_ref(b, r, "return el.checked === true;").await?;
            if cur == json!(want) {
                Text(format!("[{r}] was already {}", if want { "checked" } else { "unchecked" }))
            } else {
                b.click(r).await?;
                Text(format!("[{r}] is now {}", if want { "checked" } else { "unchecked" }))
            }
        }
        "page_clear_field" => {
            let r = ref_of(v)?;
            Text(text(on_ref(b, r, "if (!('value' in el)) return 'not a field'; el.focus(); el.value = ''; el.dispatchEvent(new Event('input', {bubbles: true})); el.dispatchEvent(new Event('change', {bubbles: true})); return 'cleared';").await?))
        }
        "page_focus" => Text(text(on_ref(b, ref_of(v)?, "el.focus(); return 'focused';").await?)),
        "page_url" => Text(b.url().await),
        "page_title" => Text(text(b.eval("document.title").await?)),
        "page_text" => {
            let max = n_or(v, "max_chars", 20000.0) as usize;
            let t = text(b.eval("document.body ? document.body.innerText : ''").await?);
            Text(if t.chars().count() > max { format!("{}\n…[truncated]", t.chars().take(max).collect::<String>()) } else { t })
        }
        "page_links" => {
            let same = b_or(v, "same_site", false);
            let out = b.call_fn("({same}) => Array.from(document.querySelectorAll('a[href]')).map((a) => [a.innerText.trim().replace(/\\s+/g, ' ').slice(0, 80), a.href]).filter(([, h]) => h.startsWith('http') && (!same || new URL(h).origin === location.origin)).slice(0, 500).map(([t, h]) => `${t || '(no text)'} — ${h}`).join('\\n')", json!({"same": same})).await?;
            Text(text(out))
        }
        "page_images" => Text(text(b.eval("Array.from(document.images).slice(0, 300).map((i) => `${i.naturalWidth}x${i.naturalHeight}  ${i.alt ? 'alt=\"' + i.alt + '\"' : '(no alt)'}  ${i.currentSrc || i.src}`).join('\\n')").await?)),
        "page_headings" => Text(text(b.eval("Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6')).map((h) => '  '.repeat(Number(h.tagName[1]) - 1) + h.tagName + ' ' + h.innerText.trim().replace(/\\s+/g, ' ')).join('\\n')").await?)),
        "page_tables" => {
            let only = v.get("index").and_then(Value::as_i64).unwrap_or(-1);
            let out = b
                .call_fn(
                    "({only}) => { const q = (c) => /[\",\\n]/.test(c) ? '\"' + c.replace(/\"/g, '\"\"') + '\"' : c; return Array.from(document.querySelectorAll('table')).map((t, i) => [i, t]).filter(([i]) => only < 0 || i === only).map(([i, t]) => `# table ${i}\\n` + Array.from(t.rows).map((r) => Array.from(r.cells).map((c) => q(c.innerText.trim().replace(/\\s+/g, ' '))).join(',')).join('\\n')).join('\\n\\n') || 'No tables on this page.'; }",
                    json!({"only": only}),
                )
                .await?;
            Text(text(out))
        }
        "page_forms" => Text(text(b.eval("Array.from(document.forms).map((f, i) => `form ${i} ${f.method || 'get'} ${f.action}\\n` + Array.from(f.elements).filter((e) => e.name || e.id).map((e) => `  ${e.tagName.toLowerCase()}${e.type ? ':' + e.type : ''} name=${e.name || e.id}${e.required ? ' (required)' : ''}${e.type !== 'password' && e.value && e.type !== 'hidden' ? ' value=\"' + String(e.value).slice(0, 40) + '\"' : ''}`).join('\\n')).join('\\n\\n') || 'No forms.'").await?)),
        "page_meta" => Text(text(b.eval("[...Array.from(document.querySelectorAll('meta[name],meta[property]')).map((m) => `${m.getAttribute('name') || m.getAttribute('property')}: ${m.content}`), 'canonical: ' + (document.querySelector('link[rel=canonical]') || {}).href, ...Array.from(document.querySelectorAll('script[type=\"application/ld+json\"]')).map((s) => 'json-ld: ' + s.textContent.trim().slice(0, 3000))].join('\\n')").await?)),
        "page_find_text" => {
            let out = b.call_fn("({q, attr}) => { const want = q.toLowerCase(); const hits = []; const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT); while (walker.nextNode() && hits.length < 30) { const t = walker.currentNode.textContent; const i = t.toLowerCase().indexOf(want); if (i < 0) continue; const el = walker.currentNode.parentElement; const near = el && (el.closest(`[${attr}]`) || el.querySelector(`[${attr}]`)); hits.push(`…${t.slice(Math.max(0, i - 60), i + want.length + 60).replace(/\\s+/g, ' ').trim()}…${near ? '  (near ref ' + near.getAttribute(attr) + ')' : ''}`); } return hits.length ? hits.join('\\n') : 'Not found on this page.'; }", json!({"q": s(v, "text")?, "attr": REF_ATTR})).await?;
            Text(text(out))
        }
        "page_wait_for_text" => {
            let want = s(v, "text")?.to_lowercase();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(n_or(v, "seconds", 20.0).clamp(1.0, 120.0) as u64);
            loop {
                let body = text(b.eval("document.body ? document.body.innerText : ''").await.unwrap_or_default()).to_lowercase();
                if body.contains(&want) {
                    break Text(format!("\"{}\" is on the page", s(v, "text")?));
                }
                if tokio::time::Instant::now() > deadline {
                    break Text(format!("\"{}\" did not appear in time", s(v, "text")?));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
        "page_scroll_to_top" => {
            b.eval("window.scrollTo(0, 0)").await?;
            Text("at the top".into())
        }
        "page_scroll_to_bottom" => {
            b.eval("window.scrollTo(0, document.body.scrollHeight)").await?;
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            Text("at the bottom".into())
        }
        "page_scroll_to_ref" => Text(text(on_ref(b, ref_of(v)?, "el.scrollIntoView({block: 'center'}); return 'scrolled into view';").await?)),
        "element_text" => Text(text(on_ref(b, ref_of(v)?, "return el instanceof HTMLInputElement && el.type === 'password' ? '(password field — contents never shown)' : (el.innerText || el.value || '').trim();").await?)),
        "element_attribute" => {
            let name = s(v, "name")?.to_lowercase();
            if name == "value" {
                return Ok(Text(text(on_ref(b, ref_of(v)?, "return el instanceof HTMLInputElement && el.type === 'password' ? '(password field — contents never shown)' : String(el.value ?? '');").await?)));
            }
            Text(text(on_ref_arg(b, ref_of(v)?, json!(name), "return el.getAttribute(arg) ?? '(not set)';").await?))
        }
        "page_screenshot" => Output::File { default_name: "screenshots/page.png".into(), bytes: b.screenshot_png().await?, path: v.get("path").and_then(Value::as_str).map(String::from) },
        "page_save_pdf" => Output::File { default_name: "pdfs/page.pdf".into(), bytes: b.pdf().await?, path: v.get("path").and_then(Value::as_str).map(String::from) },
        "page_set_viewport" => {
            let (w, h) = (n(v, "width")? as u32, n(v, "height")? as u32);
            b.set_viewport(w.clamp(200, 4000), h.clamp(200, 4000)).await?;
            Text(format!("viewport {w}x{h}"))
        }
        "page_reset_viewport" => {
            b.clear_viewport().await?;
            Text("viewport reset".into())
        }
        "page_count_elements" => Text(text(b.call_fn("({sel}) => { try { return String(document.querySelectorAll(sel).length); } catch (e) { return 'bad selector'; } }", json!({"sel": s(v, "selector")?})).await?)),
        "page_performance" => Text(text(b.eval("(() => { const n = performance.getEntriesByType('navigation')[0] || {}; const r = performance.getEntriesByType('resource'); const kb = Math.round(r.reduce((a, x) => a + (x.transferSize || 0), 0) / 1024); const slow = r.slice().sort((a, b) => b.duration - a.duration).slice(0, 10).map((x) => `  ${Math.round(x.duration)}ms ${x.name.slice(0, 100)}`).join('\\n'); return `ttfb ${Math.round(n.responseStart || 0)}ms\\ndom ready ${Math.round(n.domContentLoadedEventEnd || 0)}ms\\nload ${Math.round(n.loadEventEnd || 0)}ms\\nresources ${r.length} (${kb} KB transferred)\\nslowest:\\n${slow}`; })()").await?)),
        "page_cookie_names" => Text(text(b.eval("document.cookie.split(';').map((c) => c.split('=')[0].trim()).filter(Boolean).join('\\n') || '(no script-visible cookies)'").await?)),
        "page_word_count" => Text(text(b.eval("String((document.body ? document.body.innerText : '').split(/\\s+/).filter(Boolean).length) + ' words'").await?)),
        "page_list_items" => {
            let sel = s_or(v, "selector", "");
            Text(text(b.call_fn("({sel}) => { let root = document; if (sel) { try { root = document.querySelector(sel) || document; } catch (e) {} } return Array.from(root.querySelectorAll('li')).slice(0, 400).map((li) => '- ' + li.innerText.trim().replace(/\\s+/g, ' ').slice(0, 200)).join('\\n') || 'No list items.'; }", json!({"sel": sel})).await?))
        }
        "page_upload_file" => {
            let path = s(v, "path")?;
            let r = ref_of(v)?;
            let is_file = on_ref(b, r, "return el instanceof HTMLInputElement && el.type === 'file';").await?;
            if is_file != json!(true) {
                return Err(format!("Ref {r} is not a file input."));
            }
            let full = if path.starts_with('/') { path.to_string() } else { format!("/workspace/{path}") };
            b.set_file_input(r, &full).await?;
            Text(format!("attached {full} to [{r}]"))
        }
        "page_go_to_ref_link" => {
            let href = text(on_ref(b, ref_of(v)?, "return el.href || el.closest('a')?.href || '';").await?);
            if !href.starts_with("http") {
                return Err("That element has no link.".into());
            }
            Text(b.new_tab(&href).await?)
        }
        "page_highlight_ref" => Text(text(on_ref(b, ref_of(v)?, "el.style.outline = '3px solid #f5a623'; el.style.outlineOffset = '2px'; return 'highlighted';").await?)),
        other => return Err(format!("no browser action {other}")),
    })
}
