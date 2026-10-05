//! Load every Agent Skills folder under a directory and report parse results
//! and scan findings: `cargo run -p mybot-catalog --example check_skills -- /mnt/skills`
use mybot_catalog::agent_skill::{load_dir, scan, Severity};

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "/mnt/skills".into());
    let mut ok = 0;
    let mut bad = 0;
    let mut dirs = Vec::new();
    for group in std::fs::read_dir(&root).unwrap().flatten() {
        for d in std::fs::read_dir(group.path()).into_iter().flatten().flatten() {
            if d.path().join("SKILL.md").exists() { dirs.push(d.path()); }
        }
    }
    dirs.sort();
    for d in dirs {
        match load_dir(&d) {
            Ok(p) => {
                ok += 1;
                let f = scan(&p);
                let high = f.iter().filter(|x| x.severity == Severity::High).count();
                let med = f.iter().filter(|x| x.severity == Severity::Medium).count();
                println!("ok   {:<32} {:>3} files  high {high:<2} medium {med:<3} — {}", p.doc.name, p.files.len(), p.doc.description.chars().take(60).collect::<String>());
            }
            Err(e) => { bad += 1; println!("FAIL {}: {e}", d.display()); }
        }
    }
    println!("\n{ok} loaded, {bad} failed");
}
