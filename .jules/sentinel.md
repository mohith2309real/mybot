## 2024-05-18 - [CRITICAL] Command Injection via JSON.stringify in exec()
**Vulnerability:** Shell commands executed via `computer.exec()` were using `JSON.stringify(input)` to escape variables (like file paths). However, `JSON.stringify()` uses double quotes, which allows bash command substitution (e.g., `$(cmd)` or `\`cmd\``) to be evaluated by the shell before the command runs.
**Learning:** `JSON.stringify` is for JSON, not shell escaping. Double quotes in bash allow interpolation of variables and execution of command substitutions. This codebase passes paths directly to bash in a container, making this a critical vector.
**Prevention:** Strictly use single-quote shell escaping (e.g., `'` -> `'\''`, then wrap in `'...'`) when passing any arguments to shell commands executed via bash.
