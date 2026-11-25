//! Native, read-only OpenCode observation and dialog-driven Factory management.
mod app;
mod backend;
mod connection_info;
mod conversation;
mod design;
mod dialogs;
mod directs;
mod infrastructure;
mod i18n;
pub(crate) use i18n::tr;
mod local_runners;
mod model;
mod model_settings;
mod mcp_settings;
mod native;
mod operations;
mod offline;
mod overview;
mod patch_view;
mod persistence;
mod project_settings;
mod project_logo;
mod skill_settings;
mod factory_skills;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod language_tests;
mod views;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "YoloForge Desktop — native Factory interface")]
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
