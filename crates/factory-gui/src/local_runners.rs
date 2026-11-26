//! Durable, owner-verified desktop runner processes. Closing the GUI never stops them.
use anyhow::{bail, ensure, Context, Result};
use factory_core::*;
use factory_protocol::{client::FactoryClient, *};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalRunner {
    pub id: RunnerId,
    pub server: String,
    pub name: String,
    #[serde(default)]
    pub removing: bool,
    #[serde(default)]
    pub removed: bool,
    /// Explicit restart intent survives a GUI crash without changing identity.
    #[serde(default)]
    pub restarting: Option<RequestId>,
    device: Option<DeviceId>,
    /// Older local-Factory installations used this journal and token.
    #[serde(default)]
    legacy: bool,
    #[serde(default)]
    builtin: bool,
}

impl LocalRunner {
    pub fn visible(&self, server: &str) -> bool {
        self.server == server && !self.removed
    }
    fn root(&self) -> Result<PathBuf> {
        Ok(if self.legacy {
            crate::persistence::data_dir()?.join("local")
        } else {
            crate::persistence::data_dir()?
                .join("runners")
                .join(self.id.to_string())
        })
    }
}

pub enum Action {
    Add,
    Restore,
    Remove(RunnerId),
    Restart { id: RunnerId, recovery_start: bool },
}
pub struct Report {
    pub runners: Option<Vec<LocalRunner>>,
    pub result: Result<Option<String>, String>,
    pub user_error: Option<String>,
}

fn config_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("de", "yoloforge", "YoloForge")
        .context(crate::tr!("Kein Benutzerverzeichnis verfügbar"))?;
    let dir = dirs.config_dir().to_path_buf();
    private_dir(&dir)?;
    Ok(dir)
}
fn private_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    factory_platform::private_dir(path)
}
fn read_catalog(dir: &Path) -> Result<Vec<LocalRunner>> {
    match fs::read(dir.join("local-runners.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
pub fn load() -> Result<Vec<LocalRunner>> {
    read_catalog(&config_dir()?)
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let dir = path.parent().context(crate::tr!("Verzeichnis fehlt"))?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    factory_platform::sync_dir(dir)?;
    Ok(())
}
fn lock(dir: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(dir.join("local-runners.lock"))?;
    file.try_lock().context(crate::tr!(
        "Lokale Runner werden bereits von einer anderen GUI verwaltet"
    ))?;
    Ok(file)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Process {
    pid: u32,
    boot: String,
    started: String,
    executable: PathBuf,
}
#[cfg(target_os = "linux")]
fn process(pid: u32) -> Result<Process> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    let stat = fs::read_to_string(root.join("stat"))?;
    Ok(Process {
        pid,
        boot: fs::read_to_string("/proc/sys/kernel/random/boot_id")?,
        started: start_ticks(&stat)?.into(),
        executable: {
            let path = fs::read_link(root.join("exe"))?;
            // A replaced on-disk binary does not change ownership of a surviving child.
            path.to_str()
                .and_then(|s| s.strip_suffix(" (deleted)"))
                .map(PathBuf::from)
                .unwrap_or(path)
        },
    })
}
#[cfg(target_os = "linux")]
fn start_ticks(stat: &str) -> Result<&str> {
    // comm may contain spaces and parentheses. Field 22 is starttime.
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .context(crate::tr!("Prozessidentität ist unvollständig"))
}
#[cfg(any(target_os = "linux", test))]
fn arguments_match(arguments: &[String], runner: &LocalRunner, journal: &Path) -> bool {
    [
        ("--server", runner.server.clone()),
        ("--id", runner.id.to_string()),
        ("--journal", journal.display().to_string()),
    ]
    .iter()
    .all(|(key, value)| {
        arguments
            .windows(2)
            .any(|pair| pair[0] == *key && pair[1] == *value)
    })
}
#[cfg(target_os = "linux")]
fn owns_process(identity: &Process, runner: &LocalRunner, journal: &Path) -> bool {
    if process(identity.pid).ok().as_ref() != Some(identity) {
        return false;
    }
    let Ok(bytes) = fs::read(format!("/proc/{}/cmdline", identity.pid)) else {
        return false;
    };
    let args: Vec<_> = bytes
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect();
    arguments_match(&args, runner, journal)
}
fn runner_executable() -> Result<PathBuf> {
    let path = crate::persistence::companion_binary("factory-runner")?;
    ensure!(
        path.is_file(),
        crate::tr!("factory-runner fehlt neben der GUI")
    );
    Ok(fs::canonicalize(path)?)
}
fn find_process(runner: &LocalRunner, root: &Path) -> Result<Option<Process>> {
    if let Some(endpoint) = control_endpoint(runner, root)? {
        if endpoint.request("ping").is_ok() {
            return Ok(Some(Process {
                pid: endpoint.pid,
                boot: "control".into(),
                started: endpoint.secret,
                executable: runner_executable()?,
            }));
        }
    }
    #[cfg(target_os = "linux")]
    return legacy_find_process(runner, root);
    #[cfg(not(target_os = "linux"))]
    Ok(None)
}

fn control_endpoint(
    runner: &LocalRunner,
    root: &Path,
) -> Result<Option<factory_platform::runner_control::Endpoint>> {
    let path = root.join("runner-control.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    factory_platform::ensure_private(&path)?;
    let endpoint: factory_platform::runner_control::Endpoint = serde_json::from_slice(&bytes)?;
    ensure!(
        endpoint.runner == runner.id.to_string()
            && endpoint.server == runner.server
            && endpoint.journal == root.join("runner.sqlite3"),
        crate::tr!("Prozessidentität hat sich geändert; kein Prozess beendet")
    );
    Ok(Some(endpoint))
}

#[cfg(target_os = "linux")]
fn legacy_find_process(runner: &LocalRunner, root: &Path) -> Result<Option<Process>> {
    let journal = root.join("runner.sqlite3");
    let record = root.join("runner-process.json");
    if record.exists() {
        let identity: Process = serde_json::from_slice(&fs::read(record)?)?;
        if owns_process(&identity, runner, &journal) {
            return Ok(Some(identity));
        }
    }
    // Also adopt surviving children after a crash between spawn and record write,
    // and the original built-in local runner from earlier GUI versions.
    let executable = runner_executable()?;
    let mut found = None;
    for entry in fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(identity) = process(pid) else {
            continue;
        };
        if identity.executable == executable && owns_process(&identity, runner, &journal) {
            ensure!(
                found.is_none(),
                crate::tr!("Mehrere Prozesse für denselben lokalen Runner gefunden; keine Prozesse verändert")
            );
            found = Some(identity);
        }
    }
    if let Some(identity) = &found {
        atomic_json(&root.join("runner-process.json"), identity)?;
    }
    Ok(found)
}

fn start(runner: &LocalRunner) -> Result<()> {
    start_instance(runner, None)
}

fn start_instance(runner: &LocalRunner, instance: Option<InstanceId>) -> Result<()> {
    let root = runner.root()?;
    private_dir(&root)?;
    let owned_factory = runner.server == crate::persistence::LOCAL_FACTORY_URL;
    if let Some(process) = find_process(runner, &root)? {
        ensure!(!owned_factory || crate::persistence::owns_local_runner(runner.id, process.pid),
            "Älterer lokaler Runner läuft noch. Bitte gezielt beenden; fremde Prozesse werden nicht verändert.");
        ensure!(instance.is_none(), crate::tr!("Runner wurde noch nicht vollständig beendet"));
        return Ok(());
    }
    ensure!(
        root.join("runner.token").is_file(),
        crate::tr!("Gespeicherter Runner-Zugang fehlt")
    );
    let managed = root.join("managed-opencode.json");
    if !managed.exists() {
        // Keep this endpoint stable across runner/GUI restarts for journal recovery.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        atomic_json(
            &managed,
            &serde_json::json!({
                "port": port, "model_key_file": root.join("runner.token"),
            }),
        )?;
    }
    let mut options = fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options.open(root.join("factory-runner.log"))?;
    let mut command = Command::new(runner_executable()?);
    crate::persistence::configure_bundled_opencode(&mut command)?;
    if let Some(instance) = instance {
        command.args(["--instance-id", &instance.to_string()]);
    }
    command
        .args([
            "--server",
            &runner.server,
            "--id",
            &runner.id.to_string(),
            "--name",
            &runner.name,
            "--journal",
            &root.join("runner.sqlite3").display().to_string(),
            "--managed-opencode",
            &managed.display().to_string(),
            "--control-file",
            &root.join("runner-control.json").display().to_string(),
        ])
        .current_dir(&root)
        .env_remove("FACTORY_TOKEN")
        .env("FACTORY_TOKEN_FILE", root.join("runner.token"))
        .stdin(Stdio::null())
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
    if owned_factory {
        return crate::persistence::spawn_owned(runner.id.to_string(), &mut command);
    }
    let mut child = command.env_remove(factory_platform::GUI_LIFETIME_PIPE).spawn()?;
    // Reap eventual exits but keep the runner independent of GUI lifetime.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

async fn start_and_confirm(client: &FactoryClient, runner: &LocalRunner) -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(identity) = find_process(runner, &runner.root()?)? {
        if identity.boot != "control" {
            let bytes = fs::read(format!("/proc/{}/cmdline", identity.pid))?;
            let managed = bytes
                .split(|b| *b == 0)
                .any(|arg| arg == b"--managed-opencode");
            if !managed && runner.server != crate::persistence::LOCAL_FACTORY_URL {
                ensure!(!busy(&client.desktop_snapshot().await?, runner.id),
                crate::tr!("Runner hat laufende oder ungeklärte Aufträge; automatische OpenCode-Einrichtung wartet bis zum nächsten Verbinden."));
                stop(runner).await?;
            }
        }
    }
    start(runner)?;
    // A restarted runner may have to wait for its previous registration lease.
    for attempt in 0..300 {
        let root = runner.root()?;
        if attempt < 20 && find_process(runner, &root)?.is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        }
        ensure!(
            find_process(runner, &root)?.is_some(),
            "{}",
            startup_diagnostic(&root, &runner.name)
        );
        if client.desktop_snapshot().await?.runners.iter().any(|r| {
            r.id == runner.id
                && r.connection == ConnectionState::Online
                && matches!(r.opencode, OpenCodeStatus::Ready { .. })
        }) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    bail!("{}", crate::tr!("Runner läuft, aber die Factory hat die Anmeldung noch nicht bestätigt. Runner bleibt gespeichert.", ))
}

fn startup_diagnostic(root: &Path, name: &str) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let path = root.join("factory-runner.log");
    let tail = (|| -> Result<String> {
        let mut file = fs::File::open(&path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(16 * 1024)))?;
        let mut bytes = vec![];
        file.take(16 * 1024).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })();
    match tail {
        Ok(text) if !text.trim().is_empty() => crate::tr!(
            "{name} wurde beim Start beendet.\nProtokoll: {}\n\n{}",
            path.display(),
            text.trim(),
            name = name
        ),
        _ => crate::tr!(
            "{name} wurde beim Start beendet. Protokoll: {}",
            path.display(),
            name = name
        ),
    }
}

async fn stop(runner: &LocalRunner) -> Result<()> {
    let root = runner.root()?;
    if let Some(endpoint) = control_endpoint(runner, &root)? {
        if endpoint.request("ping").is_ok() {
            endpoint.request("stop")?;
            for _ in 0..50 {
                if endpoint.request("ping").is_err() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            bail!("{}", crate::tr!("Runner beendet sich noch nicht; keine erzwungene Beendigung. Erneut entfernen versuchen.", ))
        }
    }
    #[cfg(target_os = "linux")]
    return legacy_stop(runner).await;
    #[cfg(not(target_os = "linux"))]
    Ok(())
}

#[cfg(target_os = "linux")]
async fn legacy_stop(runner: &LocalRunner) -> Result<()> {
    let root = runner.root()?;
    let Some(identity) = find_process(runner, &root)? else {
        return Ok(());
    };
    // Pin the process before rechecking its identity. Never signal a recycled PID.
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error.into());
    }
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) };
    ensure!(
        owns_process(&identity, runner, &root.join("runner.sqlite3")),
        crate::tr!("Prozessidentität hat sich geändert; kein Prozess beendet")
    );
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    for _ in 0..50 {
        if !owns_process(&identity, runner, &root.join("runner.sqlite3")) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!("{}", crate::tr!("Runner beendet sich noch nicht; keine erzwungene Beendigung. Erneut entfernen versuchen.", ))
}

pub fn busy(snapshot: &Snapshot, id: RunnerId) -> bool {
    snapshot.commands.iter().any(|c| {
        c.runner_id == id && matches!(c.state, CommandState::Pending | CommandState::Accepted)
    }) || snapshot.streams.iter().any(|s| {
        s.runner_id == id && !matches!(s.status, StreamState::Idle | StreamState::Completed)
    })
}

pub fn restart_blocked(snapshot: &Snapshot, id: RunnerId) -> bool {
    snapshot.commands.iter().any(|c| c.runner_id == id
        && (c.state == CommandState::Accepted
            || match &c.result {
                Some(CommandOutput::Agent { agent }) => agent.phase == AgentPhase::Unknown,
                _ => c.agent_progress.as_ref().is_some_and(|p| p.phase == AgentPhase::Unknown),
            }))
        || snapshot.streams.iter().any(|s| s.runner_id == id
            && matches!(s.status, StreamState::Running | StreamState::WaitingInput | StreamState::Pausing | StreamState::Paused))
        || snapshot.controls.iter().any(|c| c.state != PauseState::Released
            && snapshot.streams.iter().any(|s| s.id == c.stream_id && s.runner_id == id))
        || snapshot.runtime_terminations.iter().any(|r| r.state != TerminationState::Stopped
            && snapshot.commands.iter().any(|c| c.id == r.command_id && c.runner_id == id))
}

pub fn recovery_start_available(snapshot: &Snapshot, id: RunnerId) -> bool {
    snapshot.runners.iter().any(|runner| runner.id == id && runner.connection == ConnectionState::Unreachable)
}

fn runner_journal_available(root: &Path) -> Result<bool> {
    if !root.join("runner.sqlite3").is_file() {
        return Ok(false);
    }
    let lock = match fs::OpenOptions::new().read(true).write(true).open(root.join("runner.lock")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if lock.try_lock().is_ok() {
        lock.unlock()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn wait_for_journal_release(root: &Path) -> Result<()> {
    let path = root.join("runner.lock");
    for _ in 0..50 {
        let lock = fs::OpenOptions::new().read(true).write(true).open(&path)?;
        if lock.try_lock().is_ok() {
            lock.unlock()?;
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!("{}", crate::tr!("Runner-Journal ist noch belegt; kein zweiter Runner gestartet"))
}

async fn restart(client: &FactoryClient, runner: &LocalRunner, recovery_start: bool) -> Result<()> {
    ensure!(!runner.removing, crate::tr!("Dieser Runner wird gerade entfernt"));
    let root = runner.root()?;
    // Validate all local prerequisites before stopping an existing process.
    runner_executable()?;
    ensure!(root.join("runner.token").is_file(), crate::tr!("Gespeicherter Runner-Zugang fehlt"));
    if recovery_start {
        client.runner_restart_status(runner.id).await?;
        // With a stopped process there is nothing to interrupt. Never use
        // this path to signal a process, even if the server calls it offline.
        ensure!(find_process(runner, &root)?.is_none() && runner_journal_available(&root)?,
            crate::tr!("Runner läuft lokal noch oder sein Journal ist nicht verfügbar. Kein Prozess wurde beendet."));
    } else {
        // Fence claims before stopping a process; a snapshot cannot do that.
        let request = runner.restarting.context(crate::tr!("Neustartauftrag fehlt"))?;
        client.hold_runner_restart(runner.id, request).await?;
        stop(runner).await?;
    }
    if root.join("runner.lock").exists() { wait_for_journal_release(&root).await?; }
    let instance = InstanceId::new();
    start_instance(runner, Some(instance))?;
    for attempt in 0..300 {
        if attempt >= 20 {
            ensure!(find_process(runner, &root)?.is_some(), "{}", startup_diagnostic(&root, &runner.name));
        }
        let status = client.runner_restart_status(runner.id).await?;
        // The prior registration can remain online until its lease expires.
        // Only this exact newly spawned instance confirms a successful restart.
        if restart_confirmed(&status, instance) {
            if let Some(request) = runner.restarting {
                client.release_runner_restart(runner.id, request).await?;
            }
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    if recovery_start {
        bail!("{}", crate::tr!("Runner-Start noch nicht bestätigt."));
    }
    bail!("{}", crate::tr!("Runner-Neustart noch nicht bestätigt. Neustartauftrag bleibt gespeichert."))
}

fn restart_confirmed(status: &RunnerRestartStatus, expected: InstanceId) -> bool {
    status.instance_id == Some(expected) && status.ready
}

async fn create(client: &FactoryClient, runner: &mut LocalRunner) -> Result<()> {
    let root = runner.root()?;
    private_dir(&root)?;
    let credential_path = root.join("credential.json");
    let credential: DeviceCredential = if credential_path.exists() {
        serde_json::from_slice(&fs::read(&credential_path)?)?
    } else {
        let invitation = client
            .enrollment(&CreateEnrollment {
                role: DeviceRole::Runner,
                runner_id: Some(runner.id),
                ttl_seconds: 60,
            })
            .await?;
        let credential = client
            .redeem(&RedeemEnrollment {
                code: invitation.code,
                name: runner.name.clone(),
            })
            .await?;
        // Save identity and secret together so a crash cannot lose the revocation ID.
        atomic_json(&credential_path, &credential)?;
        credential
    };
    if !root.join("runner.token").exists() {
        crate::persistence::write_new(&root.join("runner.token"), credential.token.as_bytes())?;
    }
    runner.device = Some(credential.device.id);
    Ok(())
}

async fn remove(
    client: &FactoryClient,
    runners: &mut [LocalRunner],
    index: usize,
    catalog: &Path,
) -> Result<()> {
    let snapshot = client.desktop_snapshot().await?;
    ensure!(
        !busy(&snapshot, runners[index].id),
        crate::tr!("Runner hat laufende oder ungeklärte Aufträge. Diese zuerst abschließen.")
    );
    // Persist removal intent before touching the process, so a GUI crash cannot
    // resurrect a runner whose removal was confirmed.
    runners[index].removing = true;
    atomic_json(catalog, &runners)?;
    stop(&runners[index]).await?;
    if let Some(device) = runners[index].device {
        client.revoke_device(device).await?;
    }
    runners[index].removed = true;
    runners[index].removing = false;
    atomic_json(catalog, &runners)?;
    Ok(())
}

pub async fn perform(client: &FactoryClient, server: &str, action: Action) -> Report {
    let starts_runner = matches!(&action, Action::Add | Action::Restore | Action::Restart { .. });
    let result = async {
        let dir = config_dir()?;
        let _lock = lock(&dir)?;
        let catalog = dir.join("local-runners.json");
        let mut runners = read_catalog(&dir)?;
        let message = match action {
            Action::Add => {
                let id = RunnerId::new();
                let mut runner = LocalRunner {
                    id,
                    server: server.into(),
                    name: crate::tr!(
                        "Lokaler Runner {}",
                        runners.iter().filter(|r| r.server == server).count() + 1
                    ),
                    removing: false,
                    removed: false,
                    restarting: None,
                    device: None,
                    legacy: false,
                    builtin: false,
                };
                // Reserve the identity durably before creating credentials or processes.
                runners.push(runner.clone());
                atomic_json(&catalog, &runners)?;
                create(client, &mut runner).await?;
                *runners.last_mut().unwrap() = runner.clone();
                atomic_json(&catalog, &runners)?;
                start_and_confirm(client, &runner).await?;
                Some(crate::tr!("Lokaler Runner gestartet.").into())
            }
            Action::Remove(id) => {
                let index = runners
                    .iter()
                    .position(|r| r.id == id && r.visible(server))
                    .context(crate::tr!("Dieser Runner wird nicht von der GUI verwaltet"))?;
                remove(client, &mut runners, index, &catalog).await?;
                Some(
                    crate::tr!("Lokaler Runner entfernt. Lokale Arbeitsdaten bleiben erhalten.")
                        .into(),
                )
            }
            Action::Restart { id, recovery_start } => {
                let index = runners.iter().position(|r| r.id == id && r.visible(server))
                    .context(crate::tr!("Dieser Runner wird nicht von der GUI verwaltet"))?;
                if !recovery_start && runners[index].restarting.is_none() {
                    runners[index].restarting = Some(RequestId::new());
                    atomic_json(&catalog, &runners)?;
                }
                restart(client, &runners[index], recovery_start).await?;
                runners[index].restarting = None;
                atomic_json(&catalog, &runners)?;
                Some(crate::tr!("Lokaler Runner neu gestartet und bestätigt.").into())
            }
            Action::Restore => {
                let mut errors = vec![];
                for index in 0..runners.len() {
                    if !runners[index].visible(server) {
                        continue;
                    }
                    let restored = async {
                        if runners[index].removing {
                            remove(client, &mut runners, index, &catalog).await
                        } else if runners[index].restarting.is_some() {
                            restart(client, &runners[index], false).await?;
                            runners[index].restarting = None;
                            atomic_json(&catalog, &runners)
                        } else {
                            create(client, &mut runners[index]).await?;
                            atomic_json(&catalog, &runners)?;
                            start_and_confirm(client, &runners[index]).await
                        }
                    }
                    .await;
                    if let Err(error) = restored {
                        errors.push(format!("{}: {error}", runners[index].name));
                    }
                }
                ensure!(errors.is_empty(), "{}", errors.join("\n"));
                None
            }
        };
        Ok::<_, anyhow::Error>(message)
    }
    .await;
    let user_error = result.as_ref().err().and_then(|error| {
        (starts_runner && incompatible_registration(error)).then(|| {
            crate::tr!("Runner kann nicht gestartet werden: Serverversion inkompatibel.").into()
        })
    });
    Report {
        runners: load().ok(),
        result: result.map_err(|e| e.to_string()),
        user_error,
    }
}

fn incompatible_registration(error: &anyhow::Error) -> bool {
    let diagnostic = format!("{error:#}");
    diagnostic.contains("Factory rejected the runner registration")
}

/// Register the original local-Factory runner in the same durable catalog.
pub async fn ensure_builtin(client: &FactoryClient, server: &str, root: &Path) -> Result<()> {
    let dir = config_dir()?;
    let _lock = lock(&dir)?;
    let mut runners = read_catalog(&dir)?;
    let id_path = root.join("runner-id");
    let old_id = fs::read_to_string(&id_path)
        .ok()
        .and_then(|s| s.trim().parse::<RunnerId>().ok());
    let existing = runners
        .iter()
        .position(|r| r.visible(server) && (r.builtin || Some(r.id) == old_id));
    let index = if let Some(index) = existing {
        index
    } else {
        let reuse_legacy = old_id.is_some_and(|id| !runners.iter().any(|r| r.id == id));
        let id = if reuse_legacy {
            old_id.unwrap()
        } else {
            RunnerId::new()
        };
        let runner = LocalRunner {
            id,
            server: server.into(),
            name: crate::tr!("Lokaler Desktop-Runner").into(),
            removing: false,
            removed: false,
            restarting: None,
            device: None,
            legacy: reuse_legacy,
            builtin: true,
        };
        runners.push(runner);
        atomic_json(&dir.join("local-runners.json"), &runners)?;
        // Keep the old ID file for upgrade compatibility; new runners use separate roots.
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(id.to_string().as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(&id_path)?;
        runners.len() - 1
    };
    ensure!(
        !runners[index].removing,
        crate::tr!("Entfernen des lokalen Runners zuerst abschließen")
    );
    if runners[index].restarting.is_some() {
        restart(client, &runners[index], false).await?;
        runners[index].restarting = None;
        atomic_json(&dir.join("local-runners.json"), &runners)?;
        return Ok(());
    }
    create(client, &mut runners[index]).await?;
    atomic_json(&dir.join("local-runners.json"), &runners)?;
    start_and_confirm(client, &runners[index]).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn fixture(id: RunnerId, server: &str) -> LocalRunner {
    LocalRunner {
        id,
        server: server.into(),
        name: crate::tr!("Lokaler Test-Runner").into(),
        removing: false,
        removed: false,
        restarting: None,
        device: None,
        legacy: false,
        builtin: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_start_requires_an_existing_unlocked_journal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!runner_journal_available(dir.path()).unwrap());
        let lock = fs::OpenOptions::new().read(true).write(true).create_new(true)
            .open(dir.path().join("runner.lock")).unwrap();
        assert!(!runner_journal_available(dir.path()).unwrap());
        fs::File::create(dir.path().join("runner.sqlite3")).unwrap();
        assert!(runner_journal_available(dir.path()).unwrap());
        lock.try_lock().unwrap();
        assert!(!runner_journal_available(dir.path()).unwrap());
        lock.unlock().unwrap();
        assert!(runner_journal_available(dir.path()).unwrap());
    }

    #[test]
    fn restart_keeps_identity_and_journal_and_requires_the_new_instance() {
        let id = RunnerId::new();
        let mut runner = fixture(id, "http://factory.invalid");
        let request = RequestId::new();
        runner.restarting = Some(request);
        let dir = tempfile::tempdir().unwrap();
        atomic_json(&dir.path().join("local-runners.json"), &vec![runner.clone()]).unwrap();
        let restored = read_catalog(dir.path()).unwrap();
        assert_eq!(restored[0].id, id);
        assert_eq!(restored[0].device, runner.device);
        assert_eq!(restored[0].server, runner.server);
        assert_eq!(restored[0].restarting, Some(request));
        assert!(!restored[0].removed);
        let expected = InstanceId::new();
        assert!(!restart_confirmed(&RunnerRestartStatus { instance_id: Some(InstanceId::new()), ready: true }, expected));
        assert!(!restart_confirmed(&RunnerRestartStatus { instance_id: Some(expected), ready: false }, expected));
        assert!(restart_confirmed(&RunnerRestartStatus { instance_id: Some(expected), ready: true }, expected));
    }

    #[test]
    fn pending_work_allows_restart_but_accepted_unknown_or_paused_work_does_not() {
        let app = crate::tests::fixture();
        let mut snapshot = (**app.snapshot.as_ref().unwrap()).clone();
        let id = snapshot.runners[0].id;
        snapshot.commands[0].state = CommandState::Pending;
        snapshot.streams[0].status = StreamState::Queued;
        snapshot.commands[0].agent_progress = None;
        assert!(!restart_blocked(&snapshot, id));
        assert!(busy(&snapshot, id), "Removal must still reject queued work");
        snapshot.commands[0].state = CommandState::Accepted;
        assert!(restart_blocked(&snapshot, id));
        snapshot.commands[0].state = CommandState::Failed;
        snapshot.streams[0].status = StreamState::Blocked;
        assert!(!restart_blocked(&snapshot, id));
        snapshot.streams[0].status = StreamState::Paused;
        assert!(restart_blocked(&snapshot, id));
    }

    #[test]
    fn startup_failure_includes_bounded_log_content_not_just_the_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("factory-runner.log");
        fs::write(
            &path,
            format!("{}\nError: konkrete Startursache", "x".repeat(20_000)),
        )
        .unwrap();
        let diagnostic = startup_diagnostic(root.path(), "Test-Runner");
        assert!(diagnostic.contains("Test-Runner wurde beim Start beendet"));
        assert!(diagnostic.contains("Error: konkrete Startursache"));
        assert!(diagnostic.contains(&path.display().to_string()));
        assert!(diagnostic.len() < 17_000);
    }

    #[test]
    fn registration_rejection_is_classified_as_server_incompatibility() {
        assert!(incompatible_registration(&anyhow::anyhow!(
            "Runner wurde beim Start beendet: Factory rejected the runner registration"
        )));
        assert!(!incompatible_registration(&anyhow::anyhow!(
            "Gespeicherter Runner-Zugang fehlt"
        )));
    }

    #[test]
    fn catalog_survives_restart_and_keeps_removal_intent_and_factory_scope() {
        let root = tempfile::tempdir().unwrap();
        let mut a = fixture(RunnerId::new(), "https://factory-a.example");
        let mut b = fixture(RunnerId::new(), "https://factory-b.example");
        a.removing = true;
        b.removed = true;
        let path = root.path().join("local-runners.json");
        atomic_json(&path, &vec![a.clone(), b.clone()]).unwrap();
        let restored = read_catalog(root.path()).unwrap();
        assert_eq!(restored[0].id, a.id);
        assert!(restored[0].removing);
        assert!(restored[0].visible(&a.server));
        assert!(!restored[0].visible(&b.server));
        assert!(!restored[1].visible(&b.server));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn corrupt_catalog_is_not_silently_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local-runners.json");
        fs::write(&path, "not a catalog").unwrap();
        assert!(read_catalog(root.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "not a catalog");
    }

    #[test]
    fn simultaneous_guis_cannot_manage_the_same_processes_twice() {
        let root = tempfile::tempdir().unwrap();
        let first = lock(root.path()).unwrap();
        assert!(lock(root.path()).is_err());
        drop(first);
        assert!(lock(root.path()).is_ok());
    }

    #[test]
    fn process_adoption_requires_exact_id_server_and_journal_arguments() {
        let runner = fixture(RunnerId::new(), "https://factory.example");
        let journal = Path::new("/private/runner.sqlite3");
        let args: Vec<String> = [
            "factory-runner",
            "--server",
            &runner.server,
            "--id",
            &runner.id.to_string(),
            "--journal",
            "/private/runner.sqlite3",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert!(arguments_match(&args, &runner, journal));
        assert!(!arguments_match(
            &args,
            &runner,
            Path::new("/other/runner.sqlite3")
        ));
        let other = fixture(RunnerId::new(), &runner.server);
        assert!(!arguments_match(&args, &other, journal));
        let mut other_factory = runner.clone();
        other_factory.server = "https://other.example".into();
        assert!(!arguments_match(&args, &other_factory, journal));
        // Merely mentioning the values in unrelated arguments does not establish ownership.
        assert!(!arguments_match(&[args.join(" ")], &runner, journal));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn process_identity_parser_handles_parentheses_and_rejects_foreign_processes() {
        let fields = (3..=22)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let stat = format!("123 (name with ) parentheses) {fields}");
        assert_eq!(start_ticks(&stat).unwrap(), "22");
        assert!(start_ticks("123 (bad) S").is_err());
        let identity = process(std::process::id()).unwrap();
        let runner = fixture(RunnerId::new(), "https://factory.example");
        assert!(!owns_process(
            &identity,
            &runner,
            Path::new("/private/runner.sqlite3")
        ));
        let mut recycled = identity;
        recycled.started = "0".into();
        assert!(!owns_process(
            &recycled,
            &runner,
            Path::new("/private/runner.sqlite3")
        ));
    }

    #[test]
    fn unfinished_commands_and_paused_or_uncertain_work_block_removal() {
        let app = crate::tests::fixture();
        let mut snapshot = app.snapshot.unwrap().as_ref().clone();
        let id = snapshot.runners[0].id;
        assert!(busy(&snapshot, id));
        assert!(!busy(&snapshot, RunnerId::new()));
        snapshot.commands[0].state = CommandState::Succeeded;
        for state in [
            StreamState::Running,
            StreamState::WaitingInput,
            StreamState::Paused,
            StreamState::Blocked,
            StreamState::Review,
            StreamState::Failed,
        ] {
            snapshot.streams[0].status = state;
            assert!(busy(&snapshot, id), "{state:?}");
        }
        snapshot.streams[0].status = StreamState::Completed;
        assert!(!busy(&snapshot, id));
        snapshot.commands[0].state = CommandState::Pending;
        assert!(busy(&snapshot, id));
    }
}
