use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use factory_core::*;
use factory_protocol::client::FactoryClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub server: String,
    pub credential_file: Option<PathBuf>,
    pub project: Option<ProjectId>,
    pub page: crate::model::Page,
    pub desktop_notifications: bool,
    pub task_drafts: Vec<TaskDraft>,
    pub pending: Vec<PendingWrite>,
    pub question_drafts: Vec<QuestionDraft>,
    pub form_drafts: Vec<FormDraft>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct FormDraft {
    pub server: String,
    pub title: String,
    pub help: String,
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct QuestionDraft {
    pub server: String,
    pub question: QuestionId,
    pub revision: Revision,
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
    pub decision: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PendingWrite {
    pub id: u64,
    pub server: String,
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
    pub label: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_token: Option<RequestId>,
    pub server: String,
    pub project: ProjectId,
    pub ticket: Option<TicketId>,
    pub revision: Revision,
    pub prompt: String,
    pub images: Vec<PromptImage>,
    pub files: Vec<PromptFile>,
    pub requested_session: Option<CommandId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementer: Option<factory_protocol::models::ModelSelection>,
    #[serde(default)]
    pub model_loaded: bool,
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("de", "yoloforge", "YoloForge")
        .context("Kein Benutzerverzeichnis verfügbar")?;
    let path = dirs.data_local_dir().to_path_buf();
    fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
pub fn load_settings() -> Result<Settings> {
    let path = data_dir()?.join("desktop.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub fn desktop_client_id() -> Result<RequestId> {
    desktop_client_id_in(&data_dir()?)
}

/// Publish a fully written identity atomically, including simultaneous first
/// starts. A corrupt/unreadable identity is an error, never silently replaced.
fn desktop_client_id_in(directory: &Path) -> Result<RequestId> {
    let path = directory.join("desktop-client-id");
    let read = || -> Result<RequestId> {
        fs::read_to_string(&path)?
            .trim()
            .parse()
            .context("Gespeicherte Desktop-ID ist ungültig; Datei wurde nicht verändert")
    };
    match fs::read_to_string(&path) {
        Ok(id) => return id.trim().parse().context("Gespeicherte Desktop-ID ist ungültig; Datei wurde nicht verändert"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Desktop-ID konnte nicht gelesen werden"),
    }
    let id = RequestId::new();
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(id.to_string().as_bytes())?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => {
            fs::File::open(directory)?.sync_all()?;
            Ok(id)
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error).context("Desktop-ID konnte nicht dauerhaft gespeichert werden"),
    }
}
pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = data_dir()?.join("desktop.json");
    let temporary = path.with_extension(format!("{}.tmp", RequestId::new()));
    write_new(&temporary, &serde_json::to_vec_pretty(settings)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("Datei existiert bereits oder kann nicht angelegt werden")?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub async fn download_file(
    client: &FactoryClient,
    command: CommandId,
    hash: &str,
    name: &str,
) -> Result<String> {
    let index = client.archive_files(command, None).await?;
    let meta = index
        .files
        .iter()
        .find(|f| f.sha256 == hash)
        .context("Datei nicht mehr verfügbar")?;
    ensure!(
        meta.bytes <= ARCHIVE_BLOB_LIMIT,
        "Datei überschreitet Exportgrenze"
    );
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < meta.bytes {
        let chunk = client
            .archive_file_chunk(command, hash, bytes.len() as u64)
            .await?;
        ensure!(
            chunk.file == *meta && chunk.offset == bytes.len() as u64,
            "Dateiidentität verändert"
        );
        let data = chunk.bytes().map_err(anyhow::Error::msg)?;
        ensure!(
            bytes.len() as u64 + data.len() as u64 <= meta.bytes,
            "Ungültige Dateigröße"
        );
        bytes.extend(data);
    }
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == hash,
        "SHA256-Prüfung fehlgeschlagen"
    );
    let name = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Anhang")
        .to_string();
    tokio::task::spawn_blocking(move || {
        if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
            write_new(&path, &bytes)?;
            Ok(path.display().to_string())
        } else {
            Ok("Export abgebrochen".into())
        }
    })
    .await?
}

pub fn image_bytes(image: &PromptImage) -> Vec<u8> {
    STANDARD.decode(&image.data).unwrap_or_default()
}

pub struct LocalConnection {
    pub server: String,
    pub token: String,
    pub credential_file: PathBuf,
}

/// Dedicated user-owned processes. Dropping the GUI never signals these children.
pub async fn start_local() -> Result<LocalConnection> {
    let root = data_dir()?.join("local");
    fs::create_dir_all(&root)?;
    let token_path = root.join("token");
    if !token_path.exists() {
        write_new(
            &token_path,
            format!("{}{}", RequestId::new(), RequestId::new()).as_bytes(),
        )?;
    }
    let token = fs::read_to_string(&token_path)?;
    let server = "http://127.0.0.1:17433".to_string();
    let client = FactoryClient::new_desktop(&server, &token, desktop_client_id()?)?;
    let bin_dir = std::env::current_exe()?
        .parent()
        .context("Programmverzeichnis fehlt")?
        .to_path_buf();
    let start = |binary: &str, arguments: &[String]| -> Result<()> {
        let executable = bin_dir.join(binary);
        ensure!(
            executable.is_file(),
            "{binary} fehlt neben der GUI. Server und Runner zusammen mit der GUI installieren."
        );
        let log = OpenOptions::new()
            .append(true)
            .create(true)
            .open(root.join(format!("{binary}.log")))?;
        let mut command = std::process::Command::new(executable);
        command
            .args(arguments)
            .current_dir(&root)
            .env_remove("FACTORY_TOKEN")
            .env("FACTORY_TOKEN_FILE", &token_path)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        // Reap eventual exits without tying the process to the UI's lifetime.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    };
    match client.snapshot().await {
        Ok(_) => {},
        Err(factory_protocol::client::ClientError::Transport) => {
            start("factoryd", &["--listen".into(), "127.0.0.1:17433".into(), "--database".into(), root.join("factory.sqlite3").display().to_string()])?;
            let mut ready = false;
            for _ in 0..30 { if client.snapshot().await.is_ok() { ready = true; break; } tokio::time::sleep(std::time::Duration::from_millis(300)).await; }
            ensure!(ready, "Lokaler Server startet nicht. Diagnose: {}", root.join("factoryd.log").display());
        }
        Err(_) => anyhow::bail!("Port 17433 gehört nicht zur gespeicherten lokalen Factory oder der Zugang wurde widerrufen. Keine fremden Prozesse verändert."),
    }
    client.desktop_snapshot().await.map_err(|error| anyhow::anyhow!(
        "Lokaler Server auf Port 17433 läuft, ist aber nicht mit dieser GUI kompatibel oder nicht erreichbar ({error}). Eine ältere laufende factoryd-Version zuerst gezielt neu starten; es werden keine fremden oder laufenden Prozesse automatisch beendet."
    ))?;
    let runner_path = root.join("runner-id");
    if !runner_path.exists() {
        write_new(&runner_path, RunnerId::new().to_string().as_bytes())?;
    }
    let runner: RunnerId = fs::read_to_string(runner_path)?.parse()?;
    let snapshot = client.snapshot().await?;
    if !snapshot
        .runners
        .iter()
        .any(|r| r.id == runner && r.connection != ConnectionState::Unreachable)
    {
        start(
            "factory-runner",
            &[
                "--server".into(),
                server.clone(),
                "--id".into(),
                runner.to_string(),
                "--name".into(),
                "Lokaler Desktop-Runner".into(),
                "--journal".into(),
                root.join("runner.sqlite3").display().to_string(),
            ],
        )?;
    }
    Ok(LocalConnection {
        server,
        token,
        credential_file: token_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_identity_survives_restart_and_simultaneous_first_starts() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8).map(|_| {
            let directory = root.path().to_path_buf();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                desktop_client_id_in(&directory).unwrap()
            })
        }).collect();
        let ids: Vec<_> = threads.into_iter().map(|thread| thread.join().unwrap()).collect();
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_eq!(desktop_client_id_in(root.path()).unwrap(), ids[0]);
        let path = root.path().join("desktop-client-id");
        assert_eq!(fs::read_to_string(&path).unwrap(), ids[0].to_string());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let other = tempfile::tempdir().unwrap();
        assert_ne!(desktop_client_id_in(other.path()).unwrap(), ids[0]);
    }

    #[test]
    fn corrupt_desktop_identity_is_not_replaced_with_another_desktop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("desktop-client-id");
        fs::write(&path, "invalid identity").unwrap();
        assert!(desktop_client_id_in(root.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "invalid identity");
    }

    #[test]
    fn private_export_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        write_new(&path, b"original").unwrap();
        assert!(write_new(&path, b"overwrite").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
