## 2024-10-30 - Command Injection in Container `exec` Call
**Vulnerability:** Found `JSON.stringify` used for escaping user input in a shell command (`cat ${JSON.stringify(wanted)}`).
**Learning:** `JSON.stringify` uses double quotes `"` and does not protect against shell expansion and command execution (e.g. `$(touch /tmp/pwned)` or backticks).
**Prevention:** Use a proper shell quote function that surrounds the text in single quotes `''` and escapes inner single quotes as `'\''` before putting it into bash `-c`.
