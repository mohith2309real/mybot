# Features (194)

What MyBot 2 does, one feature per line. On top of these are the catalogs: [249 actions](ACTIONS.md), [245 built-in skills](SKILLS.md) and [300 sites](SITES.md).


## The window (35)

1. Native desktop app (Rust, egui): no web view anywhere
2. Roster: each teammate's face, role chip, time and last message
3. Live state on each teammate's face: working (it reads, types, thinks, scans or hums), needs you (wide eyes, a bounce or a wiggle), done (^ ^)
4. A conversation per teammate, rebuilt from history on open
5. Replies stream in as they're written
6. Live preview of the model's thinking while it works
7. Finished reasoning folded into a collapsible “Thought”
8. Every step on one line (verb and target), coloured by outcome, expandable to the full result
9. Chat Markdown: headings, bullets, bold, inline code, code blocks
10. Selectable, copyable text in the conversation
11. Starter ideas for an empty conversation
12. Composer: Enter sends, Shift+Enter adds a line
13. Stop a running task
14. Pick a skill to run from the composer, with search
15. “Needs you” banner: I'm done, continue · Take over · Skip this step
16. Confirmation banner: Approve or Decline
17. Sign-in request cards above everything, top right
18. The window asks for attention when a sign-in request arrives
19. Short notices (toasts) for results and errors
20. Remembers the open teammate and the computer panel between launches
21. The face “Glance” as the window, Dock and app icon (macOS .app and Windows .exe); the “MyBot” wordmark in the sidebar
22. Adapts down to 900×560: compact header, wrapped banner, truncated steps
23. Lock and unlock from the sidebar
24. Unlock screen; first run creates the passphrase (typed twice, 8+ characters)
25. Optional: remember the passphrase in the system keychain
26. Continue without unlocking when keys come from the environment
27. New-teammate dialog: name, provider, live model list, effort, mode, persona
28. Edit a teammate; delete one (with confirmation)
29. Shows which providers have no key yet
30. System UI font (SF on macOS, Segoe UI on Windows) at real regular and semibold weights
31. Monochrome: true greys on black, white as the one accent; red only for recording and deleting
32. Round composer: + for skills, and a send / stop button
33. `MYBOT_SNAPSHOT`: repeatable screenshots of any screen, without touching your data
34. Eight faces to pick from when you create or edit a teammate; each has its own moves for every mood
35. Faces only animate while something is happening: idle faces wake the window just for a blink or a glance

## Each teammate's computer (20)

1. A Docker container per conversation
2. A separate desktop (X display) per teammate
3. A separate Chromium and browser profile per teammate
4. Starts by itself when a task needs it, or with Start computer
5. Live view drawn natively from VNC (RFB 3.3 / 3.7 / 3.8)
6. VNC password authentication
7. Raw, CopyRect and DesktopSize encodings
8. Take over the mouse
9. Take over the keyboard, including named keys and Ctrl shortcuts
10. Scroll wheel in takeover
11. Paste from your clipboard into the bot's desktop
12. Open the same desktop in a browser tab
13. Notices when the live view drops, and says why
14. Activity log of the computer
15. Docker and image status in Settings
16. Reset every teammate's browser profile (sign out everywhere)
17. Stop or remove the computer from the command line
18. The step it's waiting on shows inside the computer, with I'm done, continue and Skip this step
19. A red frame and “… is watching and learning” while you teach a task
20. The MyBot wallpaper on every desktop (“shhh... let's not leak our hard work”), with a dark panel

## What a teammate can do (16)

1. Read the page as text plus numbered elements
2. Open a URL
3. Click an element by number
4. Type into a field by number, optionally submitting
5. Scroll
6. Sign in with a saved login (fill_login)
7. Typing into a password field is redirected to fill_login
8. Run shell commands in its own computer
9. Find one of the 249 actions, and run it
10. Hand over to you with a reason (request_human)
11. Finish with a summary (task_done)
12. The 249 actions: text, data, math, dates, hashing, files, archives, web, programs, browser, notes and memory
13. Every local action carries an example that the tests run
14. Actions with consequences are marked, and need your OK
15. Remembers the last 10 tasks with you as context
16. Up to 40 steps per task

## Where it stops for you (10)

1. CAPTCHA on the page
2. A page asking for a 2-step code
3. A one-time-code field
4. A payment card field
5. Checkout and billing pages
6. Buttons that place an order or pay
7. Steps a skill marks always-confirm
8. A page you already took over isn't flagged again for the same path
9. A pause waits 15 minutes for you, then the task stops cleanly
10. Cancelling works at any point, including while it waits for you

## Asking before acting (9)

1. Three modes: Ask first, Auto, Bypass
2. Ask first: checks in when unsure
3. Auto: routine work without checking in
4. Bypass: no stops for consequential steps
5. Your request counts as permission (“…and send it”)
6. …but not when you said not to (“draft it, don't send it”)
7. Covers sending, publishing, buying, moving money, deleting, changing access, production changes, accepting terms
8. Destructive shell commands refused in every mode
9. Never types passwords, codes or card numbers itself, in any mode

## Saved logins (14)

1. Logins encrypted with AES-256-GCM, key from scrypt
2. Ask every time: allow once, always for this site, or deny
3. No answer means no
4. Bound to the exact origin, https only (http only for localhost)
5. Sign-in pages for 300 known sites, suggested as you type
6. Sites that use 2-step sign-in are flagged
7. A 5-minute allowance for two-step sign-ins within one task
8. Re-checks the live page at fill time: origin, password field, username field
9. The password is typed by MyBot, never seen by the model
10. Page reads blank out password values
11. History of sign-in requests, with no passwords in it
12. “Don't ask for this site” switch per login
13. Same files as MyBot 1.x, byte for byte
14. Passwords wiped from memory after use, hidden from debug output

## Keys and models (18)

1. API keys encrypted with your passphrase
2. Keys from the environment take precedence
3. Unlock from the system keychain
4. Claude through the Messages API, streamed
5. Adaptive thinking with summaries
6. Five effort levels, low to max
7. Server-side model fallback where the model supports it
8. Tool input streamed and checked as strict JSON
9. Refusals handled and shown
10. A tool call cut off by the length limit is never run
11. OpenAI through the Responses API
12. Gemini, with thought signatures sent back unchanged
13. Live model lists
14. A custom endpoint per provider
15. A local OpenAI-compatible proxy works without a key
16. GPT: sign in with ChatGPT (the open-source openai-oauth helper, pinned) or use an API key
17. In ChatGPT mode no API key is ever sent to the local helper
18. Finds Node.js for the sign-in helper even when MyBot is opened from Finder or the Start menu

## Skills (21)

1. 245 built-in skills in 18 categories
2. Filter the library by category, and search it
3. Skills with inputs ask for them before running
4. Safety rules on skills, including always-confirm
5. Write your own skill
6. Edit or delete your skills
7. Skills taught by demonstration
8. Import Agent Skills (SKILL.md) from a folder
9. …from a .zip or .skill file
10. …from a GitHub folder link
11. …by dropping it on the window
12. Every imported file is scanned (high / medium / info findings)
13. Review screen: findings, files, license, tools, instructions
14. Imported skills start switched off
15. Refuses to switch on a skill whose files changed after review
16. Imported scripts run only inside the bot's computer
17. Skill files are copied into the computer when used
18. Switched-on imported skills are offered to every teammate
19. Export any skill as a SKILL.md folder
20. Run a skill from the Skills screen with any teammate
21. Schedule a skill straight from its card

## Teach a task (11)

1. Record yourself doing a task in the bot's browser
2. Passwords, codes and card numbers blanked inside the page
3. A second check of every recorded action in MyBot
4. Credentials stripped from recorded URLs
5. Keys recorded by name only, never keystrokes
6. Stops by itself after 10 minutes
7. Live log of what's being recorded
8. The teammate's model drafts a skill from the recording
9. Review and edit the draft before saving
10. Saved under a unique name
11. The recording is kept if drafting fails, to try again

## Routines (7)

1. Run a skill on a schedule (cron)
2. Presets like “weekdays at 9”
3. Plain-language schedule and next run shown as you type
4. Choose the teammate and inputs per routine
5. Pause, resume, run now, delete
6. Last run shown
7. Fire while the app is open, or headless with routines watch

## Command line (18)

1. keys: set, list, remove
2. bots: list, add, remove
3. run: a task streamed to the terminal
4. Sign-in requests answered at the terminal
5. Pauses answered at the terminal
6. Ctrl+C stops the task
7. tasks and log
8. resume and cancel from another terminal
9. logins: add, list, remove, always, pending, allow, deny, history
10. skills: list, show, import, review, enable, disable, remove, export
11. teach from the terminal
12. routines: list, add, remove, on, off, run, watch
13. actions and sites search, with --json
14. models: the live list
15. computer: status, reset profiles, down
16. Secrets read without echo, or from a pipe for scripts
17. `mybot2 keys chatgpt` / `mybot2 keys api-key`: how GPT signs in
18. `mybot2 update`: check for (`--check`) or install the latest signed release

## Updates (10)

1. Checks a few seconds after opening, then every six hours
2. Every release's manifest is signed (Ed25519); unsigned or foreign-signed manifests are ignored
3. Each download is checked against the signed size and SHA-256
4. Only strictly newer versions install: an old manifest can't downgrade you
5. Downloads in the background; “MyBot x.y.z is ready · Restart” when done
6. Swaps the app in place and keeps the previous version
7. A failed swap rolls back to the version you had
8. Automatic updates can be turned off in Settings → About
9. Development builds never replace themselves
10. Says so when macOS is running MyBot from a temporary copy that can't update

## Data (5)

1. SQLite store of teammates, tasks, steps, skills, routines, requests
2. Every step of every task logged
3. Tasks left running by a crash are closed out at startup
4. MYBOT_HOME, MYBOT2_DB and MYBOT_PASSPHRASE
5. Shares ~/.mybot with MyBot 1.x
