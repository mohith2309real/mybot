MyBot 2.1 is a new look, a second way to run GPT, and updates that install themselves. Website: https://my-bot.web.app

## What's new

- **A new look.** The roster gives every teammate its own colour tile, a role chip, the time and what it last said. The window uses your system's own font, MyBot's warm dark palette and its hexagon mark — now also the app icon on Mac and Windows.
- **Hand-offs inside the computer.** When a teammate stops for you, the instruction stays on screen inside its computer, with **I'm done, continue** and **Skip this step** right there. While you teach a task, the screen gets a red frame.
- **GPT: sign in with ChatGPT, or use an API key.** Settings → AI providers → GPT. ChatGPT sign-in runs on your ChatGPT plan through the open-source `openai-oauth` helper (pinned to 2.0.0; needs Node.js). It signs you in through OpenAI in your browser and keeps the token in `~/.codex/auth.json`; MyBot never sees it. From the command line: `mybot2 keys chatgpt` or `mybot2 keys api-key`.
- **Updates over the air.** MyBot checks for a new release a few seconds after it opens and every six hours, downloads it in the background, and shows **Restart** when it's ready. Every release is signed: MyBot checks the signature against its built-in key, and the download against the signed checksum, before installing anything. Turn it off in Settings → About, or update from the command line with `mybot2 update`.

**Coming from 2.0.0?** Install 2.1 by hand once (2.0.0 can't update itself). From 2.1 on, updates install themselves.

## Download

| Your computer | File |
|---|---|
| Windows (64-bit) | `mybot2-…-windows-x86_64.zip` |
| Mac with Apple silicon (M1 and later) | `mybot2-…-macos-arm64.tar.gz` |
| Mac with Intel | `mybot2-…-macos-x86_64.tar.gz` |
| Linux (64-bit) | `mybot2-…-linux-x86_64.tar.gz` |

Each file has a `.sha256` next to it. `latest.json` and `latest.json.sig` are what the app reads to update itself.

**You also need Docker.** It runs each teammate's computer: install Docker Desktop on Windows or Mac, Docker Engine on Linux. Then bring an API key for Claude, OpenAI or Gemini — or sign in with ChatGPT.

### Opening it the first time

These builds aren't signed by Apple or Microsoft yet, so your system will warn you once.

- **Windows:** unzip and run `mybot2.exe`. If SmartScreen says "Windows protected your PC", click **More info → Run anyway**.
- **Mac:** unpack and move `MyBot.app` to Applications. Right-click it, choose **Open**, then **Open** again. If macOS still refuses, run `xattr -dr com.apple.quarantine /Applications/MyBot.app`. Keep it in Applications: macOS runs apps opened straight from Downloads from a temporary copy, which can't update itself.
- **Linux:** unpack and run `./mybot2`. The window needs X11 or Wayland; on Debian/Ubuntu: `sudo apt install libxkbcommon-x11-0 libgl1 libegl1`.

The same file is also the command line: `mybot2 --help`.

## Everything else

- **A teammate per job,** each with its own model (Claude, GPT or Gemini), effort level and persona.
- **Its own computer:** a desktop and browser in Docker. Watch it live; take over its mouse and keyboard any time.
- **It asks before things that matter.** Sending, posting, buying, paying, deleting, changing access and accepting terms wait for you, unless your request already said to do it. CAPTCHAs, 2-step codes, card fields and checkout always stop for you. Destructive commands are refused.
- **Saved logins.** You approve each use (or "always for this site"); MyBot types the password itself, only on the site it was saved for. Shared with MyBot 1.x.
- **Skills:** 245 built in plus your own; Agent Skills (`SKILL.md`) import and export. **Teach a task** by doing it once. **Routines** run skills on a schedule.
- **249 actions and 300 known sites.**

Full guide: [mybot2/README.md](https://github.com/mohith2309real/mybot/blob/v2.1.0/mybot2/README.md).

## Known limits

- **Hand-tested on Linux and macOS, not yet on Windows.** The new window was run and checked on macOS (Apple silicon); ChatGPT sign-in and self-updating were tried end to end there too. Windows is built and smoke-tested by this release's automation.
- **The Docker computer and live view haven't been tried end to end against a real Docker desktop.**
- **Teach a task records one tab:** the one open when you press Record.
- **Routines run only while the app is open,** or while `mybot2 routines watch` runs.
