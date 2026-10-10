#!/usr/bin/env python3
"""Regenerate ACTIONS.md, SKILLS.md and SITES.md from the built binary.

    cargo build -p mybot-app && python3 docs/gen_catalogs.py
"""
import json, subprocess, sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
BIN = sys.argv[1] if len(sys.argv) > 1 else str(HERE.parent / "target" / "debug" / "mybot2")


def run(*args):
    return json.loads(subprocess.check_output([BIN, *args], text=True))


def esc(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def grouped(items, key):
    g = defaultdict(list)
    for it in items:
        g[it[key]].append(it)
    return sorted(g.items())


def actions():
    acts = run("actions", "--json")
    out = [f"# Actions ({len(acts)})\n",
           "Everything a teammate can do besides its core tools. Models find these with `actions_search` and run them with `action_run`.\n",
           "- **local**: runs inside MyBot, no computer needed (every one has an example the test suite executes)",
           "- **computer**: runs in the bot's container (files, archives, web requests, programs)",
           "- **browser**: drives the bot's own browser",
           "- **meta**: notes, memory and skills",
           "\n*Needs OK* means the bot asks you first, unless your request already said to.\n"]
    for cat, items in grouped(acts, "category"):
        out.append(f"\n## {cat} ({len(items)})\n")
        out.append("| Action | Kind | Parameters | What it does | Needs OK |")
        out.append("|---|---|---|---|---|")
        for a in sorted(items, key=lambda x: x["name"]):
            props = a["params"].get("properties", {})
            req = set(a["params"].get("required", []))
            params = ", ".join(f"`{k}`" + ("" if k in req else "?") for k in props)
            out.append(f"| `{a['name']}` | {a['kind']} | {params} | {esc(a['description'])} | {esc(a['needs_ok'] or '')} |")
    (HERE / "ACTIONS.md").write_text("\n".join(out) + "\n")
    return len(acts)


def skills():
    sk = run("skills", "list", "--json")
    out = [f"# Built-in skills ({len(sk)})\n",
           "Step-by-step know-how a teammate follows. Run one from the app (✨ in the composer, or Skills → Run), or `mybot2 run <bot> --skill \"<name>\" --arg key=value`.",
           "Every one exports as an Agent Skills folder (`SKILL.md`), and skills in that format import too.\n",
           "*Asks first* lists the steps where the bot always stops for your OK.\n"]
    for cat, items in grouped(sk, "category"):
        out.append(f"\n## {cat} ({len(items)})\n")
        out.append("| Skill | What it does | Inputs | Asks first |")
        out.append("|---|---|---|---|")
        for s in items:
            confirm = [r.split(":", 1)[1].strip() for r in s.get("safety", []) if r.startswith("always-confirm")]
            out.append(f"| {esc(s['name'])} | {esc(s['description'])} | {esc(', '.join(s.get('inputs', [])))} | {esc('; '.join(confirm))} |")
    (HERE / "SKILLS.md").write_text("\n".join(out) + "\n")
    return len(sk)


def sites():
    st = run("sites", "--json")
    out = [f"# Known sites ({len(st)})\n",
           "Sites MyBot knows the sign-in page for. A saved login is bound to the exact origin of that page; "
           "*2-step* means the site usually asks for a code, which you type yourself in the live view.\n"]
    for cat, items in grouped(st, "category"):
        out.append(f"\n## {cat} ({len(items)})\n")
        out.append("| Site | Sign-in page | 2-step | Note |")
        out.append("|---|---|---|---|")
        for s in sorted(items, key=lambda x: x["name"].lower()):
            out.append(f"| {esc(s['name'])} | {esc(s['login'])} | {'yes' if s.get('two_step') else ''} | {esc(s.get('note') or '')} |")
    (HERE / "SITES.md").write_text("\n".join(out) + "\n")
    return len(st)


if __name__ == "__main__":
    print(f"{actions()} actions, {skills()} skills, {sites()} sites")
