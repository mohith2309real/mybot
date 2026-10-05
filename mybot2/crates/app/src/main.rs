//! MyBot 2.0: with no arguments, the desktop app; with arguments, the command line.

mod cli;
mod engine;
mod gui;

use clap::Parser;

fn main() {
    // Older macOS hands apps opened from Finder a `-psn_…` argument.
    let cli = cli::Cli::parse_from(std::env::args().filter(|a| !a.starts_with("-psn_")));
    let result = if matches!(cli.command, None | Some(cli::Cmd::App)) {
        #[cfg(windows)]
        hide_console_if_ours();
        app()
    } else {
        // `mybot2 skills list | head` should end quietly, not panic.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        cli::main(cli)
    };
    if let Err(e) = result {
        eprintln!("mybot2: {e:#}");
        std::process::exit(1);
    }
}

/// Double-clicked from Explorer, Windows opens a console just for us: close
/// it, so only the window shows. Run from a terminal, the console is shared
/// and stays.
#[cfg(windows)]
fn hide_console_if_ours() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 2];
    // SAFETY: the buffer holds 2 ids, as stated.
    if unsafe { GetConsoleProcessList(ids.as_mut_ptr(), 2) } == 1 {
        // SAFETY: detaching from a console has no preconditions.
        unsafe { FreeConsole() };
    }
}

fn app() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let engine = engine::Engine::open()?;
    gui::run(engine, rt.handle().clone()).map_err(|e| anyhow::anyhow!(e.to_string()))
}
