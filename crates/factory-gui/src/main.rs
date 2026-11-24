//! Native, read-only OpenCode observation and dialog-driven Factory management.
mod app;
mod backend;
mod connection_info;
mod conversation;
mod design;
mod dialogs;
mod infrastructure;
mod model;
mod model_settings;
mod native;
mod operations;
mod overview;
mod patch_view;
mod persistence;
mod project_settings;
#[cfg(test)]
mod tests;
mod views;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "YoloForge Desktop — native Factory-Oberfläche")]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:7432")]
    server: String,
    /// Save this window only, for repeatable visual smoke tests.
    #[arg(long, hide = true)]
    screenshot: Option<std::path::PathBuf>,
    #[arg(long, hide = true)]
    exit_after_ms: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    native::run(args.server, args.screenshot, args.exit_after_ms)
}
