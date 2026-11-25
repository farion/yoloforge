//! Pinned OpenCode distribution for the Factory-owned runtimes.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub const VERSION: &str = "2.4.1";

fn artifact() -> Result<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok(("linux-x64", "1584009e779594b9aacbceb6ebbe2fbd15648fcc87b990051c9195dd4086431a3c68a1b9f081cf2abb296f1aee55a8e7204a5bd2df0f78eeb71ffa16e2f246e4")),
        ("linux", "aarch64") => Ok(("linux-arm64", "a2534a7e802e1da0ed1eb5ab96ac61d913d2a2749f0193c6265aeeeed30d78d4e51f22d2ad36c37bff8727abdd611a7b838c8a2d243c486dbae14fc80bb3e0b7")),
        ("macos", "x86_64") => Ok(("darwin-x64", "27f9212e97b4a62b131830f98e975d423a9be5ef2a52c598a62464a8804dddafcfe12e5ab1f3f612b2ca49ebe6a46ce183d7c2d53a4b2671955f080df892602a")),
        ("macos", "aarch64") => Ok(("darwin-arm64", "afa3c8665e70a85c5561f5458e6d892b8b9fd5add84fb14c1154ce0c917fd6129eeb005d2bb4c7cdb00cd524b2b91b8c9d29036fdf9d77cc4d3a4f7340aeadae")),
        ("windows", "x86_64") => Ok(("windows-x64", "cd86f01fc69bdcbbf477ebb2bf0d29c6a4a37ff9bea113b11ced248c1043156bba658df56939d47228da3b111ced54ecb22c2493fd0721aa4000316e3ea66235")),
        _ => anyhow::bail!("Kein gepinnter OpenCode-Download für diese Plattform; FACTORY_OPENCODE_BIN setzen."),
    }
}

/// An explicit binary (including the preinstalled container binary) takes
/// precedence. Otherwise use only our private cache, never a personal PATH install.
pub async fn binary(runtime_dir: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        ensure!(
            path.is_absolute(),
            "OpenCode-Binary muss ein absoluter Pfad sein"
        );
        return path
            .canonicalize()
            .context("Factory-OpenCode-Binary nicht gefunden");
    }
    let (arch, checksum) = artifact()?;
    let dir = runtime_dir.join(format!("distribution-{VERSION}-{arch}"));
    factory_platform::private_dir(&dir)?;
    let target = dir.join(if cfg!(windows) { "opencode.exe" } else { "opencode" });
    match fs::symlink_metadata(&target) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && factory_platform::is_executable(&target)
                    && factory_platform::ensure_private(&target).is_ok(),
                "Ungültige OpenCode-Datei im Cache: {}",
                target.display()
            );
            return Ok(target);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    eprintln!("Lade OpenCode {VERSION} für {arch} …");
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()?;
    let url = format!(
        "https://registry.npmjs.org/@opencode/cli-{arch}/-/cli-{arch}-{VERSION}.tgz"
    );
    let mut response = client
        .get(url)
        .send()
        .await
        .context("OpenCode-Download fehlgeschlagen")?
        .error_for_status()?;
    let mut archive = tempfile::NamedTempFile::new_in(&dir)?;
    let mut digest = Sha512::new();
    let mut length = 0;
    while let Some(chunk) = response.chunk().await? {
        length += chunk.len();
        ensure!(length <= 128 * 1024 * 1024, "OpenCode-Download ist zu groß");
        archive.write_all(&chunk)?;
        digest.update(&chunk);
    }
    ensure!(
        format!("{:x}", digest.finalize()) == checksum,
        "OpenCode-Prüfsumme stimmt nicht"
    );

    // Extract one verified regular member without trusting archive-supplied paths.
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    let member = if cfg!(windows) { "package/bin/opencode.exe" } else { "package/bin/opencode" };
    let mut input = tar::Archive::new(flate2::read::GzDecoder::new(archive.reopen()?));
    let mut found = false;
    for entry in input.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(member) {
            ensure!(!found && entry.header().entry_type().is_file() && entry.size() <= 512 * 1024 * 1024,
                "OpenCode-Archiv ungültig");
            std::io::copy(&mut entry, &mut file)?;
            found = true;
        }
    }
    ensure!(found && file.as_file().metadata()?.len() > 0, "OpenCode-Binary fehlt im Archiv");
    factory_platform::set_private_executable(file.path())?;
    file.as_file().sync_all()?;
    file.as_file().sync_all()?;
    file.persist(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_and_native_distribution_use_the_same_pin() {
        let installer = include_str!("../../../deploy/install-opencode.sh");
        assert!(installer
            .lines()
            .any(|line| line == format!("version={VERSION}")));
        if let Ok((arch, checksum)) = artifact() {
            if let Some(arch) = arch.strip_prefix("linux-") {
                assert!(installer.contains(&format!("arch={arch}; checksum={checksum}")));
            }
        }
    }

    #[tokio::test]
    #[ignore = "downloads the pinned OpenCode package from npm"]
    async fn downloads_verified_binary_and_reuses_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = binary(dir.path(), None).await.unwrap();
        let output = tokio::process::Command::new(&path)
            .arg("--version")
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("opencode v{VERSION}")
        );
        assert_eq!(binary(dir.path(), None).await.unwrap(), path);
    }

    #[tokio::test]
    async fn explicit_binary_and_private_cache_require_no_network() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("explicit");
        fs::write(&explicit, b"fixture").unwrap();
        assert_eq!(binary(dir.path(), Some(&explicit)).await.unwrap(), explicit.canonicalize().unwrap());
        assert!(binary(dir.path(), Some(Path::new("opencode")))
            .await
            .is_err());

        let Ok((arch, _)) = artifact() else { return };
        let cache = dir.path().join(format!("distribution-{VERSION}-{arch}"));
        factory_platform::private_dir(&cache).unwrap();
        let target = cache.join(if cfg!(windows) { "opencode.exe" } else { "opencode" });
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&explicit, &target).unwrap();
            assert!(binary(dir.path(), None).await.is_err());
            fs::remove_file(&target).unwrap();
        }
        fs::write(&target, b"cached fixture").unwrap();
        factory_platform::set_private_executable(&target).unwrap();
        assert_eq!(binary(dir.path(), None).await.unwrap(), target);
    }
}
