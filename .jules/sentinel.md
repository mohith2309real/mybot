## 2026-11-02 - Command Injection via JSON.stringify
**Vulnerability:** Shell commands constructed using `JSON.stringify()` for string interpolation (e.g., ```cat ${JSON.stringify(path)}```) were vulnerable to command substitution injection. Because `JSON.stringify()` wraps strings in double quotes, bash evaluates embedded command substitutions like `$(cmd)` or ``cmd`` before passing them to the target program.
**Learning:** Using JSON serialization for shell arguments is a flawed security pattern because double-quoted strings in bash do not prevent shell interpolation.
**Prevention:** Strictly use single-quote escaping (e.g., replacing `'` with `'\''`) and wrap the argument in single quotes for all shell command execution to neutralize bash parsing rules.
