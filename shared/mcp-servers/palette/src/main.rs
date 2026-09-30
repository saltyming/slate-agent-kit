//! Entry point of the palette command: MCP server over stdio and the command-line checker.
//!
//! Owns argument handling and process startup only. `palette` serves MCP,
//! `palette check <project>` prints lint findings, `palette generate <project>` writes the
//! generated files, `palette --read-only-tools` prints
//! the read tool names and `palette --version` prints the version. The work is in the
//! `palette_server` library.

use std::path::PathBuf;

use palette_server::ops::Ctx;
use palette_server::project::{ProcessEnv, Roots};
use palette_server::server::{PaletteServer, read_only_tool_names};
use rmcp::ServiceExt;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None => serve(),
        Some("check") => match args.get(1) {
            Some(p) if args.len() == 2 => palette_server::cli::check(p),
            _ => usage(),
        },
        Some("generate") => match args.get(1) {
            Some(p) if args.len() == 2 => palette_server::cli::generate(p),
            _ => usage(),
        },
        Some("--read-only-tools") if args.len() == 1 => {
            for name in read_only_tool_names() {
                println!("{name}");
            }
            0
        }
        Some("--version") if args.len() == 1 => {
            println!("palette {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(_) => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!("usage: palette                  serve MCP over stdio");
    eprintln!("       palette check <project>  print lint findings; exit 1 on an error");
    eprintln!("       palette generate <project>  write the indexes and staging documents");
    eprintln!("       palette --read-only-tools");
    eprintln!("       palette --version");
    2
}

fn serve() -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("palette: cannot start the runtime: {e}");
            return 1;
        }
    };
    match rt.block_on(run_server()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("palette: {e}");
            1
        }
    }
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cwd: PathBuf = std::env::current_dir()?;
    let roots = Roots::from_env(&ProcessEnv, &cwd);
    if roots.project_root.is_none() && roots.extra_roots.is_empty() {
        tracing::warn!(
            "no project root: cwd {} is not a plausible project directory and no *_PROJECT_DIR env is set; every tool will fail with no_project_root until PALETTE_EXTRA_ROOTS or SLATE_PROJECT_DIR is set",
            cwd.display()
        );
    }
    let server = PaletteServer::new(Ctx::new(roots));
    let running = server.serve(rmcp::transport::io::stdio()).await?;
    tokio::select! {
        r = running.waiting() => { r?; }
        _ = shutdown_signal() => {}
    }
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
