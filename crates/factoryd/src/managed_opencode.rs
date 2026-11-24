//! The model gateway's central OpenCode lifecycle belongs to factoryd.
use anyhow::{ensure, Context, Result};
use factory_core::RequestId;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::TcpListener,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

const VERSION: &str = "2.4.1";
const CONFIG_FILES: &[&str] = &[
    "opencode.json",
    "opencode.jsonc",
    ".opencode.json",
    ".opencode",
];

fn private_dir(path: &Path) -> Result<PathBuf> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0,
        "Factory OpenCode directory must be private: {}",
        path.display()
    );
    Ok(path.canonicalize()?)
}

fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn secret() -> String {
    format!("{}{}", RequestId::new(), RequestId::new())
}

fn model_key(path: &Path) -> Result<String> {
    if absent(path)? {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().context("Missing state directory")?)?;
        writeln!(file, "{}", secret())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path)?;
        File::open(path.parent().expect("validated parent"))?.sync_all()?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o077 == 0 && metadata.len() <= 130,
        "Factory model key must be a private regular file: {}",
        path.display()
    );
    Ok(fs::read_to_string(path)?
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}

pub struct Runtime {
    _lock: File,
    binary: PathBuf,
    workspace: PathBuf,
    environment: Vec<(&'static str, PathBuf)>,
    password: String,
    port: u16,
    child: Option<Child>,
}

impl Runtime {
    pub async fn start(
        database: &Path,
        binary: PathBuf,
    ) -> Result<(Self, factoryd::model_gateway::Gateway)> {
        let binary = resolve_binary(&binary)?;
        let name = database
            .file_name()
            .context("Factory database needs a filename")?;
        let mut directory = name.to_os_string();
        directory.push(".opencode");
        let root = private_dir(&database.with_file_name(directory))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(root.join("runtime.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("This Factory OpenCode state is already in use")?;
        let workspace = private_dir(&root.join("workspace"))?;
        ensure!(absent(Path::new("/etc/opencode"))?, "External OpenCode config /etc/opencode is visible; run factoryd in an isolated environment");
        for parent in workspace.ancestors() {
            for file in CONFIG_FILES {
                ensure!(
                    absent(&parent.join(file))?,
                    "External OpenCode configuration is visible at {}",
                    parent.join(file).display()
                );
            }
        }
        let environment = [
            ("HOME", "home"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
            ("TMPDIR", "tmp"),
        ]
        .into_iter()
        .map(|(name, dir)| Ok((name, private_dir(&root.join(dir))?)))
        .collect::<Result<Vec<_>>>()?;
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let mut runtime = Self {
            _lock: lock,
            binary,
            workspace,
            environment,
            password: secret(),
            port,
            child: None,
        };
        let version = tokio::time::timeout(
            Duration::from_secs(5),
            runtime.command().arg("--version").output(),
        )
        .await??;
        ensure!(
            version.status.success()
                && std::str::from_utf8(&version.stdout)?.trim() == format!("opencode v{VERSION}"),
            "factoryd requires OpenCode {VERSION}; place it in PATH or set FACTORY_OPENCODE_BIN"
        );
        let key_file = root.join("model.key");
        let gateway = factoryd::model_gateway::Gateway::managed(
            runtime.url(),
            runtime.password.clone(),
            model_key(&key_file)?,
        )?;
        runtime.spawn().await?;
        println!(
            "Factory-owned OpenCode ready; runner model key file: {}",
            key_file.display()
        );
        Ok((runtime, gateway))
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .envs(self.environment.iter().map(|(key, path)| (*key, path)))
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("OPENCODE_PASSWORD", &self.password)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    async fn spawn(&mut self) -> Result<()> {
        let mut child = self
            .command()
            .args([
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &self.port.to_string(),
            ])
            .spawn()
            .context("Could not start Factory-owned OpenCode")?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(400))
            .build()?;
        let endpoint = format!("{}/api/info", self.url());
        let readiness = async {
            loop {
                ensure!(
                    child.try_wait()?.is_none(),
                    "Factory-owned OpenCode exited during startup"
                );
                if let Ok(response) = client
                    .get(&endpoint)
                    .basic_auth("opencode", Some(&self.password))
                    .send()
                    .await
                {
                    if response.status().is_success() {
                        let value: serde_json::Value = response.json().await?;
                        ensure!(
                            value["version"] == VERSION,
                            "Unexpected version of Factory-owned OpenCode"
                        );
                        return Ok(());
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(15), readiness)
            .await
            .context("Factory-owned OpenCode is not ready")??;
        self.child = Some(child);
        Ok(())
    }

    pub async fn supervise(&mut self) {
        loop {
            if let Some(child) = &mut self.child {
                // Keep the child in self while waiting, so cancelling this future
                // on Factory shutdown does not drop a still-running process early.
                let _ = child.wait().await;
                self.child = None;
                eprintln!("Factory-owned OpenCode exited; restarting");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Err(error) = self.spawn().await {
                eprintln!("Factory-owned OpenCode restart failed: {error}");
            }
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
    }
}

fn resolve_binary(binary: &Path) -> Result<PathBuf> {
    if binary.components().count() > 1 {
        return binary
            .canonicalize()
            .context("Factory OpenCode binary not found");
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(binary))
        .find(|path| {
            fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .context("OpenCode is missing from PATH; install it or set FACTORY_OPENCODE_BIN")?
        .canonicalize()
        .context("Factory OpenCode binary not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an explicit OpenCode 2.4.1 binary"]
    async fn owns_and_restarts_only_its_opencode_and_keeps_its_state() {
        let binary = PathBuf::from(std::env::var_os("FACTORY_TEST_OPENCODE").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.sqlite3");
        let (mut runtime, _) = Runtime::start(&db, binary.clone()).await.unwrap();
        let key_path = dir.path().join("test.sqlite3.opencode/model.key");
        let first_key = fs::read(&key_path).unwrap();
        let url = runtime.url();
        let password = runtime.password.clone();
        assert!(Runtime::start(&db, binary.clone()).await.is_err());
        runtime.child.as_mut().unwrap().kill().await.unwrap();
        // Stop waiting once the supervised replacement accepts the same route.
        let verify = async {
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            loop {
                let result = client
                    .get(format!("{url}/api/credential"))
                    .basic_auth("opencode", Some(&password))
                    .send()
                    .await;
                if let Ok(response) = result.and_then(reqwest::Response::error_for_status) {
                    let credentials: serde_json::Value = response.json().await.unwrap();
                    assert_eq!(credentials["data"], serde_json::json!([]));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(12), verify) => result.unwrap(),
            _ = runtime.supervise() => panic!("supervisor exited"),
        };
        runtime.shutdown().await;
        drop(runtime);
        let (mut restarted, _) = Runtime::start(&db, binary).await.unwrap();
        assert_eq!(fs::read(&key_path).unwrap(), first_key);
        assert_eq!(
            fs::symlink_metadata(key_path).unwrap().permissions().mode() & 0o077,
            0
        );
        restarted.shutdown().await;
    }
}
