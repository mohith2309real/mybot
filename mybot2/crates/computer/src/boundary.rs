//! Security boundaries: where a run stops and hands the browser to a human.
//!
//! A CAPTCHA, a 2FA code, a password field (when no saved login applies), a
//! card field, a checkout page, or anything a skill marked always-confirm.
//! Ported from 1.x's liveview.ts with the same patterns and the same rules:
//!   - a page merely *containing* a login form is not a boundary — typing into
//!     the password field is;
//!   - a page-level boundary the human already took over is acknowledged for
//!     that exact path, so a resumed run does not pause forever on it;
//!   - element-level hits and order-commit buttons are never acknowledged.

use std::collections::HashSet;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use crate::browser::{Browser, Snapshot};
use mybot_core::agent::Pause;

static EL_PASSWORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)input:password|\bpass(word|phrase)\b|\bpwd\b").unwrap());
static EL_OTP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\botp\b|one[\s-]?time|two[\s-]?factor|\b2fa\b|\bmfa\b|verification code|security code|auth(entication)? code|sms code|passcode|confirmation code|backup code|recovery code").unwrap()
});
static EL_CARD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)card ?number|credit card|debit card|\bcvv\b|\bcvc\b|expiry|expiration|exp date|\biban\b|routing number|sort code|cardholder|card holder|\bupi\b").unwrap()
});
static EL_COMMIT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)place (the )?order|pay now|complete (the )?(purchase|order)|confirm (and pay|payment|purchase|order)|buy now|submit payment|authorize payment|start (free )?trial|subscribe now").unwrap()
});
static URL_PAYMENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)checkout|/(payments?|billing|purchase|subscribe|order[-_]?(confirm|review|summary))(/|\?|$)|[?&](step|stage)=(payment|checkout)").unwrap()
});

/// What the bot is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Type,
    Click,
    Navigate,
    Read,
}

/// A rule from a skill's safety section: "always-confirm: posting to the company account".
#[derive(Debug, Clone)]
pub struct ConfirmRule {
    pub label: String,
    pub pattern: Regex,
    /// "element", "page" or "any".
    pub scope: String,
}

impl ConfirmRule {
    /// A plain phrase becomes a case-insensitive substring match.
    pub fn phrase(label: &str, phrase: &str) -> Self {
        Self { label: label.into(), pattern: Regex::new(&format!("(?i){}", regex::escape(phrase))).unwrap(), scope: "any".into() }
    }
}

#[derive(Default)]
pub struct Boundaries {
    acknowledged: Mutex<HashSet<String>>,
    rules: Mutex<Vec<ConfirmRule>>,
}

const SIGNALS_JS: &str = r#"
(() => {
  const OTP = /\botp\b|one[\s-]?time|two[\s-]?factor|\b2fa\b|\bmfa\b|verification code|security code|auth(entication)? code|sms code|passcode|confirmation code/i;
  const CARD = /card ?number|cardnumber|credit card|debit card|\bcvv\b|\bcvc\b|\bcc[-_]?(num|number|exp|csc)|expiry|expiration|exp[-_ ]?(date|month|year)|\biban\b|routing ?number|sort ?code|cardholder/i;
  const CAPTCHA_SRC = /recaptcha|hcaptcha|turnstile|arkoselabs|funcaptcha|geetest|captcha/i;
  const CAPTCHA_TEXT = /verify (that )?you (are|'re) (a )?human|i'm not a robot|complete the security check|unusual traffic from your computer|checking your browser before/i;
  const seen = (el) => { const r = el.getBoundingClientRect(); if (r.width < 4 || r.height < 4) return false;
    const s = getComputedStyle(el); return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05; };
  const labelOf = (el) => {
    const parts = ['type','name','id','placeholder','aria-label','autocomplete'].map((a) => el.getAttribute(a) || '');
    const id = el.getAttribute('id');
    if (id) { const lab = document.querySelector(`label[for="${CSS.escape(id)}"]`); if (lab && lab.innerText) parts.push(lab.innerText); }
    const wrap = el.closest('label'); if (wrap && wrap.innerText) parts.push(wrap.innerText);
    return parts.join(' ').replace(/\s+/g, ' ').trim().slice(0, 120);
  };
  const captcha = [];
  for (const f of Array.from(document.querySelectorAll('iframe'))) {
    const src = f.getAttribute('src') || '';
    if (!CAPTCHA_SRC.test(src)) continue;
    // Invisible reCAPTCHA and the score badge are on a third of the web; only
    // a real, human-sized challenge counts.
    if (/size=invisible/i.test(src) || f.closest('.grecaptcha-badge')) continue;
    const r = f.getBoundingClientRect();
    if (!seen(f) || r.width < 200 || r.height < 60) continue;
    captcha.push(`captcha iframe ${src.slice(0, 120)}`);
  }
  for (const el of Array.from(document.querySelectorAll('.g-recaptcha, .h-captcha, .cf-turnstile, [data-sitekey], #challenge-form'))) {
    if (!seen(el) || el.getAttribute('data-size') === 'invisible') continue;
    captcha.push(`captcha widget <${el.tagName.toLowerCase()}>`);
  }
  const bodyText = (document.body ? document.body.innerText : '').replace(/\s+/g, ' ').trim();
  if (CAPTCHA_TEXT.test(bodyText)) captcha.push('captcha wording on the page');
  const otp = [], card = [];
  for (const el of Array.from(document.querySelectorAll('input, textarea'))) {
    if (el.type === 'hidden' || !seen(el)) continue;
    const label = labelOf(el);
    const auto = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (auto.includes('one-time-code') || OTP.test(label)) otp.push(label);
    else if (auto.startsWith('cc-') || CARD.test(label)) card.push(label);
  }
  return { url: location.href, title: document.title, captcha: captcha.slice(0, 5), otp: otp.slice(0, 5), card: card.slice(0, 5), text: bodyText.slice(0, 4000) };
})()
"#;

fn path_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()),
        Err(_) => url.to_string(),
    }
}

fn ack_key(kind: &str, url: &str) -> String {
    let path = match url::Url::parse(url) {
        Ok(u) => format!("{}{}", u.origin().ascii_serialization(), u.path()),
        Err(_) => url.to_string(),
    };
    format!("{kind}|{path}")
}

fn pause(kind: &str, reason: impl Into<String>, url: &str, page_level: bool) -> Pause {
    Pause { kind: kind.into(), reason: reason.into(), url: url.into(), page_level }
}

impl Boundaries {
    pub fn set_rules(&self, rules: Vec<ConfirmRule>) {
        *self.rules.lock().unwrap() = rules;
    }

    pub fn acknowledge(&self, p: &Pause) {
        if p.page_level {
            self.acknowledged.lock().unwrap().insert(ack_key(&p.kind, &p.url));
        }
    }

    /// Start of a run: a takeover granted for one task is not consent for the next.
    pub fn reset(&self) {
        self.acknowledged.lock().unwrap().clear();
    }

    /// Classify the element the bot is about to touch (from its snapshot line).
    pub fn element(&self, line: &str, url: &str, action: Action) -> Option<Pause> {
        if line.is_empty() {
            return None;
        }
        for rule in self.rules.lock().unwrap().iter() {
            if rule.scope != "page" && rule.pattern.is_match(line) {
                return Some(pause("always_confirm", format!("A skill marked this always-confirm: {}.", rule.label), url, false));
            }
        }
        match action {
            Action::Click => EL_COMMIT
                .is_match(line)
                .then(|| pause("payment", "This button completes a purchase or commits an order.", url, false)),
            Action::Type => {
                if EL_PASSWORD.is_match(line) {
                    Some(pause("password", "This is a password field. A human signs in; the bot continues afterwards.", url, false))
                } else if EL_OTP.is_match(line) {
                    Some(pause("two_factor", "This is a one-time / 2FA code field. Only the human has the code.", url, false))
                } else if EL_CARD.is_match(line) {
                    Some(pause("payment", "This is a payment card field.", url, false))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Classify the page from signals read out of the live DOM.
    pub fn page(&self, s: &Value) -> Option<Pause> {
        let url = s["url"].as_str().unwrap_or("");
        let list = |k: &str| -> Vec<String> { s[k].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect() };
        let hit = if !list("captcha").is_empty() {
            Some(pause("captcha", "A CAPTCHA is on screen. Solve it in the live view, then hand back.", url, true))
        } else if !list("otp").is_empty() {
            Some(pause("two_factor", "This page is asking for a one-time / 2FA code.", url, true))
        } else if !list("card").is_empty() {
            Some(pause("payment", "This page is asking for payment card details.", url, true))
        } else if URL_PAYMENT.is_match(&path_of(url)) {
            // URL alone is enough: a needless wait is recoverable, an unattended purchase is not.
            Some(pause("payment", "This is a checkout or billing page.", url, true))
        } else {
            let haystack = format!("{url}\n{}\n{}", s["title"].as_str().unwrap_or(""), s["text"].as_str().unwrap_or(""));
            self.rules
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.scope != "element" && r.pattern.is_match(&haystack))
                .map(|r| pause("always_confirm", format!("A skill marked this always-confirm: {}.", r.label), url, true))
        };
        hit.filter(|p| !self.acknowledged.lock().unwrap().contains(&ack_key(&p.kind, &p.url)))
    }

    /// Element first (if the action targets one), then the page.
    pub async fn detect(&self, browser: &Browser, action: Action, r: Option<u32>, snap: Option<&Snapshot>) -> Option<Pause> {
        if let (Some(r), Some(snap), Action::Type | Action::Click) = (r, snap, action) {
            if let Some(p) = self.element(snap.line(r).unwrap_or(""), &snap.url, action) {
                return Some(p);
            }
        }
        // Mid-navigation the context is torn down; element checks already ran
        // and the next tool call re-checks, so a failed read is not fatal.
        let signals = browser.eval(SIGNALS_JS).await.ok()?;
        self.page(&signals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn element_rules() {
        let b = Boundaries::default();
        let url = "https://x.example/login";
        assert_eq!(b.element(r#"[3] input:password """#, url, Action::Type).unwrap().kind, "password");
        assert_eq!(b.element(r#"[4] input:text "Verification code""#, url, Action::Type).unwrap().kind, "two_factor");
        assert_eq!(b.element(r#"[5] input:text "Card number""#, url, Action::Type).unwrap().kind, "payment");
        assert_eq!(b.element(r#"[6] button "Place order""#, url, Action::Click).unwrap().kind, "payment");
        assert!(b.element(r#"[6] button "Place order""#, url, Action::Type).is_none());
        assert!(b.element(r#"[7] input:email "Email""#, url, Action::Type).is_none());
        assert!(b.element(r#"[8] a "Forgot password?""#, url, Action::Click).is_none(), "reading a login form is fine");
    }

    #[test]
    fn page_rules_and_acknowledgement() {
        let b = Boundaries::default();
        let checkout = json!({"url": "https://shop.example/checkout?x=1", "captcha": [], "otp": [], "card": []});
        let p = b.page(&checkout).unwrap();
        assert_eq!(p.kind, "payment");
        b.acknowledge(&p);
        assert!(b.page(&checkout).is_none(), "acknowledged for this exact path");
        assert!(b.page(&json!({"url": "https://shop.example/checkout/pay", "captcha": [], "otp": [], "card": []})).is_some());
        b.reset();
        assert!(b.page(&checkout).is_some(), "no consent carries over to a new task");
        let cap = json!({"url": "https://a.example/", "captcha": ["captcha iframe"], "otp": [], "card": []});
        assert_eq!(b.page(&cap).unwrap().kind, "captcha");
        assert!(b.page(&json!({"url": "https://a.example/docs", "captcha": [], "otp": [], "card": [], "text": "hello"})).is_none());
    }

    #[test]
    fn skill_rules() {
        let b = Boundaries::default();
        b.set_rules(vec![ConfirmRule::phrase("posts to the company account", "Post to Acme")]);
        assert_eq!(b.element(r#"[2] button "Post to Acme page""#, "https://x.example", Action::Click).unwrap().kind, "always_confirm");
        assert!(b.page(&json!({"url": "https://x.example", "title": "", "text": "Post to Acme", "captcha": [], "otp": [], "card": []})).is_some());
    }
}
