## 2024-05-18 - [Command Substitution Injection via JSON.stringify()]
**Vulnerability:** User input passed to shell commands (like `computer.exec`) was escaped using `JSON.stringify()`, which wraps strings in double quotes. In bash, double-quoted strings are subject to command substitution via backticks (`` ` ``) or `$()`, allowing for arbitrary command execution.
**Learning:** `JSON.stringify()` is not a safe way to escape arguments for bash commands because of how double quotes are interpreted by the shell.
**Prevention:** Always use single-quote shell escaping (e.g., replacing `'` with `'\''` and wrapping the entire string in single quotes) when passing dynamic data to shell commands executed via bash.
