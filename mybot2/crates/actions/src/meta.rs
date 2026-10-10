//! Meta actions: the run, the bot's notes and memory, and the built-in library.
//! Executed by the toolbox, which knows the run context.

use crate::{Action, meta};

const M: &str = "Notes & Memory";
const L: &str = "Library";
const R: &str = "Run";

pub fn defs() -> Vec<Action> {
    vec![
        meta("memory_save", M, "Remember a fact for later runs (key → value, kept in /workspace/.mybot/memory.json)", &[("key", "string", "Short key", true), ("value", "string", "What to remember", true)]),
        meta("memory_recall", M, "Look up remembered facts (all, or one key)", &[("key", "string", "Optional key", false)]),
        meta("memory_forget", M, "Forget a remembered fact", &[("key", "string", "Key", true)]),
        meta("note_add", M, "Append a timestamped note to /workspace/notes.md", &[("text", "string", "The note", true)]),
        meta("notes_read", M, "Read /workspace/notes.md", &[]),
        meta("todo_add", M, "Add an item to /workspace/todo.md", &[("item", "string", "To-do item", true)]),
        meta("todo_list", M, "Show /workspace/todo.md", &[]),
        meta("todo_done", M, "Tick off a to-do item by its number", &[("number", "integer", "Item number from todo_list", true)]),
        meta("skill_search", L, "Search the skill library (built-in, yours and imported)", &[("query", "string", "What you need", true)]),
        meta("skill_show", L, "Show a skill's full instructions", &[("name", "string", "Skill name", true)]),
        meta("site_info", L, "Where a site lives and where you sign in", &[("site", "string", "Name or domain, e.g. github or amazon.co.uk", true)]),
        meta("site_search", L, "List known sites in a category or matching a word", &[("query", "string", "Category or word", true)]),
        meta("wait_seconds", R, "Wait a few seconds (max 120) for something to finish", &[("seconds", "integer", "Seconds", true)]),
        meta("task_info", R, "This run's bot, task id and original instruction", &[]),
        meta("progress_note", R, "Post a short progress update to the human's thread", &[("text", "string", "One or two sentences", true)]),
        meta("list_desktops", R, "Desktops on this computer, one per bot", &[]),
    ]
}
