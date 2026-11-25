//! The runner owns this OpenCode instance. Never start or discover a user service.
use anyhow::{bail, ensure, Context, Result};
use factory_core::{OpenCodeStatus, RequestId};
use factory_opencode::distribution::{self, VERSION};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

const CONFIGS: &[&str] = &[
    "opencode.json",
    "opencode.jsonc",
    ".opencode.json",
    ".opencode/opencode.json",
    ".opencode/opencode.jsonc",
];

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    binary: Option<PathBuf>,
    port: u16,
    config: Option<PathBuf>,
    model_key_file: Option<PathBuf>,
}

pub(crate) fn registration(journal: &Path, runner: factory_core::RunnerId) -> PathBuf {
    journal
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("opencode-{runner}"))
        .join("registration.json")
}

fn owned_dir(path: &Path) -> Result<PathBuf> {
    factory_platform::private_dir(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir(),
        "Factory OpenCode-Verzeichnis muss privat sein: {}",
        path.display()
    );
    Ok(path.canonicalize()?)
}

fn replace_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("Missing directory")?)?;
    factory_platform::private_file(file.path())?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn no_ambient_config(path: &Path) -> Result<()> {
    for relative in CONFIGS.iter().copied() {
        let file = path.join(relative);
        ensure!(
            !exists(&file)?,
            "Externe OpenCode-Konfiguration nicht erlaubt: {}",
            file.display()
        );
    }
    let directory = path.join(".opencode");
    if exists(&directory)? {
        // The installer also uses ~/.opencode for binaries and npm dependencies.
        // That alone is not configuration. Keep rejecting every discovery source
        // (agents, plugins, skills, commands, etc.) and unknown entries, without
        // touching or importing the user's installation.
        ensure!(
            fs::symlink_metadata(&directory)?.file_type().is_dir(),
            "Externes OpenCode-Verzeichnis muss ein reguläres Verzeichnis sein: {}",
            directory.display()
        );
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            ensure!(
                matches!(name.to_str(), Some("bin" | "node_modules" | "package.json"
                    | "package-lock.json" | "bun.lock" | "bun.lockb" | ".gitignore")),
                "Externe OpenCode-Konfiguration nicht erlaubt: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(crate) async fn start(
    settings: &Path,
    registration: &Path,
    factory_url: &str,
) -> Result<tokio::process::Child> {
    let settings: Settings = serde_json::from_slice(&fs::read(settings)?)
        .context("Invalid managed OpenCode settings")?;
    let port = settings.port;
    ensure!(
        port != 0,
        "Verwaltetes OpenCode braucht einen festen Loopback-Port"
    );
    ensure!(
        !exists(Path::new("/etc/opencode"))?,
        "System-Konfiguration /etc/opencode ist sichtbar; Runner isoliert starten"
    );
    let root = owned_dir(registration.parent().context("Missing runtime directory")?)?;
    for ancestor in root.ancestors().skip(1) {
        no_ambient_config(ancestor)?;
    }
    let home = owned_dir(&root.join("home"))?;
    let config_dir = owned_dir(&root.join("config"))?;
    let config_home = owned_dir(&config_dir.join("opencode"))?;
    owned_dir(&config_home.join("skills"))?;
    let data = owned_dir(&root.join("data"))?;
    let state = owned_dir(&root.join("state"))?;
    let cache = owned_dir(&root.join("cache"))?;
    let runtime = owned_dir(&root.join("runtime"))?;
    let tmp = owned_dir(&root.join("tmp"))?;
    let base = owned_dir(&root.join("workspace"))?;
    no_ambient_config(&base)?;
    no_ambient_config(&root)?;
    write_config(&settings, &config_home.join("opencode.jsonc"), factory_url).await?;
    let override_path = std::env::var_os("FACTORY_OPENCODE_BIN").map(PathBuf::from);
    let binary = distribution::binary(&root, settings.binary.as_deref().or(override_path.as_deref())).await?;
    let password = format!("{}{}", RequestId::new(), RequestId::new());
    let url = format!("http://127.0.0.1:{port}");
    let command = || {
        let mut command = tokio::process::Command::new(&binary);
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &config_dir)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CACHE_HOME", &cache)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("TMPDIR", &tmp)
            .env("TEMP", &tmp)
            .env("TMP", &tmp)
            .env("USERPROFILE", &home)
            .env("APPDATA", &config_dir)
            .env("LOCALAPPDATA", &data)
            // Agent workspaces otherwise accumulate full debug information and
            // incremental state independently for every job.
            .envs(cargo_build_env(std::env::var_os("CARGO_TARGET_DIR")))
            .env("OPENCODE_PASSWORD", &password)
            .current_dir(&base)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        for key in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
    };
    let reported =
        tokio::time::timeout(Duration::from_secs(5), command().arg("--version").output()).await??;
    ensure!(
        reported.status.success()
            && std::str::from_utf8(&reported.stdout)?.trim() == format!("opencode v{VERSION}"),
        "Managed OpenCode requires version {VERSION}"
    );
    let mut child = command()
        .args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .spawn()
        .context("Factory-OpenCode konnte nicht gestartet werden")?;
    let probe = tempfile::NamedTempFile::new_in(&root)?;
    let descriptor = serde_json::to_vec(&serde_json::json!({"url": url, "password": password}))?;
    fs::write(probe.path(), &descriptor)?;
    let client = factory_opencode::OpenCodeClient::from_registration(probe.path())?;
    for _ in 0..60 {
        ensure!(
            child.try_wait()?.is_none(),
            "Factory-OpenCode wurde beim Start beendet"
        );
        if let OpenCodeStatus::Ready { version } = client.status().await {
            ensure!(version == VERSION, "Unerwartete Factory-OpenCode-Version");
            replace_private(registration, &descriptor)?;
            return Ok(child);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    bail!("Factory-OpenCode ist nicht erreichbar")
}

pub(crate) async fn refresh(settings: &Path, registration: &Path, factory_url: &str) -> Result<()> {
    let settings: Settings = serde_json::from_slice(&fs::read(settings)?)
        .context("Invalid managed OpenCode settings")?;
    if settings.model_key_file.is_some() {
        let config = registration
            .parent()
            .context("Missing runtime directory")?
            .join("config/opencode/opencode.jsonc");
        write_config(&settings, &config, factory_url).await?;
    }
    Ok(())
}

async fn write_config(settings: &Settings, output: &Path, factory_url: &str) -> Result<()> {
    ensure!(
        settings
            .config
            .as_ref()
            .is_none_or(|path| path.is_absolute()),
        "Runner-Konfiguration muss ein absoluter Pfad sein"
    );
    let raw = settings
        .config
        .as_ref()
        .map(fs::read)
        .transpose()?
        .unwrap_or_else(|| b"{}".to_vec());
    let mut content = parse_jsonc(&raw).context("Ungültige Runner-OpenCode-Konfiguration")?;
    if let Some(key_file) = &settings.model_key_file {
        ensure!(key_file.is_absolute(), "Model key file path must be absolute");
        add_gateway_config(factory_url, key_file, &mut content).await?;
    }
    let plugin = owned_dir(&output.parent().context("Missing configuration directory")?.join("yoloforge-direct-intents"))?;
    for (name, bytes) in [
        ("package.json", br#"{"name":"yoloforge-direct-intents","type":"module","exports":"./index.js"}"#.as_slice()),
        ("index.js", include_bytes!("direct_intent_tool.js").as_slice()),
    ] {
        let file = plugin.join(name);
        if !fs::read(&file).is_ok_and(|content| content == bytes) {
            replace_private(&file, bytes)?;
        }
    }
    if content["plugins"].is_null() {
        content["plugins"] = serde_json::json!([]);
    }
    content["plugins"].as_array_mut().context("Managed OpenCode plugins must be an array")?
        .push(serde_json::json!(plugin));
    let content = serde_json::to_vec(&content)?;
    if fs::read(output).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    replace_private(output, &content)
}

async fn add_gateway_config(factory_url: &str, key_file: &Path, config: &mut Value) -> Result<()> {
    use factory_protocol::models::ModelAccess;
    let url = reqwest::Url::parse(factory_url)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && (url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))),
        "Model gateway requires HTTPS or a local tunnel"
    );
    let metadata = fs::symlink_metadata(key_file)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 130,
        "Model gateway key file must be a private regular file (0600)"
    );
    factory_platform::ensure_private(key_file)?;
    let key = fs::read_to_string(key_file)?;
    let key = key.trim_end_matches(['\n', '\r']);
    ensure!(
        (32..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic()),
        "Invalid model gateway key"
    );
    let base = format!("{}/v1/model-gateway", factory_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()?;
    let accesses: Vec<ModelAccess> = client
        .get(format!("{base}/catalog"))
        .bearer_auth(key)
        .send()
        .await
        .context("Factory-Modellkatalog nicht erreichbar")?
        .error_for_status()
        .context("Factory-Modellzugang nicht verfügbar")?
        .json()
        .await?;
    validate_config(Path::new("Runner-Konfiguration"), config)?;
    let config = config.as_object_mut().expect("validated config");
    let mut rules = config
        .remove("policies")
        .unwrap_or_else(|| Value::Array(vec![]));
    let rules_list = rules
        .as_array_mut()
        .context("Runner policies must be an array")?;
    rules_list.push(serde_json::json!({"action":"provider.use","resource":"*","effect":"deny"}));
    let mut providers = serde_json::Map::new();
    for access in accesses {
        ensure!(
            access.id.starts_with("factory-")
                && access
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "Invalid Factory provider alias"
        );
        let models = access
            .models
            .into_iter()
            .map(|model| {
                let mut config = serde_json::json!({"capabilities":{"tools":true}});
                if let Some(transport) = access.transports.get(&model) {
                    config["package"] = serde_json::json!(transport.runtime_package());
                }
                (model, config)
            })
            .collect::<serde_json::Map<_, _>>();
        let alias = access.id;
        rules_list
            .push(serde_json::json!({"action":"provider.use","resource":alias,"effect":"allow"}));
        let package = factory_protocol::models::KeyModelProvider::from_access_id(&alias)
            .map_or("@opencode/ai/providers/openai-compatible/responses", |p| p.runtime_package());
        let value = serde_json::json!({
            "name": access.name,
            "package":package,
            "settings":{"baseURL":format!("{base}/{alias}"),"apiKey":key},
            "models":models,
        });
        providers.insert(alias, value);
    }
    config.insert("providers".into(), Value::Object(providers));
    config.insert("policies".into(), rules);
    let permissions = config.entry("permissions").or_insert_with(|| serde_json::json!([]));
    if let Some(rules) = permissions.as_array_mut() {
        rules.push(serde_json::json!({"action":"skill","resource":format!("{}*",factory_core::FACTORY_SKILL_PREFIX),"effect":"deny"}));
    }
    Ok(())
}

fn parse_jsonc(source: &[u8]) -> Result<Value> {
    // Mask comments and trailing commas, keeping quoted literals intact.
    let mut bytes = source.to_vec();
    let mut i = 0;
    let mut comma = None;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1).copied()) {
            (b'"', _) => {
                comma = None;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            (b'/', Some(b'/')) => {
                while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                    bytes[i] = b' ';
                    i += 1;
                }
            }
            (b'/', Some(b'*')) => {
                let end = source[i + 2..]
                    .windows(2)
                    .position(|w| w == b"*/")
                    .context("Unclosed comment")?
                    + i
                    + 4;
                for b in &mut bytes[i..end] {
                    if !matches!(*b, b'\n' | b'\r') {
                        *b = b' ';
                    }
                }
                i = end;
            }
            (b',', _) => {
                comma = Some(i);
                i += 1;
            }
            (b']' | b'}', _) => {
                if let Some(previous) = comma.take() {
                    bytes[previous] = b' ';
                }
                i += 1;
            }
            (b, _) if b.is_ascii_whitespace() => i += 1,
            _ => {
                comma = None;
                i += 1;
            }
        }
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(value.is_object(), "OpenCode config must be an object");
    Ok(value)
}

fn model_field(key: &str) -> bool {
    matches!(
        key,
        "model"
            | "provider"
            | "providers"
            | "small_model"
            | "enabled_providers"
            | "disabled_providers"
    )
}

fn policy_field(key: &str, value: &Value) -> bool {
    key == "policies"
        && value.as_array().is_some_and(|rules| {
            rules.iter().any(|rule| {
                rule.get("action")
                    .and_then(Value::as_str)
                    .is_none_or(|action| {
                        action == "*"
                            || action.contains('{')
                            || action.starts_with("provider.")
                            || action.starts_with("model.")
                    })
            })
        })
}

pub(crate) fn validate_config(path: &Path, value: &Value) -> Result<()> {
    for (key, value) in value
        .as_object()
        .context("OpenCode config must be an object")?
    {
        ensure!(
            !key.contains('{'),
            "{}: keine Variablen in OpenCode-Feldnamen erlaubt",
            path.display()
        );
        ensure!(
            !(model_field(key) || policy_field(key, value)),
            "{}: {key} muss in der Factory konfiguriert werden",
            path.display()
        );
        ensure!(
            !matches!(key.as_str(), "plugin" | "plugins"),
            "{}: Repo-Plugins benötigen eine Factory-Freigabe",
            path.display()
        );
        if matches!(key.as_str(), "agent" | "agents" | "command" | "commands") {
            for (name, fields) in value.as_object().into_iter().flatten() {
                for field in fields.as_object().into_iter().flat_map(|f| f.keys()) {
                    ensure!(
                        !model_field(field) && !field.contains('{'),
                        "{}: {key}.{name}.{field} muss in der Factory konfiguriert werden",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_markdown(path: &Path) -> Result<()> {
    let source = fs::read_to_string(path)?;
    let mut lines = source.trim_start_matches('\u{feff}').lines();
    if !lines.next().is_some_and(|line| line.trim() == "---") {
        return Ok(());
    }
    let mut frontmatter = String::new();
    for line in lines {
        if matches!(line.trim(), "---" | "...") {
            let fields: serde_yaml::Mapping = if frontmatter.trim().is_empty() {
                serde_yaml::Mapping::new()
            } else {
                serde_yaml::from_str(&frontmatter)
                    .with_context(|| format!("{}: ungültige OpenCode-Metadaten", path.display()))?
            };
            ensure!(
                !fields.keys().any(|key| key
                    .as_str()
                    .is_none_or(|key| key == "<<" || model_field(key))),
                "{}: Modellvorgaben oder YAML-Vererbung sind nicht erlaubt",
                path.display()
            );
            return Ok(());
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    bail!("{}: OpenCode-Metadaten nicht abgeschlossen", path.display())
}

fn scan_markdown(dir: &Path, depth: usize) -> Result<()> {
    ensure!(
        depth <= 16,
        "{}: OpenCode-Konfiguration zu tief verschachtelt",
        dir.display()
    );
    if !exists(dir)? {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(dir)?.file_type().is_symlink(),
        "{}: Symlink nicht erlaubt",
        dir.display()
    );
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        ensure!(
            !kind.is_symlink(),
            "{}: Symlink nicht erlaubt",
            path.display()
        );
        if kind.is_dir() {
            scan_markdown(&path, depth + 1)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            ensure!(kind.is_file(), "{}: keine reguläre Datei", path.display());
            validate_markdown(&path)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_workspace(workspace: &Path) -> Result<()> {
    validate_workspace_with_profile(workspace, false)
}

pub(crate) fn validate_workspace_with_profile(workspace: &Path, managed_integrations: bool) -> Result<()> {
    let workspace = workspace.canonicalize()?;
    for ancestor in workspace.ancestors().skip(1) {
        no_ambient_config(ancestor)?;
    }
    let oc = workspace.join(".opencode");
    if exists(&oc)? {
        ensure!(
            fs::symlink_metadata(&oc)?.is_dir(),
            ".opencode: muss ein reguläres Verzeichnis sein"
        );
    }
    for name in ["plugin", "plugins"] {
        ensure!(
            !exists(&oc.join(name))?,
            ".opencode/{name}: Repo-Plugins benötigen eine Factory-Freigabe"
        );
    }
    for relative in CONFIGS {
        let path = workspace.join(relative);
        if exists(&path)? {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && metadata.len() <= 1024 * 1024,
                "{relative}: ungültige Konfigurationsdatei"
            );
            let value = parse_jsonc(&fs::read(&path)?)
                .with_context(|| format!("{relative}: ungültige OpenCode-Konfiguration"))?;
            validate_config(Path::new(relative), &value)?;
            ensure!(
                !managed_integrations || (value.get("skills").is_none() && value.get("mcp").is_none()),
                "{relative}: externe Skill-Quellen und Repo-MCPs sind mit verwalteten Integrationen nicht erlaubt"
            );
        }
    }
    for name in ["agent", "agents", "command", "commands"] {
        scan_markdown(&oc.join(name), 0)?;
    }
    Ok(())
}

const CARGO_BUILD_ENV: [(&str, &str); 4] = [
    ("CARGO_PROFILE_DEV_DEBUG", "0"),
    ("CARGO_PROFILE_DEV_INCREMENTAL", "false"),
    ("CARGO_PROFILE_TEST_DEBUG", "0"),
    ("CARGO_PROFILE_TEST_INCREMENTAL", "false"),
];

fn cargo_build_env(target: Option<std::ffi::OsString>) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut env: Vec<_> = CARGO_BUILD_ENV
        .iter()
        .map(|(key, value)| ((*key).into(), (*value).into()))
        .collect();
    if let Some(target) = target {
        env.push(("CARGO_TARGET_DIR".into(), target));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runner_builds_disable_debug_symbols_and_incremental_state() {
        let mut command = std::process::Command::new("unused");
        command.env_clear().envs(cargo_build_env(None));
        let env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
            .collect();
        for profile in ["DEV", "TEST"] {
            assert_eq!(env[format!("CARGO_PROFILE_{profile}_DEBUG").as_str()], "0");
            assert_eq!(
                env[format!("CARGO_PROFILE_{profile}_INCREMENTAL").as_str()],
                "false"
            );
        }
        assert!(!env.contains_key("CARGO_TARGET_DIR"));
    }

    #[test]
    fn runner_forwards_the_shared_cargo_target_without_other_ambient_settings() {
        let target = std::ffi::OsString::from("/shared/build files");
        let env = cargo_build_env(Some(target.clone()));
        assert_eq!(env.len(), 5);
        assert_eq!(env.last().unwrap(), &("CARGO_TARGET_DIR".into(), target));
    }

    #[test]
    fn repo_options_are_preserved_but_model_sources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("opencode.jsonc");
        fs::write(
            &file,
            r#"{"instructions":["https://example.test/foo"], /* keep */ "share": "disabled",}"#,
        )
        .unwrap();
        validate_workspace(dir.path()).unwrap();
        for (json, diagnostic) in [
            (r#"{"providers":{}}"#, "opencode.jsonc: providers"),
            (
                r#"{"agents":{"build":{"model":"openai/test"}}}"#,
                "opencode.jsonc: agents.build.model",
            ),
            (
                r#"{"commands":{"review":{"\u006dodel":"openai/test"}}}"#,
                "opencode.jsonc: commands.review.model",
            ),
            (r#"{"plugins":["unsafe"]}"#, "opencode.jsonc: Repo-Plugins"),
        ] {
            fs::write(&file, json).unwrap();
            assert!(validate_workspace(dir.path())
                .unwrap_err()
                .to_string()
                .starts_with(diagnostic));
        }
        fs::remove_file(file).unwrap();
        fs::create_dir_all(dir.path().join(".opencode/agents")).unwrap();
        let agent = dir.path().join(".opencode/agents/reviewer.md");
        fs::write(
            &agent,
            "---\n{\"model\": \"openai/test\"}\n---\nReview only.\n",
        )
        .unwrap();
        assert!(validate_workspace(dir.path())
            .unwrap_err()
            .to_string()
            .contains("reviewer.md: Modell"));
    }

    #[test]
    fn configured_repo_skill_profiles_reject_extra_sources_but_legacy_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("opencode.jsonc");
        for json in [
            r#"{"skills":["https://example.test/skills"]}"#,
            r#"{"mcp":{"servers":{"local":{"type":"local","command":["ignored"]}}}}"#,
        ] {
            fs::write(&file, json).unwrap();
            validate_workspace(dir.path()).unwrap();
            assert!(validate_workspace_with_profile(dir.path(), true)
                .unwrap_err().to_string().contains("externe Skill-Quellen"));
        }
    }

    #[test]
    fn personal_binary_installation_does_not_block_a_private_runtime_or_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join(".opencode");
        fs::create_dir_all(install.join("bin")).unwrap();
        fs::create_dir_all(install.join("node_modules")).unwrap();
        fs::write(install.join("package.json"), r#"{"dependencies":{"opencode-ai":"latest"}}"#).unwrap();
        fs::write(install.join("package-lock.json"), "{}").unwrap();
        fs::write(install.join(".gitignore"), "*").unwrap();
        let workspace = dir.path().join("factory/runtime/workspace");
        fs::create_dir_all(&workspace).unwrap();
        no_ambient_config(dir.path()).unwrap();
        validate_workspace(&workspace).unwrap();
        assert_eq!(fs::read_to_string(install.join("package-lock.json")).unwrap(), "{}");
        for name in ["agents", "commands", "skills", "plugins", "plugin", "unknown"] {
            let external = install.join(name);
            fs::create_dir(&external).unwrap();
            assert!(no_ambient_config(dir.path()).is_err(), "{name} must remain blocked");
            assert!(validate_workspace(&workspace).is_err());
            fs::remove_dir(external).unwrap();
        }
        for name in ["opencode.json", "opencode.jsonc"] {
            let external = install.join(name);
            fs::write(&external, "{}").unwrap();
            assert!(no_ambient_config(dir.path()).is_err(), "{name} must remain blocked");
            fs::remove_file(external).unwrap();
        }
    }

    #[test]
    fn rejects_config_in_a_worktree_ancestor_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("repo");
        fs::create_dir(&workspace).unwrap();
        let foreign = dir.path().join("opencode.json");
        fs::write(&foreign, "{}").unwrap();
        assert!(validate_workspace(&workspace)
            .unwrap_err()
            .to_string()
            .contains("Externe OpenCode-Konfiguration"));
        fs::remove_file(&foreign).unwrap();
        crate::test_symlink_file(&foreign, workspace.join(".opencode")).unwrap();
        assert!(validate_workspace(&workspace).is_err());
    }

    #[tokio::test]
    #[ignore = "requires an explicit OpenCode 2.4.1 binary, not the user's running service"]
    async fn private_runtime_fetches_the_factory_catalog_and_uses_separate_state() {
        let binary = std::env::var("FACTORY_TEST_OPENCODE")
            .expect("Pass the absolute OpenCode 2.4.1 binary");
        let root = tempfile::tempdir_in("/tmp/opencode").unwrap();
        let settings = root.path().join("settings.json");
        let registration = root.path().join("runtime/registration.json");
        let factory = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let factory_url = format!("http://{}", factory.local_addr().unwrap());
        let key = "runner-only-model-key-for-isolated-test";
        let key_file = root.path().join("model.key");
        replace_private(&key_file, key.as_bytes()).unwrap();
        use factory_protocol::models::KeyModelProvider;
        let providers = KeyModelProvider::ALL.map(|p| (p, p.new_access_id()));
        let mut catalog = vec![serde_json::json!({"id":"factory-test","name":"Factory Test","models":["sample"]})];
        catalog.extend(providers.iter().map(|(provider, id)| {
            serde_json::json!({"id":id,"name":provider.label(),"models":["sample"]})
        }));
        let copilot = format!("{}{}", factory_protocol::models::COPILOT_ACCESS_PREFIX, factory_core::RequestId::new());
        catalog.push(serde_json::json!({"id":copilot,"name":"GitHub Copilot",
            "models":["sample-chat","sample-responses","sample-messages"],
            "transports":{"sample-chat":"chat","sample-responses":"responses","sample-messages":"messages"}}));
        let route = axum::Router::new().route("/v1/model-gateway/catalog", axum::routing::get(
            move |headers: axum::http::HeaderMap| {
                let catalog = catalog.clone();
                async move {
                    assert_eq!(headers["authorization"], format!("Bearer {key}"));
                    axum::Json(catalog)
                }
            }));
        let mock = tokio::spawn(async move {
            axum::serve(factory, route).await.unwrap();
        });
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        fs::write(
            &settings,
            serde_json::to_vec(&serde_json::json!({
                "binary":binary,"port":port,"model_key_file":key_file
            }))
            .unwrap(),
        )
        .unwrap();
        let mut child = start(&settings, &registration, &factory_url).await.unwrap();
        let descriptor: Value = serde_json::from_slice(&fs::read(&registration).unwrap()).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for resource in ["config", "credential"] {
            let result: Value = client
                .get(format!("http://127.0.0.1:{port}/api/{resource}"))
                .basic_auth("opencode", descriptor["password"].as_str())
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            if resource == "config" {
                let entries = result.as_array().unwrap();
                assert!(result
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(
                        |entry| entry["info"]["providers"]["factory-test"]["models"]["sample"]
                            .is_object()
                    ));
                for (provider, id) in &providers {
                    assert!(entries.iter().any(|entry| {
                        let access = &entry["info"]["providers"][id];
                        access["package"] == provider.runtime_package()
                            && access["models"]["sample"]["capabilities"]["tools"] == true
                    }), "{provider:?} missing from runner config");
                }
                for (model, transport) in [
                    ("sample-chat", factory_protocol::models::ModelTransport::Chat),
                    ("sample-responses", factory_protocol::models::ModelTransport::Responses),
                    ("sample-messages", factory_protocol::models::ModelTransport::Messages),
                ] {
                    assert!(entries.iter().any(|entry| {
                        entry["info"]["providers"][&copilot]["models"][model]["package"] == transport.runtime_package()
                    }));
                }
            } else {
                assert_eq!(result["data"], serde_json::json!([]));
            }
        }
        child.kill().await.unwrap();
        mock.abort();
    }
}
