## 2024-03-24 - Command Injection via JSON.stringify
**Vulnerability:** Shell arguments were being passed through `JSON.stringify()`, which uses double-quotes and allows command substitution (e.g. `$(cmd)` or `` `cmd` ``) leading to arbitrary command execution.
**Learning:** `JSON.stringify()` is not safe for shell arguments because it does not escape variables or backticks in bash.
**Prevention:** Use a dedicated quoting function `q(val)` that wraps the value in single quotes and safely escapes inner single quotes.
