## 2024-12-10 - Command Substitution Injection in Bash Execution
**Vulnerability:** User input passed to bash shell commands via `JSON.stringify()` allowed for command substitution injection (e.g., `$(cmd)`). This was found in `src/dashboard/server.ts` and `src/computer/merge.ts` where paths were being passed to `cat` and `ls -la`.
**Learning:** `JSON.stringify()` is not a safe way to escape arguments for bash shell commands, as bash will evaluate `$(...)` and backticks even inside double quotes.
**Prevention:** Always use single-quote escaping for shell commands (e.g., replacing `'` with `'\''` and wrapping in single quotes) when passing dynamic data to `exec` or `spawn` running `bash -lc`.
