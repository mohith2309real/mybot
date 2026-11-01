## 2023-11-20 - [Command Substitution Risk]
**Vulnerability:** Use of JSON.stringify() for shell arguments in computer.exec(cmd)
**Learning:** In bash context (like bash -lc), JSON-stringified values still allow command substitution via $(cmd) because bash interprets " as double quotes where substitution is active.
**Prevention:** Always use single quotes with proper escaping (replace ' with '\'') for shell arguments.
