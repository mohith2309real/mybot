//! Windows only: give mybot2.exe the MyBot icon, so Explorer, pinned taskbar
//! items and shortcuts show it (the window icon is set at runtime elsewhere).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = dir.join("../../assets/AppIcon.ico").canonicalize().expect("assets/AppIcon.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    // An absolute path, so the resource compiler doesn't have to guess what a
    // relative one is relative to.
    let rc = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("mybot.rc");
    let path = ico.display().to_string().trim_start_matches(r"\\?\").replace('\\', "\\\\");
    std::fs::write(&rc, format!("1 ICON \"{path}\"\n")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().unwrap();
}
