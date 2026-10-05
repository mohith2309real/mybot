# MyBot

A self-hosted AI-teammate harness in the shape of xAI's Grok Bot: bots that share
one persistent computer with a browser, filesystem and terminal; a live view you
can take over when the bot hits a CAPTCHA, 2FA prompt or payment page; skills;
teach-a-task recording; and routines.

Runs entirely on your own model API keys. No xAI, no Grok models, no Cursor.

---

## Architecture note: one computer per *account*, not per bot

The public write-ups say each Grok Bot gets "its own cloud computer." The docs say
otherwise, and the docs win:

> "Every Bot on your account uses the same computer." — *Use the computer and apps*
> "…isolated to your account, not to an individual Bot." — *Grok Bot overview*

Browser cookies and logins are shared across bots because it is literally the same
machine, not because a cookie jar is being synced around. xAI's own security page
warns users not to treat separate bots as isolation boundaries.

MyBot copies that: **one container per account**, one `/workspace`, one browser
profile. Bots are personas (provider + model + system prompt) over a shared
machine. This is both more faithful and materially less to build than one VM per
bot plus a sync layer.

The consequence is worth stating plainly: **bots are not a security boundary from
each other.** Any bot can use any session any other bot signed into.

---

## Status

| # | Build step | State |
|---|------------|-------|
| 1 | Provider abstraction + streaming chat | **Done** |
| 2 | Bot in Docker driving Chromium, task execution + logging | **Done** |
| 3 | noVNC live view + pause-on-security-boundary | Image ready, unwired |
| 4 | Shared `/workspace` + shared cookie jar | **Done** (falls out of one-container-per-account) |
| 5 | Skills (manual) | Schema in place, unwired |
| 6 | Teach-a-task → skill compilation | Not started |
| 7 | Routines (schedule / trigger) | Schema in place, unwired |
| 8 | Web dashboard | Not started |

---

## Step 1: the model layer

```
src/providers/types.ts      Neutral Message / ToolDefinition / StreamChunk types
src/providers/anthropic.ts  Claude — adaptive thinking, refusal fallbacks
src/providers/openai.ts     GPT — Responses API (not chat.completions)
src/providers/gemini.ts     Gemini — unified @google/genai SDK
src/secrets/vault.ts        AES-256-GCM key store, scrypt-derived
src/db/                     SQLite via node:sqlite (no native dependency)
src/cli.ts                  Driver for everything above
```

Every adapter yields the same `StreamChunk` union, so nothing downstream — the
agent loop, the container harness, skills, routines — ever sees a provider shape.

### Quick start

```bash
npm install
npm test                       # no API key needed; runs against a stub server

node src/cli.ts keys set anthropic     # prompts, stores encrypted at ~/.mybot/keys.enc
node src/cli.ts bots add scout anthropic
node src/cli.ts chat scout "What can you do?"
node src/cli.ts tasks --verbose
```

`node src/cli.ts models <provider>` lists models live from the provider API —
model ids churn faster than any table worth hardcoding.

### Key storage

AES-256-GCM under a scrypt-derived key, at `~/.mybot/keys.enc` mode 0600.
Passphrase comes from `MYBOT_PASSPHRASE` or an interactive prompt and is never
written to disk. Environment variables (`ANTHROPIC_API_KEY` etc.) take precedence
over the vault, so throwaway shells and CI work without a passphrase.

This is a laptop-grade store. The threat model is "someone reads the file," not
"someone has root and a debugger."

### Provider gotchas encoded in the adapters

- **Anthropic** — `budget_tokens` is a 400 on Opus 5; thinking is adaptive-only
  and on by default, so `max_tokens` caps thinking *and* text together.
  `display: "summarized"` is opt-in — without it you get empty thinking blocks,
  which would make the Agent Computer log look like a stall. `temperature` /
  `top_p` / `top_k` are all 400s. A safety refusal is **HTTP 200** with
  `stop_reason: "refusal"`, so server-side fallbacks are enabled by default —
  a policy decline mid-run would otherwise strand an autonomous bot.
- **OpenAI** — Responses API, which streams typed semantic events
  (`response.output_text.delta`), not opaque chat chunks. Tool results must
  reference `call_id`, **not** `item.id`; the two are different and only one
  round-trips. Function arguments arrive as a JSON string.
- **Gemini** — roles are `user`/`model`. A tool result is keyed by function
  **name**, not call id, so the originating call gets looked up to recover it.
  Call ids are often absent, so a synthetic one is minted. Text is read from
  `parts` rather than the `.text` accessor, which logs a stderr warning on every
  tool-calling turn.

---

## Step 2: the computer

```
docker/Dockerfile        Debian + Chromium under Xvfb + x11vnc/noVNC
docker/entrypoint.sh     Boot order; Chromium is the foreground process
docker/cdp-relay.py      0.0.0.0:9223 -> 127.0.0.1:9222
src/computer/container.ts  Lifecycle, volumes, docker exec
src/computer/browser.ts    CDP control via playwright-core, ref-based elements
src/computer/tools.ts      Tool surface + the credential guard
src/agent/loop.ts          model -> tools -> results -> model
```

```bash
node src/cli.ts computer up
node src/cli.ts run scout "Go to example.com and tell me the heading"
node src/cli.ts computer sh "ls -la /workspace"
```

**Chromium runs headful under Xvfb, not headless.** Step 3 needs a human to take
over the same session the bot is driving, and a headless browser has no screen
to hand over. x11vnc and noVNC are already in the image and running on :6080 —
step 3 is wiring, not a rebuild.

**Elements are addressed by `ref`, not CSS selector.** `page_read` stamps a
number onto each interactive element and returns them as a list. Models pick
"ref 12" off a list they were just shown far more reliably than they invent
selectors, and a stale ref throws instead of silently clicking the wrong thing.

Three things that cost real debugging time, recorded so they don't get
"simplified" back:

- **Chromium ignores `--remote-debugging-address`.** It binds DevTools to
  loopback regardless, so a published Docker port reaches nothing. Hence
  `cdp-relay.py`.
- **Chromium must be the container's foreground process.** An earlier entrypoint
  backgrounded everything and ended on `wait -n`, which returns when *any* job
  exits — the container died seconds after boot.
- **The persisted profile keeps a `SingletonLock` naming its creating
  container's hostname.** After a recreate the hostname differs, Chromium
  decides another machine owns the profile, and exits 21. The entrypoint clears
  stale locks; without that, profile persistence breaks on every recreate.

### The credential guard

`page_type` refuses password, one-time-code, 2FA, and card fields outright. The
model never types a credential. Passwords go through saved logins (below); codes
and cards pause for a human in the live view.

---

## Saved logins

A bot can sign in to your accounts with logins you saved, after you approve each
use. It works like a password manager's autofill, with you as the approval step.

```bash
mybot logins add https://github.com/login     # prompts for username + password
mybot logins list                             # sites and usernames, never passwords
mybot logins always github.com on             # stop asking for this one site
mybot logins rm github.com
mybot logins history                          # every use, allowed or refused
```

Or open **Logins** in the console. When a bot reaches a sign-in form it calls
`fill_login`, and you get a card: *"scout wants to sign in to github.com as
you@… — Deny / Always for this site / Allow once"*. In a terminal run, the same
question is asked in the terminal. The card shows up in the console from any
thread and on a paired phone. If you don't answer within 3 minutes, the request
expires and nothing is filled.

What holds this together:

- **The model never sees a password.** It asks to fill fields by ref. MyBot
  decrypts the password and types it into the page directly. The password never
  appears in a tool result, a task log, a database row, a console response, or a
  request to Anthropic, OpenAI or Google. `page_read` also never shows a password
  field's contents. The tests check for a leak at each of those points.
- **Exact origin only.** A login saved for `https://github.com` fills only on
  `https://github.com`. It won't fill on `gist.github.com`, on
  `github.com.login-check.example`, or on `http://`. The origin is read from the
  browser at the moment of filling, never taken from the model. A page that
  navigated while you were deciding gets nothing.
- **Real field types.** The password goes only into a real
  `<input type=password>`, and the username only into a text, email or tel
  input. A field merely labelled "password" is refused.
- **Silence is no.** An expired request, a Stop, or a "Deny" all fill nothing,
  and the bot is told not to ask again in that task. Two-step sign-ins (username
  page, then password page) need only one approval per task.
- **2FA stays yours.** One-time codes and card fields still pause for the live
  view.
- **Stored like your API keys.** `~/.mybot/logins.enc` uses AES-256-GCM with
  the same scrypt-derived key and the same passphrase, at mode 0600. Sites and
  usernames are encrypted too. Logins can be added in the console only from the
  machine itself, never over the phone link. Approving or denying works from
  anywhere.

One limit, stated plainly: a bot's `bash` runs in the same container as its
browser. A prompt-injected bot could, in principle, read a field's contents
through Chromium's local debugging port in the moment before the form submits.
That is why `fill_login` submits by default, and why the approval card names
the exact site and account. Approve a sign-in only when you asked that bot to
do something on that site.

The console also now refuses cross-site requests: a different `Origin` on a
state-changing call, or a `Host` that isn't this machine (DNS rebinding).
Without that check, any web page open in your browser could post to the local
console and start a bot, or answer a sign-in card.

---

## Using your ChatGPT account instead of an API key

MyBot can drive the OpenAI provider through a local
[`openai-oauth`](https://github.com/EvanZhouDev/openai-oauth) proxy, so a bot
runs on a ChatGPT plan with no API key.

```bash
npx openai-oauth@latest --detach     # sign in once, runs in the background
node src/cli.ts auth oauth           # detects it, lists your account's models
node src/cli.ts bots add scout openai gpt-5.6-terra
node src/cli.ts run scout "..."
```

No new adapter was needed: that proxy serves the same Responses API the OpenAI
adapter already speaks (`/v1/responses`, `/v1/models`, and function calling all
verified against it), so oauth mode is just a `baseURL` swap plus skipping key
resolution. Auth mode lives in `~/.mybot/config.json` — plaintext on purpose,
since a mode and a localhost URL are not secrets and requiring a passphrase to
read them would mean prompting on every run.

`mybot auth apikey` switches back. Two caveats worth knowing:

- This is **subscription auth, not an API key** — it draws on your ChatGPT
  plan's limits, and it sits in tension with this project's own non-goal of
  "every model call goes through the documented API with my own key." It is your
  account and an Apache-2.0 tool, but it is a different trust posture than a key.
- **Model IDs differ per account.** This one has no `gpt-5.6-sol`; it has
  `gpt-5.6-terra` / `-luna`. `mybot auth oauth` and `mybot models openai` both
  list what the account actually has rather than a hardcoded table.

### Tests

`npm test` needs no API keys. `test/local.test.mjs` covers vault crypto (including
that a wrong passphrase fails closed and no plaintext reaches disk) and the SQLite
layer. `test/wire.test.mjs` stands up a stub HTTP server and asserts the exact
request body each adapter sends plus how it parses a real SSE stream — the part
most likely to silently corrupt tool calling later.

---

## Security design

Two things are settled and should not be relitigated:

**No skill marketplace.** Skills are hand-written or self-recorded only; the
`skills.source` column is constrained to `manual | taught` at the schema level.
On 1 Feb 2026 Koi Security found 341 of 2,857 skills on OpenClaw's ClawHub
registry were malicious (11.9%), 335 from a single campaign shipping the AMOS
infostealer; a follow-up on 16 Feb found 824. Nothing is ever fetched from the
internet and executed.

**No borrowed subscription auth.** Every model call goes through the documented
API with your own key.

### Human-in-the-loop boundaries (step 3)

Execution pauses and opens the live view on: CAPTCHA, 2FA/OTP field, payment or
checkout page, and anything a skill tags `always-confirm`. While paused the bot
waits — no background polling, no auto-retry of the sensitive step. The `paused`
task status exists for exactly this.

Worth adopting verbatim from xAI's approvals list: sending messages or
invitations, publishing content, purchases and financial transfers, deleting or
overwriting data, changing permissions, production changes, accepting legal terms.

---

## Chosen sandbox (step 2)

Docker container running headful Chromium under Xvfb, driven over CDP, with
x11vnc + noVNC for the Agent Computer view.

Rejected: Daytona (went closed-source June 2026), E2B (Firecracker, hosted — and
per-tenant kernel isolation buys nothing when the design is deliberately one
container per account), AIO Sandbox (right shape, but bundles VSCode/Jupyter and
has unclear licensing).

Verified working on arm64 (Apple Silicon): image is ~1.5GB, Chromium 151.

---

## Requirements

Node ≥ 22.6 (TypeScript runs directly via type-stripping — no build step).
Developed on Node 26. Type-stripping means no enums, no namespaces, and no
constructor parameter properties in this codebase.
