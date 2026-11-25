//! Export the verified runtime for a GUI package, never the runtime state.
use anyhow::{ensure, Context, Result};
use std::{fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let output = PathBuf::from(
        args.next()
            .context("Expected the package binary directory")?,
    );
    ensure!(args.next().is_none(), "Expected only one argument");
    let cache = tempfile::tempdir()?;
    let binary = factory_opencode::distribution::binary(cache.path(), None).await?;
    let status = tokio::process::Command::new(&binary)
        .arg("--version")
        .output()
        .await?;
    let version = factory_opencode::distribution::VERSION;
    ensure!(
        status.status.success()
            && std::str::from_utf8(&status.stdout)?.trim() == format!("opencode v{version}"),
        "Bundled OpenCode did not report the pinned version"
    );
    let destination = output.join(
        binary
            .file_name()
            .context("OpenCode binary has no filename")?,
    );
    fs::copy(binary, destination)?;
    Ok(())
}
