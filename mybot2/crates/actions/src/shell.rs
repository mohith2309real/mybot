//! Actions that run inside the bot's container, in /workspace.
//!
//! Each one only builds a bash command; the toolbox runs it with the same
//! destructive-command check as `bash`. Every path and string from the model
//! is single-quoted, so nothing it passes can break out into the shell.

use mybot_core::policy::Capability;
use serde_json::Value;

use crate::{Action, b_or, n_or, s, s_or, sh_quote as q, shell, with_capability};

const F: &str = "Files";
const W: &str = "Web & Network";
const R: &str = "Run & System";
const A: &str = "Archives";
const PATH: (&str, &str, &str, bool) = ("path", "string", "Path (relative paths are under /workspace)", true);

fn py(code: &str) -> String {
    // A code line that equals the delimiter would end the heredoc early and
    // run the rest as bash; pick one the code does not contain.
    let mut delim = "PYEOF".to_string();
    while code.lines().any(|l| l.trim() == delim) {
        delim.push('_');
    }
    format!("python3 - <<'{delim}'\n{code}\n{delim}")
}

fn url_arg(v: &Value) -> Result<String, String> {
    let u = s(v, "url")?;
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("only http(s) URLs".into());
    }
    Ok(q(u))
}

pub fn defs() -> Vec<Action> {
    vec![
        shell("file_read", F, "Read a text file (first 400 lines by default)", &[PATH, ("lines", "integer", "Max lines (default 400)", false)],
            |v| Ok(format!("head -n {} -- {} ", n_or(v, "lines", 400.0) as u64, q(s(v, "path")?)))),
        shell("file_write", F, "Write text to a file, creating folders (overwrites)", &[PATH, ("content", "string", "File contents", true)],
            |v| { let p = q(s(v, "path")?); Ok(format!("mkdir -p \"$(dirname -- {p})\" && printf '%s' {} > {p} && wc -c -- {p}", q(s(v, "content")?))) }),
        shell("file_append", F, "Append text to a file", &[PATH, ("content", "string", "Text to add", true)],
            |v| { let p = q(s(v, "path")?); Ok(format!("mkdir -p \"$(dirname -- {p})\" && printf '%s\\n' {} >> {p} && wc -l -- {p}", q(s(v, "content")?))) }),
        shell("file_list", F, "List a folder with sizes and dates", &[("path", "string", "Folder (default /workspace)", false)],
            |v| Ok(format!("ls -la --time-style=long-iso -- {}", q(s_or(v, "path", "/workspace"))))),
        shell("file_tree", F, "Show a folder tree (depth 3)", &[("path", "string", "Folder (default /workspace)", false), ("depth", "integer", "Depth (default 3)", false)],
            |v| Ok(format!("find {} -maxdepth {} -not -path '*/.*' | sort | head -500", q(s_or(v, "path", "/workspace")), n_or(v, "depth", 3.0) as u32))),
        shell("file_find", F, "Find files by name pattern", &[("pattern", "string", "Name glob, e.g. *.pdf", true), ("path", "string", "Where (default /workspace)", false)],
            |v| Ok(format!("find {} -type f -iname {} 2>/dev/null | head -500", q(s_or(v, "path", "/workspace")), q(s(v, "pattern")?)))),
        shell("file_search_text", F, "Search inside files for text (grep)", &[("text", "string", "Text or regex to find", true), ("path", "string", "Where (default /workspace)", false)],
            |v| Ok(format!("grep -rIn --color=never -e {} -- {} 2>/dev/null | head -300", q(s(v, "text")?), q(s_or(v, "path", "/workspace"))))),
        shell("file_info", F, "Size, type, dates and permissions of a file", &[PATH],
            |v| { let p = q(s(v, "path")?); Ok(format!("stat -- {p} && (command -v file >/dev/null && file -- {p} || true)")) }),
        shell("file_head", F, "First N lines of a file", &[PATH, ("lines", "integer", "Lines (default 20)", false)], |v| Ok(format!("head -n {} -- {}", n_or(v, "lines", 20.0) as u64, q(s(v, "path")?)))),
        shell("file_tail", F, "Last N lines of a file", &[PATH, ("lines", "integer", "Lines (default 20)", false)], |v| Ok(format!("tail -n {} -- {}", n_or(v, "lines", 20.0) as u64, q(s(v, "path")?)))),
        shell("file_line_count", F, "Count lines, words and bytes", &[PATH], |v| Ok(format!("wc -lwc -- {}", q(s(v, "path")?)))),
        shell("file_copy", F, "Copy a file or folder (never overwrites)", &[("from", "string", "Source", true), ("to", "string", "Destination", true)],
            |v| Ok(format!("cp -rn -- {} {} && echo copied", q(s(v, "from")?), q(s(v, "to")?)))),
        shell("file_move", F, "Move or rename a file (never overwrites)", &[("from", "string", "Source", true), ("to", "string", "Destination", true)],
            |v| Ok(format!("mv -n -- {} {} && echo moved", q(s(v, "from")?), q(s(v, "to")?)))),
        with_capability(shell("file_delete", F, "Delete a file or folder (asks the human unless the task said to delete)", &[PATH],
            |v| { let p = s(v, "path")?; if mybot_core::policy::check_destructive(&format!("rm -rf {p}")).is_some() { return Err("refused: that path is a system or home root".into()); } Ok(format!("rm -rf -- {} && echo deleted", q(p))) }), Capability::DeleteData),
        shell("make_folder", F, "Create a folder (and parents)", &[PATH], |v| Ok(format!("mkdir -p -- {} && echo created", q(s(v, "path")?)))),
        shell("file_diff", F, "Unified diff of two files", &[("a", "string", "First file", true), ("b", "string", "Second file", true)],
            |v| Ok(format!("diff -u -- {} {}; true", q(s(v, "a")?), q(s(v, "b")?)))),
        shell("file_hash", F, "SHA-256 checksum of a file", &[PATH], |v| Ok(format!("sha256sum -- {}", q(s(v, "path")?)))),
        shell("find_duplicates", F, "Find files with identical content", &[("path", "string", "Folder (default /workspace)", false)],
            |v| Ok(format!("find {} -type f -not -path '*/.*' -exec sha256sum {{}} + | sort | awk '{{if($1==p){{print l; print $0}} p=$1; l=$0}}' | uniq | head -200", q(s_or(v, "path", "/workspace"))))),
        shell("folder_size", F, "Size of a folder and its biggest items", &[("path", "string", "Folder (default /workspace)", false)],
            |v| Ok(format!("du -sh -- {p} && du -ah -- {p} 2>/dev/null | sort -rh | head -20", p = q(s_or(v, "path", "/workspace"))))),
        shell("disk_free", R, "Free disk space", &[], |_| Ok("df -h /workspace".into())),
        shell("file_replace_text", F, "Find and replace text inside a file (keeps a .bak copy)", &[PATH, ("find", "string", "Text to find", true), ("replace", "string", "Replacement", true)],
            |v| Ok(py(&format!("import shutil\np={}\nshutil.copy(p, p+'.bak')\nt=open(p,encoding='utf-8').read()\nn=t.count({})\nopen(p,'w',encoding='utf-8').write(t.replace({}, {}))\nprint(f'replaced {{n}} occurrence(s); backup at {{p}}.bak')", pyq(s(v, "path")?), pyq(s(v, "find")?), pyq(s(v, "find")?), pyq(s(v, "replace")?))))),
        shell("csv_preview", F, "Show the first rows of a CSV file as a table", &[PATH, ("rows", "integer", "Rows (default 10)", false)],
            |v| Ok(py(&format!("import csv\nrows=list(csv.reader(open({},newline='',encoding='utf-8-sig')))\nfor r in rows[:{}+1]: print(' | '.join(r))\nprint(f'... {{len(rows)-1}} data rows total')", pyq(s(v, "path")?), n_or(v, "rows", 10.0) as u64)))),
        shell("json_file_query", F, "Query a JSON file with a jq filter", &[PATH, ("filter", "string", "jq filter, e.g. .items[].name", true)],
            |v| Ok(format!("jq -r {} -- {}", q(s(v, "filter")?), q(s(v, "path")?)))),
        shell("image_info", F, "Format and pixel size of an image file", &[PATH],
            |v| Ok(py(&format!("import struct\np={}\nb=open(p,'rb').read(64*1024)\nif b[:8]==b'\\x89PNG\\r\\n\\x1a\\n': w,h=struct.unpack('>II',b[16:24]); print('PNG',w,'x',h)\nelif b[:3]==b'GIF': w,h=struct.unpack('<HH',b[6:10]); print('GIF',w,'x',h)\nelif b[:2]==b'\\xff\\xd8':\n  i=2\n  while i<len(b):\n    if b[i]!=0xff: i+=1; continue\n    m=b[i+1]\n    if m in (0xc0,0xc1,0xc2):\n      h,w=struct.unpack('>HH',b[i+5:i+9]); print('JPEG',w,'x',h); break\n    i+=2+struct.unpack('>H',b[i+2:i+4])[0]\nelif b[:4]==b'RIFF' and b[8:12]==b'WEBP': print('WEBP')\nelse: print('unknown format')\nimport os; print(os.path.getsize(p),'bytes')", pyq(s(v, "path")?))))),
        shell("pdf_to_text", F, "Extract text from a PDF (pdftotext if installed, else a basic reader)", &[PATH],
            |v| { let p = q(s(v, "path")?); Ok(format!("if command -v pdftotext >/dev/null; then pdftotext -layout -- {p} - | head -c 200000; else strings -n 4 -- {p} | grep -v '^[/<>]' | head -400; echo '(basic extraction: install poppler-utils for proper text)'; fi")) }),
        // --- archives
        shell("zip_create", A, "Create a .zip from a file or folder", &[("source", "string", "File or folder", true), ("output", "string", "Output .zip path", true)],
            |v| Ok(py(&format!("import shutil,os\nsrc={}\nout={}\nbase=out[:-4] if out.endswith('.zip') else out\nif os.path.isdir(src): shutil.make_archive(base,'zip',src)\nelse:\n  import zipfile\n  with zipfile.ZipFile(base+'.zip','w',zipfile.ZIP_DEFLATED) as z: z.write(src, os.path.basename(src))\nprint('created', base+'.zip', os.path.getsize(base+'.zip'), 'bytes')", pyq(s(v, "source")?), pyq(s(v, "output")?))))),
        shell("zip_extract", A, "Extract a .zip (refuses paths that escape the target folder)", &[("archive", "string", ".zip file", true), ("to", "string", "Target folder", true)],
            |v| Ok(py(&format!("import zipfile,os\nto=os.path.realpath({})\nos.makedirs(to,exist_ok=True)\nwith zipfile.ZipFile({}) as z:\n  for m in z.namelist():\n    d=os.path.realpath(os.path.join(to,m))\n    if not d.startswith(to+os.sep) and d!=to: raise SystemExit('refused: '+m+' escapes the folder')\n  z.extractall(to)\n  print('extracted', len(z.namelist()), 'entries to', to)", pyq(s(v, "to")?), pyq(s(v, "archive")?))))),
        shell("zip_list", A, "List the contents of a .zip", &[("archive", "string", ".zip file", true)],
            |v| Ok(py(&format!("import zipfile\nfor i in zipfile.ZipFile({}).infolist(): print(f'{{i.file_size:>10}}  {{i.filename}}')", pyq(s(v, "archive")?))))),
        shell("tar_create", A, "Create a .tar.gz from a file or folder", &[("source", "string", "File or folder", true), ("output", "string", "Output .tar.gz", true)],
            |v| Ok(format!("tar -czf {} -- {} && ls -la -- {}", q(s(v, "output")?), q(s(v, "source")?), q(s(v, "output")?)))),
        shell("tar_extract", A, "Extract a .tar / .tar.gz into a folder", &[("archive", "string", "Archive", true), ("to", "string", "Target folder", true)],
            |v| Ok(format!("mkdir -p -- {to} && tar --no-same-owner -xf {} -C {to} && echo extracted", q(s(v, "archive")?), to = q(s(v, "to")?)))),
        // --- web & network
        shell("download_url", W, "Download a URL to a file in /workspace", &[("url", "string", "http(s) URL", true), ("path", "string", "Where to save (default: name from the URL)", false)],
            |v| { let u = url_arg(v)?; Ok(match v.get("path").and_then(Value::as_str) { Some(p) => format!("mkdir -p \"$(dirname -- {p})\" && curl -fsSL --max-time 300 -o {p} {u} && ls -la -- {p}", p = q(p)), None => format!("curl -fsSLO --max-time 300 {u} && ls -lat | head -3") }) }),
        shell("http_get", W, "Fetch a URL and show the response body (first 20 KB)", &[("url", "string", "http(s) URL", true)],
            |v| Ok(format!("curl -fsSL --max-time 60 {} | head -c 20000", url_arg(v)?))),
        shell("http_headers", W, "Show a URL's status and response headers", &[("url", "string", "http(s) URL", true)],
            |v| Ok(format!("curl -sSIL --max-time 30 {}", url_arg(v)?))),
        shell("http_status", W, "Status code and timing for a URL", &[("url", "string", "http(s) URL", true)],
            |v| Ok(format!("curl -s -o /dev/null --max-time 30 -w 'status %{{http_code}}\\ntime %{{time_total}}s\\nfinal %{{url_effective}}\\n' -L {}", url_arg(v)?))),
        shell("tls_certificate", W, "When a site's TLS certificate expires", &[("host", "string", "Hostname, e.g. example.com", true)],
            |v| { let h = s(v, "host")?; if !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') { return Err("hostname only".into()); } Ok(py(&format!("import ssl,socket,datetime\nh={}\nc=ssl.create_default_context().wrap_socket(socket.create_connection((h,443),timeout=15),server_hostname=h).getpeercert()\nprint('subject', dict(x[0] for x in c['subject']).get('commonName'))\nprint('issuer', dict(x[0] for x in c['issuer']).get('organizationName'))\nprint('expires', c['notAfter'])", pyq(h)))) }),
        shell("dns_lookup", W, "Resolve a hostname to addresses", &[("host", "string", "Hostname", true)],
            |v| { let h = s(v, "host")?; if !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') { return Err("hostname only".into()); } Ok(py(&format!("import socket\nfor a in sorted({{i[4][0] for i in socket.getaddrinfo({}, None)}}): print(a)", pyq(h)))) }),
        shell("robots_txt", W, "Read a site's robots.txt (what automated visitors may fetch)", &[("site", "string", "https://example.com", true)],
            |v| { let site = s(v, "site")?.trim_end_matches('/'); if !site.starts_with("http") { return Err("give an http(s) site".into()); } Ok(format!("curl -fsSL --max-time 20 {}/robots.txt | head -200", q(site))) }),
        shell("git_clone", W, "Clone a git repository into /workspace/repos", &[("url", "string", "Repository URL", true)],
            |v| { let u = s(v, "url")?; if !(u.starts_with("https://") || u.starts_with("git@")) { return Err("https or ssh git URLs only".into()); } Ok(format!("mkdir -p /workspace/repos && cd /workspace/repos && git clone --depth 50 -- {} && ls", q(u))) }),
        shell("git_status", R, "git status and recent commits of a repository", &[PATH],
            |v| Ok(format!("cd -- {} && git status -sb && git log --oneline -10", q(s(v, "path")?)))),
        // --- run & system
        shell("python_run", R, "Run a Python 3 script inside the computer", &[("code", "string", "Python source", true)],
            |v| Ok(py(s(v, "code")?))),
        shell("process_list", R, "Running processes (top by CPU)", &[], |_| Ok("ps aux --sort=-%cpu | head -20".into())),
        shell("system_info", R, "OS, CPU, memory and tool versions in the computer", &[],
            |_| Ok("uname -a; echo; (cat /etc/os-release | head -3); echo; nproc; free -h; echo; python3 --version; git --version; curl --version | head -1; jq --version".into())),
        shell("env_tools", R, "Which common tools are installed", &[],
            |_| Ok("for t in python3 node git curl wget jq unzip tar zip pdftotext convert ffmpeg sqlite3; do printf '%-10s %s\\n' $t \"$(command -v $t || echo missing)\"; done".into())),
        shell("sqlite_query", R, "Run a read-only SQL query on a SQLite file", &[PATH, ("sql", "string", "SELECT query", true)],
            |v| { let sql = s(v, "sql")?; if !sql.trim_start().to_lowercase().starts_with("select") && !sql.trim_start().to_lowercase().starts_with("with") { return Err("read-only: SELECT or WITH queries only".into()); } Ok(py(&format!("import sqlite3\nc=sqlite3.connect('file:'+{}+'?mode=ro', uri=True)\ncur=c.execute({})\nprint(' | '.join(d[0] for d in cur.description or []))\nfor r in cur.fetchmany(200): print(' | '.join(map(str,r)))", pyq(s(v, "path")?), pyq(sql)))) }),
        shell("wait_for_file", R, "Wait (up to N seconds) for a file to appear, e.g. a download", &[PATH, ("seconds", "integer", "Max wait (default 60)", false)],
            |v| Ok(format!("for i in $(seq 1 {}); do [ -e {p} ] && ls -la -- {p} && exit 0; sleep 1; done; echo 'not there yet'", n_or(v, "seconds", 60.0).min(600.0) as u64, p = q(s(v, "path")?)))),
        shell("browser_downloads", F, "List files the browser downloaded", &[], |_| Ok("ls -lat ~/Downloads 2>/dev/null | head -30 || echo 'no downloads folder'".into())),
        shell("count_files", F, "Count files by extension in a folder", &[("path", "string", "Folder (default /workspace)", false)],
            |v| Ok(format!("find {} -type f -not -path '*/.*' | sed -E 's/.*\\.([^./]+)$/\\1/;t;s/.*/(none)/' | sort | uniq -c | sort -rn | head -30", q(s_or(v, "path", "/workspace"))))),
        shell("touch_timestamp", F, "Create a file or update its modified time", &[PATH], |v| Ok(format!("touch -- {} && echo ok", q(s(v, "path")?)))),
        shell("open_in_editor_view", F, "Show a file with line numbers", &[PATH, ("from", "integer", "First line (default 1)", false), ("lines", "integer", "How many (default 80)", false)],
            |v| { let from = n_or(v, "from", 1.0).max(1.0) as u64; Ok(format!("awk -v a={from} -v b={} 'NR>=a && NR<b {{printf \"%5d  %s\\n\", NR, $0}}' -- {}", from + n_or(v, "lines", 80.0) as u64, q(s(v, "path")?))) }),
        shell("markdown_file_to_html", F, "Convert a Markdown file to an HTML file next to it", &[PATH],
            |v| Ok(py(&format!("import html,re,os\np={}\nt=open(p,encoding='utf-8').read()\nout=[]\nfor line in t.split('\\n'):\n  m=re.match(r'^(#{{1,6}}) (.*)',line)\n  if m: out.append(f'<h{{len(m.group(1))}}>{{html.escape(m.group(2))}}</h{{len(m.group(1))}}>')\n  elif line.startswith(('- ','* ')): out.append('<li>'+html.escape(line[2:])+'</li>')\n  elif line.strip(): out.append('<p>'+html.escape(line)+'</p>')\nh=os.path.splitext(p)[0]+'.html'\nopen(h,'w',encoding='utf-8').write('<!doctype html><meta charset=utf-8>'+'\\n'.join(out))\nprint('wrote',h)", pyq(s(v, "path")?))))),
        shell("lines_matching", F, "Lines of a file that match a regex (with line numbers)", &[PATH, ("pattern", "string", "Regular expression", true), ("ignore_case", "boolean", "Case-insensitive", false)],
            |v| Ok(format!("grep -n{} -E -e {} -- {} | head -300", if b_or(v, "ignore_case", false) { "i" } else { "" }, q(s(v, "pattern")?), q(s(v, "path")?)))),
    ]
}

/// A Python string literal (repr-style, single-quoted, escaped).
fn pyq(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_literals_cannot_break_out() {
        assert_eq!(pyq("a'b\\c\nd"), r"'a\'b\\c\nd'");
        // The heredoc delimiter is quoted ('PYEOF'), so $ and ` are inert; a
        // literal PYEOF line inside code would end it early — reject that.
        let cmd = py("print('$HOME `id`')");
        assert!(cmd.starts_with("python3 - <<'PYEOF'\n"));
        let tricky = py("x = 1\nPYEOF\nrm -rf ~");
        assert!(tricky.starts_with("python3 - <<'PYEOF_'\n") && tricky.ends_with("\nPYEOF_"));
    }
}
