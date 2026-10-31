## 2024-10-31 - [Command Injection via JSON.stringify in Bash]
**Vulnerability:** Shell Command Injection via `$()` in `JSON.stringify` output when passed to `bash -lc`.
**Learning:** `JSON.stringify` outputs strings enclosed in double quotes (`"`). When these strings are passed to `bash -lc` (or similar shell contexts), the shell processes command substitutions (e.g., `$(cmd)`) inside double quotes before executing the command. This allows arbitrary command execution if an attacker controls the string.
**Prevention:** Always use single-quote shell escaping (e.g., `"' + input.replace(/'/g, "'\\''") + '"`) instead of `JSON.stringify` when passing variables or user input to bash command strings to ensure they are treated as literal strings.
