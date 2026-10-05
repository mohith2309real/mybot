//! Data actions: JSON, CSV, TOML, URLs and encodings.

use base64::Engine;
use serde_json::{Map, Value, json};

use crate::{Action, b_or, local, n_or, num, s, s_or};

const C: &str = "Data";

fn parse_json(v: &Value, k: &str) -> Result<Value, String> {
    match v.get(k) {
        Some(Value::String(t)) => serde_json::from_str(t).map_err(|e| format!("\"{k}\" is not valid JSON: {e}")),
        Some(other) => Ok(other.clone()),
        None => Err(format!("missing \"{k}\" (JSON)")),
    }
}

/// `a.b[0].c` or `a.b.0.c`
pub fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    let norm = path.trim().trim_start_matches('$').trim_start_matches('.').replace('[', ".").replace(']', "");
    for part in norm.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            Value::Object(o) => o.get(part)?,
            _ => return None,
        };
    }
    Some(cur)
}

pub fn flatten(prefix: &str, v: &Value, out: &mut Map<String, Value>) {
    match v {
        Value::Object(o) if !o.is_empty() => {
            for (k, x) in o {
                flatten(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, x, out);
            }
        }
        Value::Array(a) if !a.is_empty() => {
            for (i, x) in a.iter().enumerate() {
                flatten(&if prefix.is_empty() { i.to_string() } else { format!("{prefix}.{i}") }, x, out);
            }
        }
        _ => {
            out.insert(prefix.to_string(), v.clone());
        }
    }
}

fn unflatten(flat: &Map<String, Value>) -> Value {
    fn insert(map: &mut Map<String, Value>, parts: &[&str], v: &Value) {
        let Some((first, rest)) = parts.split_first() else { return };
        if rest.is_empty() {
            map.insert((*first).to_string(), v.clone());
            return;
        }
        let child = map.entry((*first).to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !child.is_object() {
            *child = Value::Object(Map::new());
        }
        if let Value::Object(m) = child {
            insert(m, rest, v);
        }
    }
    let mut root = Map::new();
    for (k, v) in flat {
        let parts: Vec<&str> = k.split('.').collect();
        insert(&mut root, &parts, v);
    }
    Value::Object(root)
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn read_csv(text: &str, delimiter: u8) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut r = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = r.headers().map_err(|e| e.to_string())?.iter().map(|h| h.trim().to_string()).collect();
    let mut rows = Vec::new();
    for rec in r.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        rows.push(rec.iter().map(String::from).collect());
    }
    Ok((headers, rows))
}

pub fn write_csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut w = csv::Writer::from_writer(vec![]);
    let _ = w.write_record(headers);
    for r in rows {
        let _ = w.write_record(r);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn delim(v: &Value) -> u8 {
    s_or(v, "delimiter", ",").bytes().next().unwrap_or(b',')
}

fn col_index(headers: &[String], name: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| format!("no column \"{name}\" (columns: {})", headers.join(", ")))
}

fn csv_in(v: &Value) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    read_csv(s(v, "csv")?, delim(v))
}

const CSV: (&str, &str, &str, bool) = ("csv", "string", "CSV text with a header row", true);
const DELIM: (&str, &str, &str, bool) = ("delimiter", "string", "Field separator (default ,)", false);

pub fn defs() -> Vec<Action> {
    vec![
        local("json_pretty", C, "Pretty-print JSON", &[("json", "string", "JSON text", true)],
            |v| serde_json::to_string_pretty(&parse_json(v, "json")?).map_err(|e| e.to_string()),
            (json!({"json": "{\"a\":1}"}), "\"a\": 1")),
        local("json_minify", C, "Minify JSON", &[("json", "string", "JSON text", true)],
            |v| Ok(parse_json(v, "json")?.to_string()),
            (json!({"json": "{ \"a\" : [1, 2] }"}), "{\"a\":[1,2]}")),
        local("json_validate", C, "Check that text is valid JSON and say where it breaks", &[("json", "string", "JSON text", true)],
            |v| Ok(match serde_json::from_str::<Value>(s(v, "json")?) { Ok(_) => "valid JSON".into(), Err(e) => format!("invalid: {e}") }),
            (json!({"json": "{\"a\":}"}), "invalid")),
        local("json_query", C, "Get a value by path, e.g. items[0].name", &[("json", "string", "JSON text", true), ("path", "string", "Path like a.b[0].c", true)],
            |v| { let doc = parse_json(v, "json")?; json_path(&doc, s(v, "path")?).map(|x| match x { Value::String(t) => t.clone(), o => serde_json::to_string_pretty(o).unwrap_or_default() }).ok_or_else(|| "nothing at that path".into()) },
            (json!({"json": "{\"items\":[{\"name\":\"pen\"}]}", "path": "items[0].name"}), "pen")),
        local("json_keys", C, "List the keys of a JSON object (at an optional path)", &[("json", "string", "JSON text", true), ("path", "string", "Optional path", false)],
            |v| { let doc = parse_json(v, "json")?; let at = json_path(&doc, s_or(v, "path", "")).ok_or("nothing at that path")?; match at { Value::Object(o) => Ok(o.keys().cloned().collect::<Vec<_>>().join("\n")), Value::Array(a) => Ok(format!("an array of {} items", a.len())), _ => Err("not an object".into()) } },
            (json!({"json": "{\"b\":1,\"a\":2}"}), "a")),
        local("json_flatten", C, "Flatten nested JSON into dot.paths", &[("json", "string", "JSON text", true)],
            |v| { let mut out = Map::new(); flatten("", &parse_json(v, "json")?, &mut out); serde_json::to_string_pretty(&Value::Object(out)).map_err(|e| e.to_string()) },
            (json!({"json": "{\"a\":{\"b\":[1]}}"}), "\"a.b.0\": 1")),
        local("json_unflatten", C, "Turn dot.path keys back into nested JSON", &[("json", "string", "Flat JSON object", true)],
            |v| { let doc = parse_json(v, "json")?; let o = doc.as_object().ok_or("expected an object")?; serde_json::to_string_pretty(&unflatten(o)).map_err(|e| e.to_string()) },
            (json!({"json": "{\"a.b\":1}"}), "\"b\": 1")),
        local("json_merge", C, "Deep-merge two JSON objects (second wins)", &[("a", "string", "First JSON", true), ("b", "string", "Second JSON", true)],
            |v| {
                fn merge(a: &mut Value, b: &Value) { match (a, b) { (Value::Object(x), Value::Object(y)) => { for (k, vv) in y { merge(x.entry(k.clone()).or_insert(Value::Null), vv); } } (a, b) => *a = b.clone() } }
                let mut a = parse_json(v, "a")?; merge(&mut a, &parse_json(v, "b")?); serde_json::to_string_pretty(&a).map_err(|e| e.to_string())
            },
            (json!({"a": "{\"x\":{\"y\":1,\"z\":1}}", "b": "{\"x\":{\"z\":2}}"}), "\"z\": 2")),
        local("json_diff", C, "List the differences between two JSON documents", &[("a", "string", "Old JSON", true), ("b", "string", "New JSON", true)],
            |v| {
                let (mut fa, mut fb) = (Map::new(), Map::new());
                flatten("", &parse_json(v, "a")?, &mut fa); flatten("", &parse_json(v, "b")?, &mut fb);
                let mut out = Vec::new();
                for (k, x) in &fa { match fb.get(k) { None => out.push(format!("- {k}: {x}")), Some(y) if y != x => out.push(format!("~ {k}: {x} → {y}")), _ => {} } }
                for (k, y) in &fb { if !fa.contains_key(k) { out.push(format!("+ {k}: {y}")); } }
                Ok(if out.is_empty() { "identical".into() } else { out.join("\n") })
            },
            (json!({"a": "{\"p\":1,\"q\":2}", "b": "{\"p\":1,\"q\":3,\"r\":4}"}), "~ q: 2 → 3")),
        local("json_to_csv", C, "Turn a JSON array of objects into CSV", &[("json", "string", "JSON array", true)],
            |v| {
                let doc = parse_json(v, "json")?;
                let arr = doc.as_array().ok_or("expected a JSON array")?;
                let mut headers: Vec<String> = Vec::new();
                let mut flat_rows = Vec::new();
                for item in arr { let mut m = Map::new(); flatten("", item, &mut m); for k in m.keys() { if !headers.contains(k) { headers.push(k.clone()); } } flat_rows.push(m); }
                let rows: Vec<Vec<String>> = flat_rows.iter().map(|m| headers.iter().map(|h| m.get(h).map(cell).unwrap_or_default()).collect()).collect();
                Ok(write_csv(&headers, &rows))
            },
            (json!({"json": "[{\"a\":1,\"b\":{\"c\":\"x\"}}]"}), "a,b.c\n1,x")),
        local("csv_to_json", C, "Turn CSV into a JSON array of objects", &[CSV, DELIM],
            |v| { let (h, rows) = csv_in(v)?; let out: Vec<Value> = rows.iter().map(|r| Value::Object(h.iter().cloned().zip(r.iter().map(|c| json!(c))).collect())).collect(); serde_json::to_string_pretty(&out).map_err(|e| e.to_string()) },
            (json!({"csv": "name,age\nAda,36"}), "\"name\": \"Ada\"")),
        local("csv_columns", C, "List CSV columns with types and sample values", &[CSV, DELIM],
            |v| {
                let (h, rows) = csv_in(v)?;
                Ok(h.iter().enumerate().map(|(i, name)| {
                    let vals: Vec<&str> = rows.iter().filter_map(|r| r.get(i)).map(|x| x.as_str()).filter(|x| !x.trim().is_empty()).collect();
                    let numeric = !vals.is_empty() && vals.iter().all(|x| x.trim().parse::<f64>().is_ok());
                    format!("{name}: {} ({} filled of {}), e.g. {}", if numeric { "number" } else { "text" }, vals.len(), rows.len(), vals.first().unwrap_or(&""))
                }).collect::<Vec<_>>().join("\n"))
            },
            (json!({"csv": "a,b\n1,x\n2,"}), "a: number (2 filled of 2)")),
        local("csv_stats", C, "Count, min, max, mean, median and sum for numeric columns", &[CSV, DELIM],
            |v| {
                let (h, rows) = csv_in(v)?;
                let mut out = Vec::new();
                for (i, name) in h.iter().enumerate() {
                    let mut xs: Vec<f64> = rows.iter().filter_map(|r| r.get(i)).filter_map(|x| x.trim().replace(',', "").parse().ok()).collect();
                    if xs.is_empty() { continue; }
                    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let sum: f64 = xs.iter().sum(); let n = xs.len();
                    let median = if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 };
                    out.push(format!("{name}: count {n}, min {}, max {}, mean {}, median {}, sum {}", num(xs[0]), num(xs[n - 1]), num(sum / n as f64), num(median), num(sum)));
                }
                Ok(if out.is_empty() { "No numeric columns.".into() } else { out.join("\n") })
            },
            (json!({"csv": "x\n1\n2\n6"}), "mean 3, median 2, sum 9")),
        local("csv_filter", C, "Keep rows where a column matches (=, !=, >, <, contains)", &[CSV, ("column", "string", "Column name", true), ("op", "string", "=, !=, >, <, >=, <=, contains", true), ("value", "string", "Value to compare", true), DELIM],
            |v| {
                let (h, rows) = csv_in(v)?; let i = col_index(&h, s(v, "column")?)?; let (op, want) = (s(v, "op")?, s(v, "value")?);
                let keep = |c: &str| -> bool { let (a, b) = (c.trim().parse::<f64>(), want.trim().parse::<f64>()); match (op, a, b) { (">", Ok(a), Ok(b)) => a > b, ("<", Ok(a), Ok(b)) => a < b, (">=", Ok(a), Ok(b)) => a >= b, ("<=", Ok(a), Ok(b)) => a <= b, ("=" | "==", _, _) => c.trim().eq_ignore_ascii_case(want.trim()), ("!=", _, _) => !c.trim().eq_ignore_ascii_case(want.trim()), ("contains", _, _) => c.to_lowercase().contains(&want.to_lowercase()), _ => false } };
                let out: Vec<Vec<String>> = rows.into_iter().filter(|r| r.get(i).is_some_and(|c| keep(c))).collect();
                Ok(write_csv(&h, &out))
            },
            (json!({"csv": "n,v\na,5\nb,15", "column": "v", "op": ">", "value": "10"}), "b,15")),
        local("csv_sort", C, "Sort rows by a column (numbers sort numerically)", &[CSV, ("column", "string", "Column", true), ("descending", "boolean", "Largest first", false), DELIM],
            |v| {
                let (h, mut rows) = csv_in(v)?; let i = col_index(&h, s(v, "column")?)?;
                rows.sort_by(|a, b| { let (x, y) = (a.get(i).cloned().unwrap_or_default(), b.get(i).cloned().unwrap_or_default()); match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) { (Ok(p), Ok(q)) => p.partial_cmp(&q).unwrap_or(std::cmp::Ordering::Equal), _ => x.to_lowercase().cmp(&y.to_lowercase()) } });
                if b_or(v, "descending", false) { rows.reverse(); }
                Ok(write_csv(&h, &rows))
            },
            (json!({"csv": "v\n10\n9", "column": "v"}), "v\n9\n10")),
        local("csv_select_columns", C, "Keep only some columns, in a given order", &[CSV, ("columns", "array", "Column names", true), DELIM],
            |v| { let (h, rows) = csv_in(v)?; let cols: Vec<usize> = crate::list(v, "columns")?.iter().map(|c| col_index(&h, &cell(c))).collect::<Result<_, _>>()?; Ok(write_csv(&cols.iter().map(|&i| h[i].clone()).collect::<Vec<_>>(), &rows.iter().map(|r| cols.iter().map(|&i| r.get(i).cloned().unwrap_or_default()).collect()).collect::<Vec<_>>())) },
            (json!({"csv": "a,b,c\n1,2,3", "columns": ["c", "a"]}), "c,a\n3,1")),
        local("csv_dedupe", C, "Remove duplicate rows (optionally by some columns)", &[CSV, ("columns", "array", "Columns to compare (default all)", false), DELIM],
            |v| {
                let (h, rows) = csv_in(v)?;
                let idx: Vec<usize> = match crate::list(v, "columns") { Ok(c) => c.iter().map(|x| col_index(&h, &cell(x))).collect::<Result<_, _>>()?, Err(_) => (0..h.len()).collect() };
                let mut seen = std::collections::HashSet::new();
                let out: Vec<Vec<String>> = rows.into_iter().filter(|r| seen.insert(idx.iter().map(|&i| r.get(i).map(|x| x.trim().to_lowercase()).unwrap_or_default()).collect::<Vec<_>>())).collect();
                Ok(write_csv(&h, &out))
            },
            (json!({"csv": "a\nx\nX\ny"}), "a\nx\ny")),
        local("csv_group_sum", C, "Group by a column and count/sum/average another", &[CSV, ("group_by", "string", "Column to group by", true), ("value", "string", "Numeric column to total", true), DELIM],
            |v| {
                let (h, rows) = csv_in(v)?; let g = col_index(&h, s(v, "group_by")?)?; let x = col_index(&h, s(v, "value")?)?;
                let mut m: std::collections::BTreeMap<String, (usize, f64)> = Default::default();
                for r in &rows { let e = m.entry(r.get(g).cloned().unwrap_or_default()).or_default(); e.0 += 1; e.1 += r.get(x).and_then(|c| c.trim().parse::<f64>().ok()).unwrap_or(0.0); }
                let out: Vec<Vec<String>> = m.into_iter().map(|(k, (c, sum))| vec![k, c.to_string(), num(sum), num(sum / c as f64)]).collect();
                Ok(write_csv(&["group".into(), "count".into(), "sum".into(), "mean".into()], &out))
            },
            (json!({"csv": "k,v\na,1\na,3\nb,5", "group_by": "k", "value": "v"}), "a,2,4,2")),
        local("csv_to_markdown_table", C, "Render CSV as a Markdown table", &[CSV, DELIM],
            |v| { let (h, rows) = csv_in(v)?; let mut out = vec![format!("| {} |", h.join(" | ")), format!("|{}|", h.iter().map(|_| "---").collect::<Vec<_>>().join("|"))]; for r in rows { out.push(format!("| {} |", r.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | "))); } Ok(out.join("\n")) },
            (json!({"csv": "a,b\n1,2"}), "| 1 | 2 |")),
        local("csv_transpose", C, "Swap rows and columns", &[CSV, DELIM],
            |v| { let (h, rows) = csv_in(v)?; let mut all = vec![h]; all.extend(rows); let w = all.iter().map(Vec::len).max().unwrap_or(0); let t: Vec<Vec<String>> = (0..w).map(|j| all.iter().map(|r| r.get(j).cloned().unwrap_or_default()).collect()).collect(); Ok(write_csv(&t[0], &t[1..])) },
            (json!({"csv": "a,b\n1,2"}), "a,1\nb,2")),
        local("csv_row_count", C, "Count data rows and columns", &[CSV, DELIM],
            |v| { let (h, rows) = csv_in(v)?; Ok(format!("{} rows, {} columns", rows.len(), h.len())) },
            (json!({"csv": "a,b\n1,2\n3,4"}), "2 rows, 2 columns")),
        local("toml_to_json", C, "Convert TOML to JSON", &[("toml", "string", "TOML text", true)],
            |v| { let t: toml::Value = toml::from_str(s(v, "toml")?).map_err(|e| e.to_string())?; serde_json::to_string_pretty(&t).map_err(|e| e.to_string()) },
            (json!({"toml": "a = 1\n[b]\nc = \"x\""}), "\"c\": \"x\"")),
        local("json_to_toml", C, "Convert a JSON object to TOML", &[("json", "string", "JSON object", true)],
            |v| { let j = parse_json(v, "json")?; let t: toml::Value = serde_json::from_value(j).map_err(|e| e.to_string())?; toml::to_string_pretty(&t).map_err(|e| e.to_string()) },
            (json!({"json": "{\"name\":\"x\"}"}), "name = \"x\"")),
        local("url_parse", C, "Split a URL into scheme, host, path, query and fragment", &[("url", "string", "URL", true)],
            |v| {
                let u = url::Url::parse(s(v, "url")?).map_err(|e| e.to_string())?;
                let q: Vec<String> = u.query_pairs().map(|(k, x)| format!("  {k} = {x}")).collect();
                Ok(format!("scheme: {}\nhost: {}\nport: {}\npath: {}\nquery:\n{}\nfragment: {}\norigin: {}", u.scheme(), u.host_str().unwrap_or(""), u.port_or_known_default().map(|p| p.to_string()).unwrap_or_default(), u.path(), q.join("\n"), u.fragment().unwrap_or(""), u.origin().ascii_serialization()))
            },
            (json!({"url": "https://a.example:8443/p?x=1#top"}), "origin: https://a.example:8443")),
        local("url_join", C, "Resolve a relative link against a base URL", &[("base", "string", "Base URL", true), ("link", "string", "Relative or absolute link", true)],
            |v| { let (base, link) = (s(v, "base")?, s(v, "link")?); url::Url::parse(base).and_then(|b| b.join(link)).map(|u| u.to_string()).map_err(|e| e.to_string()) },
            (json!({"base": "https://a.example/docs/x", "link": "../img.png"}), "https://a.example/img.png")),
        local("url_encode", C, "Percent-encode text for a URL", &[("text", "string", "Text", true)],
            |v| Ok(url::form_urlencoded::byte_serialize(s(v, "text")?.as_bytes()).collect()),
            (json!({"text": "a b&c"}), "a+b%26c")),
        local("url_decode", C, "Decode percent-encoded text", &[("text", "string", "Encoded text", true)],
            |v| Ok(url::form_urlencoded::parse(format!("x={}", s(v, "text")?).as_bytes()).next().map(|(_, x)| x.to_string()).unwrap_or_default()),
            (json!({"text": "a+b%26c"}), "a b&c")),
        local("query_string_build", C, "Build a query string from JSON key/values", &[("params", "string", "JSON object", true)],
            |v| { let p = parse_json(v, "params")?; let o = p.as_object().ok_or("expected an object")?; let mut s2 = url::form_urlencoded::Serializer::new(String::new()); for (k, x) in o { s2.append_pair(k, &cell(x)); } Ok(s2.finish()) },
            (json!({"params": "{\"q\":\"red shoes\",\"page\":2}"}), "q=red+shoes")),
        local("base64_encode", C, "Base64-encode text", &[("text", "string", "Text", true), ("url_safe", "boolean", "URL-safe alphabet", false)],
            |v| { let t = s(v, "text")?.as_bytes(); Ok(if b_or(v, "url_safe", false) { base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(t) } else { base64::engine::general_purpose::STANDARD.encode(t) }) },
            (json!({"text": "hello"}), "aGVsbG8=")),
        local("base64_decode", C, "Decode base64 to text", &[("text", "string", "Base64", true)],
            |v| {
                let t = s(v, "text")?.trim();
                let bytes = base64::engine::general_purpose::STANDARD.decode(t).or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t.trim_end_matches('='))).map_err(|e| e.to_string())?;
                Ok(String::from_utf8(bytes.clone()).unwrap_or_else(|_| format!("(binary, {} bytes) {}", bytes.len(), hex::encode(&bytes[..bytes.len().min(32)]))))
            },
            (json!({"text": "aGVsbG8="}), "hello")),
        local("hex_encode", C, "Hex-encode text", &[("text", "string", "Text", true)], |v| Ok(hex::encode(s(v, "text")?)), (json!({"text": "hi"}), "6869")),
        local("hex_decode", C, "Decode hex to text", &[("text", "string", "Hex", true)],
            |v| { let b = hex::decode(s(v, "text")?.trim().trim_start_matches("0x")).map_err(|e| e.to_string())?; Ok(String::from_utf8_lossy(&b).to_string()) },
            (json!({"text": "6869"}), "hi")),
        local("list_unique", C, "Unique items of a list, in order", &[("items", "array", "Items", true)],
            |v| { let mut seen = std::collections::HashSet::new(); Ok(crate::list(v, "items")?.iter().map(cell).filter(|x| seen.insert(x.clone())).collect::<Vec<_>>().join("\n")) },
            (json!({"items": ["a", "b", "a"]}), "a\nb")),
        local("list_sort", C, "Sort a list", &[("items", "array", "Items", true), ("descending", "boolean", "Largest first", false)],
            |v| { let mut it: Vec<String> = crate::list(v, "items")?.iter().map(cell).collect(); it.sort_by(|a, b| match (a.parse::<f64>(), b.parse::<f64>()) { (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal), _ => a.to_lowercase().cmp(&b.to_lowercase()) }); if b_or(v, "descending", false) { it.reverse(); } Ok(it.join("\n")) },
            (json!({"items": [3, 1, 2]}), "1\n2\n3")),
        local("list_chunk", C, "Split a list into groups of N", &[("items", "array", "Items", true), ("size", "integer", "Group size", true)],
            |v| { let it: Vec<String> = crate::list(v, "items")?.iter().map(cell).collect(); let n = n_or(v, "size", 10.0).max(1.0) as usize; Ok(it.chunks(n).enumerate().map(|(i, c)| format!("group {}: {}", i + 1, c.join(", "))).collect::<Vec<_>>().join("\n")) },
            (json!({"items": [1, 2, 3], "size": 2}), "group 2: 3")),
        local("list_compare", C, "Items only in A, only in B, and in both", &[("a", "array", "List A", true), ("b", "array", "List B", true)],
            |v| {
                let a: Vec<String> = crate::list(v, "a")?.iter().map(cell).collect(); let b: Vec<String> = crate::list(v, "b")?.iter().map(cell).collect();
                let only_a: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect(); let only_b: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect(); let both: Vec<&String> = a.iter().filter(|x| b.contains(x)).collect();
                Ok(format!("only in A: {:?}\nonly in B: {:?}\nin both: {:?}", only_a, only_b, both))
            },
            (json!({"a": ["x", "y"], "b": ["y", "z"]}), "in both: [\"y\"]")),
    ]
}
