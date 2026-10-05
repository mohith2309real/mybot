MyBot 2.0 is a native desktop app, rebuilt in Rust. There is no web view anywhere: the window and the live view of each bot's computer are drawn by the app itself.

You talk to an AI teammate in the middle and watch its computer on the right. When it hits a CAPTCHA, a 2-step code, a card form, or anything it shouldn't do alone, it stops and asks you. It signs in to your accounts with saved logins only after you allow it, and the AI never sees the password.

## Download

| Your computer | File |
|---|---|
| Windows (64-bit) | `mybot2-…-windows-x86_64.zip` |
| Mac with Apple silicon (M1 and later) | `mybot2-…-macos-arm64.tar.gz` |
| Mac with Intel | `mybot2-…-macos-x86_64.tar.gz` |
| Linux (64-bit) | `mybot2-…-linux-x86_64.tar.gz` |

Each file has a `.sha256` next to it, to check the download.

**You also need Docker.** It runs each teammate's computer: install Docker Desktop on Windows or Mac, Docker Engine on Linux. Then bring an API key for Claude, OpenAI or Gemini.

### Opening it the first time

These builds aren't signed yet, so your system will warn you once.

- **Windows:** unzip and run `mybot2.exe`. If SmartScreen says "Windows protected your PC", click **More info → Run anyway**.
- **Mac:** unpack and move `MyBot.app` to Applications. Right-click it, choose **Open**, then **Open** again. If macOS still refuses, run `xattr -dr com.apple.quarantine /Applications/MyBot.app`.
- **Linux:** unpack and run `./mybot2`. The window needs X11 or Wayland; on Debian/Ubuntu: `sudo apt install libxkbcommon-x11-0 libgl1 libegl1`.

The same file is also the command line: `mybot2 --help`.

## What's in it

- **A teammate per job.** Each one has its own model (Claude, GPT or Gemini), effort level and persona.
- **Its own computer.** Each teammate gets a desktop and browser in Docker. You watch it live, and you can take over its mouse and keyboard at any time.
- **It asks before things that matter.** Sending, posting, buying, paying, deleting, changing access and accepting terms all wait for you, unless your request already said to do it. CAPTCHAs, 2-step codes, card fields and checkout pages always stop for you. Destructive commands are refused.
- **Saved logins.**
  - You approve each use, or choose "always for this site".
  - MyBot types the password itself, and only on the exact site it was saved for.
  - Logins and keys are shared with MyBot 1.x.
- **Skills.**
  - 245 built-in skills, plus your own.
  - Skills in the Agent Skills format (`SKILL.md`, the format Claude uses) import from a folder, a zip or a GitHub link.
  - Imported skills are checked file by file and start switched off.
  - Any skill can be exported the same way.
- **Teach a task.** Do something once in the bot's browser and it becomes a skill. Passwords and codes are never recorded.
- **Routines.** Run skills on a schedule.
- **249 actions and 300 known sites.**

Full guide: [mybot2/README.md](https://github.com/mohith2309real/mybot/blob/v2.0.0/mybot2/README.md). Complete lists: [features](https://github.com/mohith2309real/mybot/blob/v2.0.0/mybot2/docs/FEATURES.md), [actions](https://github.com/mohith2309real/mybot/blob/v2.0.0/mybot2/docs/ACTIONS.md), [skills](https://github.com/mohith2309real/mybot/blob/v2.0.0/mybot2/docs/SKILLS.md), [sites](https://github.com/mohith2309real/mybot/blob/v2.0.0/mybot2/docs/SITES.md).

## Known limits

- **Windows and Mac are new.** They are built and smoke-tested by this release's automation. The app has been tried by hand only on Linux, so please report anything odd on Windows or Mac.
- **The Docker computer and live view haven't been tried end to end against a real Docker desktop.** The rest of the app was tried end to end on Linux.
- **Teach a task records one tab:** the one open when you press Record.
- **Routines run only while the app is open,** or while `mybot2 routines watch` runs.
