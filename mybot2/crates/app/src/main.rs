//! MyBot 2.0: with no arguments, the desktop app; with arguments, the command line.

mod cli;
mod engine;
mod gui;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let result = if matches!(cli.command, None | Some(cli::Cmd::App)) {
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

fn app() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let engine = engine::Engine::open()?;
    gui::run(engine, rt.handle().clone()).map_err(|e| anyhow::anyhow!(e.to_string()))
}
