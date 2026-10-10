MyBot 3.0.9 is a new look: black and white, a face for every teammate, and a wallpaper for every teammate's computer. Website: https://mybot2.web.app

## What's new

- **Monochrome.** True greys on black, with white as the one accent. Colour is kept for meaning only: red while you teach a task and on buttons that delete.
- **A face for every teammate.** Eight to pick from when you create or edit a teammate: Pip, Glance, Bean, Brick, Leaf, Ghost, Moon and Blob. The face shows what the teammate is doing, and each one moves in its own ways. While it works, it reads, types, thinks, scans or hums. When it needs you, it goes wide-eyed and bounces, wiggles or pulses. When it's done, ^ ^ with a hop, a sway or a sparkle. Idle faces blink and now and then glance aside, breathe or doze. Faces only animate while something is happening.
- **A new icon and wordmark.** The app icon is the face *Glance*; the sidebar shows the MyBot wordmark.
- **A wallpaper for its computer.** Every teammate's desktop starts on the MyBot wallpaper ("shhh... let's not leak our hard work"), with a dark panel. The desktop image is rebuilt once, automatically, the first time a computer starts (it's now `mybot-desktop:0.4`). The 4K wallpaper is on the website.

Updating from 2.1 or 2.2 happens by itself: **Restart** when MyBot says it's ready.

## Download

| Your computer | File |
|---|---|
| Windows (64-bit) | `mybot2-…-windows-x86_64.zip` |
| Mac with Apple silicon (M1 and later) | `mybot2-…-macos-arm64.tar.gz` |
| Mac with Intel | `mybot2-…-macos-x86_64.tar.gz` |
| Linux (64-bit) | `mybot2-…-linux-x86_64.tar.gz` |

Each file has a `.sha256` next to it. `latest.json` and `latest.json.sig` are what the app reads to update itself.

**You also need Docker.** It runs each teammate's computer: install Docker Desktop on Windows or Mac, Docker Engine on Linux. Then bring an API key for Claude, OpenAI or Gemini, or sign in with ChatGPT.

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
- **Saved logins.** You approve each use (or "always for this site"); MyBot types the password itself, only on the site it was saved for.
- **Skills:** 245 built in plus your own; Agent Skills (`SKILL.md`) import and export. **Teach a task** by doing it once. **Routines** run skills on a schedule.
- **GPT** with an API key or your ChatGPT sign-in. **Signed updates** that install themselves.

Full guide: [mybot2/README.md](https://github.com/mohith2309real/mybot/blob/v3.0.9/mybot2/README.md).

## Known limits

- **Hand-tested on macOS, not yet on Windows.** The new window, faces and icon were run and checked on macOS (Apple silicon); the new desktop image was built and a teammate desktop started in Docker there. Windows and Linux are built and smoke-tested by this release's automation.
- **The app's live view of a teammate's computer hasn't been tried end to end against Docker yet.**
- **Teach a task records one tab:** the one open when you press Record.
- **Routines run only while the app is open,** or while `mybot2 routines watch` runs.
