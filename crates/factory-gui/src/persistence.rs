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
    pub language: crate::i18n::Language,
    pub task_keep_open: bool,
    pub task_as_draft: bool,
    pub cached_projects: Vec<CachedProjects>,
    pub offline_tasks: Vec<OfflineTask>,
    pub session_prompts: Vec<SessionPromptDraft>,
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub archived_orchestrator_failures: std::collections::BTreeSet<CommandId>,
    pub server: String,
    pub credential_file: Option<PathBuf>,
    pub project: Option<ProjectId>,
    #[serde(deserialize_with = "desktop_page")]
    pub page: crate::model::Page,
    pub desktop_notifications: bool,
    pub task_drafts: Vec<TaskDraft>,
    pub pending: Vec<PendingWrite>,
    pub question_drafts: Vec<QuestionDraft>,
    pub form_drafts: Vec<FormDraft>,
}
fn desktop_page<'de, D: serde::Deserializer<'de>>(d: D) -> Result<crate::model::Page, D::Error> {
    use crate::model::Page;
    Ok(match Page::deserialize(d)? {
        Page::Orchestrator | Page::Reviewer | Page::Merger => Page::Tasks,
        Page::Supervisor | Page::Memorizer => Page::ProjectActivity,
        page => page,
    })
}

#[cfg(test)]
#[test]
fn retired_sidebar_pages_restore_to_their_replacements() {
    use crate::model::Page;
    for (legacy, page) in [
        ("Orchestrator", Page::Tasks),
        ("Reviewer", Page::Tasks),
        ("Merger", Page::Tasks),
        ("Supervisor", Page::ProjectActivity),
        ("Memorizer", Page::ProjectActivity),
        ("Directs", Page::Sessions),
        ("Sessions", Page::Sessions),
    ] {
        let settings: Settings = serde_json::from_value(serde_json::json!({"page": legacy})).unwrap();
        assert_eq!(settings.page, page);
    }
}
impl Settings {
    pub fn startup_server(&self, requested: String) -> String {
        if requested.is_empty() {
            self.server.clone()
        } else {
            requested
        }
    }

    pub fn credential_for(&self, server: &str) -> Option<&PathBuf> {
        if self.server.trim_end_matches('/') == server.trim_end_matches('/') {
            self.credential_file.as_ref()
        } else {
            None
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CachedProjects {
    pub server: String,
    pub projects: Vec<Project>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct OfflineTask {
    pub id: RequestId,
    pub draft: TaskDraft,
    pub project_name: String,
    #[serde(default)]
    pub edit: Option<TaskDraft>,
    #[serde(default)]
    pub sync_request: Option<u64>,
    #[serde(default)]
    pub sync_error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionPromptDraft {
    pub id: RequestId,
    pub draft: TaskDraft,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_mcps: Option<Vec<String>>,
    #[serde(default)]
    pub model_loaded: bool,
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("de", "yoloforge", "YoloForge")
        .context(crate::tr!("Kein Benutzerverzeichnis verfügbar"))?;
    let path = dirs.data_local_dir().to_path_buf();
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    factory_platform::private_dir(&path)?;
    Ok(path)
}
pub fn load_settings() -> Result<Settings> {
    let path = data_dir()?.join("desktop.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// Only a confirmed connection updates this file. Independent GUI instances
/// can save window state and drafts without rolling back the last connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RememberedFactory {
    pub server: String,
    pub credential_file: PathBuf,
}

#[cfg(test)]
#[test]
fn remembered_connection_survives_unrelated_settings_writes_and_bad_desktop_json() {
    let directory = tempfile::tempdir().unwrap();
    let credential = save_connection_token_in(&directory.path().join("credentials"), "https://factory.example", "test-secret").unwrap();
    let profile = RememberedFactory { server: "https://factory.example".into(), credential_file: credential };
    write_remembered_factory(directory.path(), &profile).unwrap();
    let desktop = directory.path().join("desktop.json");
    for content in [b"{\"server\":\"https://old.example\"}".as_slice(), b"invalid json".as_slice()] {
        fs::write(&desktop, content).unwrap();
        let mut settings: Settings = serde_json::from_slice(&fs::read(&desktop).unwrap()).unwrap_or_default();
        settings.apply_remembered_factory(read_remembered_factory(directory.path()).unwrap().unwrap());
        let url = settings.startup_server(String::new());
        assert_eq!(url, profile.server);
        assert_eq!(settings.credential_for(&url), Some(&profile.credential_file));
        assert!(settings.credential_for("https://old.example").is_none());
    }
    assert!(!fs::read_to_string(directory.path().join("last-factory.json")).unwrap().contains("test-secret"));
}

pub(crate) fn load_remembered_factory() -> Result<Option<RememberedFactory>> {
    read_remembered_factory(&data_dir()?)
}

fn read_remembered_factory(directory: &Path) -> Result<Option<RememberedFactory>> {
    match fs::read(directory.join("last-factory.json")) {
        Ok(data) => Ok(Some(serde_json::from_slice(&data)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn save_remembered_factory(connection: &RememberedFactory) -> Result<()> {
    write_remembered_factory(&data_dir()?, connection)
}

fn write_remembered_factory(directory: &Path, connection: &RememberedFactory) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    factory_platform::private_file(temporary.path())?;
    serde_json::to_writer(temporary.as_file_mut(), connection)?;
    temporary.as_file().sync_all()?;
    temporary.persist(directory.join("last-factory.json"))?;
    factory_platform::sync_dir(directory)
}

impl Settings {
    pub(crate) fn apply_remembered_factory(&mut self, connection: RememberedFactory) {
        self.server = connection.server;
        self.credential_file = Some(connection.credential_file);
    }
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
            .context(crate::tr!("Gespeicherte Desktop-ID ist ungültig; Datei wurde nicht verändert"))
    };
    match fs::read_to_string(&path) {
        Ok(id) => return id.trim().parse().context(crate::tr!("Gespeicherte Desktop-ID ist ungültig; Datei wurde nicht verändert")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context(crate::tr!("Desktop-ID konnte nicht gelesen werden")),
    }
    let id = RequestId::new();
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(id.to_string().as_bytes())?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => {
            factory_platform::sync_dir(directory)?;
            Ok(id)
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error).context(crate::tr!("Desktop-ID konnte nicht dauerhaft gespeichert werden")),
    }
}
pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = data_dir()?.join("desktop.json");
    let temporary = path.with_extension(format!("{}.tmp", RequestId::new()));
    write_new(&temporary, &serde_json::to_vec_pretty(settings)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Credentials are separate from desktop drafts and readable only by their owner.
pub fn save_connection_token(server: &str, token: &str) -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("de", "yoloforge", "YoloForge")
        .context(crate::tr!("Kein Benutzerverzeichnis verfügbar"))?;
    save_connection_token_in(dirs.config_dir(), server, token)
}

fn save_connection_token_in(directory: &Path, server: &str, token: &str) -> Result<PathBuf> {
    if let Some(parent) = directory.parent() { fs::create_dir_all(parent)?; }
    factory_platform::private_dir(directory)?;
    let path = directory.join(format!("connection-{:x}.token", Sha256::digest(server.as_bytes())));
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    factory_platform::private_file(temporary.path())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary.as_file().set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(token.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).context(crate::tr!("Zugangstoken konnte nicht gespeichert werden"))?;
    factory_platform::sync_dir(directory)?;
    Ok(path)
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
        .context(crate::tr!("Datei existiert bereits oder kann nicht angelegt werden"))?;
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
        .context(crate::tr!("Datei nicht mehr verfügbar"))?;
    ensure!(
        meta.bytes <= ARCHIVE_BLOB_LIMIT,
        crate::tr!("Datei überschreitet Exportgrenze")
    );
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < meta.bytes {
        let chunk = client
            .archive_file_chunk(command, hash, bytes.len() as u64)
            .await?;
        ensure!(
            chunk.file == *meta && chunk.offset == bytes.len() as u64,
            crate::tr!("Dateiidentität verändert")
        );
        let data = chunk.bytes().map_err(anyhow::Error::msg)?;
        ensure!(
            bytes.len() as u64 + data.len() as u64 <= meta.bytes,
            crate::tr!("Ungültige Dateigröße")
        );
        bytes.extend(data);
    }
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == hash,
        crate::tr!("SHA256-Prüfung fehlgeschlagen")
    );
    let name = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(crate::tr!("Anhang"))
        .to_string();
    tokio::task::spawn_blocking(move || {
        if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
            write_new(&path, &bytes)?;
            Ok(path.display().to_string())
        } else {
            Ok(crate::tr!("Export abgebrochen").into())
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

pub const LOCAL_FACTORY_URL: &str = "http://127.0.0.1:17433";
include!(concat!(env!("OUT_DIR"), "/factory_runtime.rs"));

fn unpack_runtime(runtime: &[(&str, &[u8])], cache: &Path) -> Result<PathBuf> {
    let entries: Vec<_> = runtime.iter().map(|(name, payload)| (*name, *payload, Sha256::digest(payload))).collect();
    let mut fingerprint = Sha256::new();
    for (_, _, digest) in &entries { fingerprint.update(digest); }
    factory_platform::private_dir(cache)?;
    let destination = cache.join(format!("{:x}", fingerprint.finalize()));
    factory_platform::private_dir(&destination)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(destination.join("extract.lock"))?;
    lock.lock()?;
    for (name, payload, digest) in entries {
        let size = payload.len() as u64;
        let path = destination.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if factory_platform::is_executable(&path) && factory_platform::ensure_private(&path).is_ok() {
            let mut existing = fs::File::open(&path)?;
            let mut hash = Sha256::new();
            if existing.metadata()?.len() == size && std::io::copy(&mut existing, &mut hash)? == size
                && hash.finalize() == digest
            {
                continue;
            }
        }
        let mut file = tempfile::NamedTempFile::new_in(&destination)?;
        file.write_all(payload)?;
        file.as_file().sync_all()?;
        factory_platform::set_private_executable(file.path())?;
        file.persist(&path)?;
    }
    Ok(destination)
}

pub(crate) fn verify_local_runtime(cache: &Path) -> Result<()> {
    ensure!(EMBEDDED_RUNTIME.len() == 3, "Keine eingebettete Factory-Laufzeit");
    let directory = unpack_runtime(EMBEDDED_RUNTIME, cache)?;
    for (name, _) in EMBEDDED_RUNTIME {
        ensure!(std::process::Command::new(directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
            .arg("--version").status()?.success(), "{name} --version fehlgeschlagen");
    }
    Ok(())
}

pub(crate) fn companion_binary(name: &str) -> Result<PathBuf> {
    static RUNTIME: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
    let gui = std::env::current_exe()?;
    if !EMBEDDED_RUNTIME.is_empty() {
        let mut cached = RUNTIME.lock().unwrap_or_else(|p| p.into_inner());
        if cached.is_none() {
            *cached = Some(unpack_runtime(EMBEDDED_RUNTIME, &data_dir()?.join("runtime"))?);
        }
        let directory = cached.as_ref().expect("initialized runtime path");
        return Ok(directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)));
    }
    // Unpackaged Cargo development builds may use binaries from their target directory.
    let directory = gui.parent().context(crate::tr!("Programmverzeichnis fehlt"))?;
    Ok(directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

/// Release packages use their included runtime. Cargo development builds can
/// still fall back to the existing explicit override or managed download.
pub(crate) fn configure_bundled_opencode(command: &mut std::process::Command) -> Result<()> {
    let binary = companion_binary("opencode")?;
    if binary.is_file() {
        command.env("FACTORY_OPENCODE_BIN", binary);
    }
    Ok(())
}

type OwnedChildren = std::collections::BTreeMap<String, std::process::Child>;
static LOCAL_CHILDREN: std::sync::Mutex<Option<OwnedChildren>> =
    std::sync::Mutex::new(Some(std::collections::BTreeMap::new()));

pub(crate) struct LocalServicesGuard;
impl Drop for LocalServicesGuard {
    fn drop(&mut self) {
        let children = LOCAL_CHILDREN.lock().unwrap_or_else(|p| p.into_inner()).take();
        stop_owned(children.unwrap_or_default());
    }
}

fn stop_owned(mut children: OwnedChildren) {
    for child in children.values_mut() { drop(child.stdin.take()); }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !children.is_empty() && std::time::Instant::now() < deadline {
        children.retain(|_, child| !matches!(child.try_wait(), Ok(Some(_))));
        if !children.is_empty() { std::thread::sleep(std::time::Duration::from_millis(25)); }
    }
    for mut child in children.into_values() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn spawn_owned(name: String, command: &mut std::process::Command) -> Result<()> {
    let mut local = LOCAL_CHILDREN.lock().unwrap_or_else(|p| p.into_inner());
    let children = local.as_mut().context("YoloForge wird beendet")?;
    if name != "factoryd" {
        ensure!(children.contains_key("factoryd"), "Lokale Factory zuerst in dieser GUI starten");
    }
    if let Some(child) = children.get_mut(&name) {
        ensure!(child.try_wait()?.is_some(), "Lokaler Dienst wird bereits von dieser GUI verwaltet");
    }
    let child = command.stdin(std::process::Stdio::piped())
        .env(factory_platform::GUI_LIFETIME_PIPE, "1").spawn()?;
    children.insert(name, child);
    Ok(())
}

pub(crate) fn owns_local_runner(id: RunnerId, pid: u32) -> bool {
    LOCAL_CHILDREN.lock().unwrap_or_else(|p| p.into_inner()).as_mut()
        .and_then(|children| children.get_mut(&id.to_string()))
        .is_some_and(|child| child.id() == pid && matches!(child.try_wait(), Ok(None)))
}

struct LocalStartup(bool);
impl Drop for LocalStartup {
    fn drop(&mut self) {
        if self.0 {
            let children = LOCAL_CHILDREN.lock().unwrap_or_else(|p| p.into_inner())
                .as_mut().map(std::mem::take).unwrap_or_default();
            stop_owned(children);
        }
    }
}

pub async fn start_local() -> Result<LocalConnection> {
    let root = data_dir()?.join("local");
    // Do not change permissions of an existing installation's state directory.
    if !root.exists() {
        factory_platform::private_dir(&root)?;
    }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(root.join("gui-start.lock"))?;
    lock.try_lock().context("Die lokale Factory wird bereits gestartet")?;
    let owned = {
        let mut local = LOCAL_CHILDREN.lock().unwrap_or_else(|p| p.into_inner());
        match local.as_mut().and_then(|children| children.get_mut("factoryd")) {
            Some(child) => child.try_wait()?.is_none(),
            None => false,
        }
    };
    let token_path = root.join("token");
    if !token_path.exists() {
        write_new(
            &token_path,
            format!("{}{}", RequestId::new(), RequestId::new()).as_bytes(),
        )?;
        factory_platform::private_file(&token_path)?;
    }
    let token = fs::read_to_string(&token_path)?;
    let server = LOCAL_FACTORY_URL.to_string();
    let client = FactoryClient::new_desktop(&server, &token, desktop_client_id()?)?;
    let start = |binary: &str, arguments: &[String]| -> Result<()> {
        let executable = companion_binary(binary)?;
        ensure!(executable.is_file(), crate::tr!("Eingebettete Factory-Laufzeit fehlt: {}", binary));
        let log = OpenOptions::new()
            .append(true)
            .create(true)
            .open(root.join(format!("{binary}.log")))?;
        let mut command = std::process::Command::new(executable);
        configure_bundled_opencode(&mut command)?;
        command
            .args(arguments)
            .current_dir(&root)
            .env_remove("FACTORY_TOKEN")
            .env("FACTORY_TOKEN_FILE", &token_path)
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        }
        spawn_owned(binary.into(), &mut command)
    };
    let mut startup = LocalStartup(false);
    if !owned {
        // Do not adopt a standalone server or leave it running on GUI exit.
        std::net::TcpListener::bind(("127.0.0.1", 17433)).context(
            crate::tr!("Port 17433 ist bereits belegt. Eine ältere lokale Factory bitte gezielt beenden; fremde Prozesse werden nicht verändert."))?;
        start("factoryd", &["--listen".into(), "127.0.0.1:17433".into(), "--database".into(), root.join("factory.sqlite3").display().to_string()])?;
        startup.0 = true;
        let mut ready = false;
        for _ in 0..30 {
            if client.snapshot().await.is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        ensure!(ready, crate::tr!("Lokaler Server startet nicht. Diagnose: {}", root.join("factoryd.log").display()));
    }
    client.desktop_snapshot().await.map_err(|error| anyhow::anyhow!("{}", crate::tr!("Lokaler Server auf Port 17433 läuft, ist aber nicht mit dieser GUI kompatibel oder nicht erreichbar ({error}). Eine ältere laufende factoryd-Version zuerst gezielt neu starten; es werden keine fremden oder laufenden Prozesse automatisch beendet.", error = error)))?;
    crate::local_runners::ensure_builtin(&client, &server, &root).await?;
    startup.0 = false;
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
    fn embedded_runtime_extracts_privately_and_repairs_damaged_cache() {
        let root = tempfile::tempdir().unwrap();
        let runtime: &[(&str, &[u8])] = &[("factoryd", b"factory fixture"), ("opencode", b"open fixture")];
        let cache = root.path().join("runtime");
        let directory = unpack_runtime(runtime, &cache).unwrap();
        let server = directory.join(format!("factoryd{}", std::env::consts::EXE_SUFFIX));
        factory_platform::ensure_private(&server).unwrap();
        assert_eq!(fs::read(&server).unwrap(), b"factory fixture");
        fs::write(&server, b"bad runtime!!!!").unwrap();
        assert_eq!(unpack_runtime(runtime, &cache).unwrap(), directory);
        assert_eq!(fs::read(&server).unwrap(), b"factory fixture");
    }
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
    fn saved_connection_tokens_are_private_replaceable_and_server_scoped() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("yoloforge");
        let server = "https://factory.example";
        let path = save_connection_token_in(&directory, server, "first-secret").unwrap();
        let settings = Settings {
            server: server.into(),
            credential_file: Some(path.clone()),
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("first-secret"));
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.server, server);
        assert_eq!(fs::read_to_string(restored.credential_file.unwrap()).unwrap(), "first-secret");
        assert_eq!(save_connection_token_in(&directory, server, "new-secret").unwrap(), path);
        let other = save_connection_token_in(&directory, "https://other.example", "other-secret").unwrap();
        assert_ne!(path, other);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new-secret");
        assert_eq!(fs::read_to_string(&other).unwrap(), "other-secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&directory).unwrap().permissions().mode() & 0o777, 0o700);
            for file in [&path, &other] {
                assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), 2);
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
