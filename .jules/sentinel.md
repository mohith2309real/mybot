## 2024-05-24 - [CRITICAL] JSON.stringify() Command Substitution Injection in Bash

**Vulnerability:**
Using `JSON.stringify(path)` to pass arguments to bash executed commands (`exec('bash', '-lc', cmd)`) is vulnerable to command substitution injection. If the user input contains `$(cmd)` or `` `cmd` ``, bash will evaluate the command inside the double quotes output by `JSON.stringify()`.

**Learning:**
`JSON.stringify()` wraps the string in double quotes (`"`). In bash, double quotes allow for variable expansion and command substitution. For example, `JSON.stringify("$(whoami)")` becomes `"$(whoami)"`. When this is passed to `bash -lc 'cat "$(whoami)"'`, bash executes `whoami` before running `cat`.

**Prevention:**
Always use single-quote shell escaping for arguments passed to shell commands.
Instead of: `JSON.stringify(input)`
Use: `'${input.replace(/'/g, "'\\''")}'`
This wraps the argument in single quotes, which bash treats as completely literal, preventing any evaluation of enclosed characters.
