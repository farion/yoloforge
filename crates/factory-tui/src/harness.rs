//! Lossless project-harness edits. Only the selected MCP and explicit additions change.
use serde_json::{json, Map, Value};

pub struct Edit<'a> {
    pub name: &'a str,
    pub action: &'a str,
    pub endpoint: &'a str,
    pub arguments: &'a str,
    pub instructions: &'a str,
}
fn object<'a>(value: &'a mut Value, field: &str) -> Result<&'a mut Map<String, Value>, String> {
    value
        .as_object_mut()
        .ok_or_else(|| format!("{field} muss ein JSON-Objekt sein"))
}
pub fn apply(base: Option<Value>, edit: Edit<'_>) -> Result<Option<Value>, String> {
    if edit.action == "keep" && edit.instructions.trim().is_empty() {
        if !edit.name.is_empty() || !edit.endpoint.is_empty() || !edit.arguments.is_empty() {
            return Err(
                "MCP-Aktion auswaehlen; Eingaben werden nicht stillschweigend verworfen".into(),
            );
        }
        return Ok(base);
    }
    let mut config = base.unwrap_or_else(|| json!({"$schema":"https://opencode.ai/config.json"}));
    let root = object(&mut config, "Harness")?;
    if edit.action != "keep" {
        if edit.name.is_empty()
            || edit.name.len() > 120
            || !edit
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err("MCP-Name: 1..120 Buchstaben, Ziffern, _ oder -".into());
        }
        let mcp = object(root.entry("mcp").or_insert_with(|| json!({})), "mcp")?;
        let servers = object(
            mcp.entry("servers").or_insert_with(|| json!({})),
            "mcp.servers",
        )?;
        match edit.action {
            "remove" => {
                if servers.remove(edit.name).is_none() {
                    return Err("MCP nicht vorhanden".into());
                }
            }
            "enable" | "disable" => {
                if !edit.endpoint.is_empty() || !edit.arguments.is_empty() {
                    return Err(
                        "Aktivieren/Deaktivieren: Programm/URL und Argumente leer lassen".into(),
                    );
                }
                let server = object(
                    servers.get_mut(edit.name).ok_or("MCP nicht vorhanden")?,
                    "MCP",
                )?;
                server.insert("disabled".into(), json!(edit.action == "disable"));
            }
            kind @ ("local" | "remote") => {
                let mut server = servers
                    .get(edit.name)
                    .cloned()
                    .unwrap_or_else(|| json!({"type":kind}));
                let server_object = object(&mut server, "MCP")?;
                if server_object.get("type").and_then(Value::as_str) != Some(kind) {
                    return Err(
                        "MCP-Typwechsel: bestehenden Eintrag explizit entfernen, dann neu anlegen"
                            .into(),
                    );
                }
                if kind == "local" {
                    if edit.endpoint.trim().is_empty()
                        || edit.endpoint.chars().any(char::is_control)
                    {
                        return Err(
                            "Lokaler MCP: ausfuehrbares Programm angeben (Argumente separat)"
                                .into(),
                        );
                    }
                    let mut command = vec![edit.endpoint.to_owned()];
                    if !edit.arguments.is_empty() {
                        command.extend(edit.arguments.lines().map(str::to_owned));
                    }
                    if command.len() > 128 || command.iter().any(|s| s.contains('\0')) {
                        return Err("Zu viele oder ungueltige MCP-Argumente".into());
                    }
                    server_object.insert("command".into(), json!(command));
                } else {
                    if !edit.arguments.is_empty() {
                        return Err("Entfernte MCPs haben keine Programmargumente".into());
                    }
                    let url =
                        reqwest::Url::parse(edit.endpoint).map_err(|_| "Ungueltige MCP-URL")?;
                    if !matches!(url.scheme(), "http" | "https")
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.fragment().is_some()
                    {
                        return Err("MCP-URL: HTTP(S), ohne Benutzerinfo oder Fragment".into());
                    }
                    server_object.insert("url".into(), json!(edit.endpoint));
                }
                servers.insert(edit.name.into(), server);
            }
            _ => return Err("Unbekannte MCP-Aktion".into()),
        }
    }
    if !edit.instructions.trim().is_empty() {
        let instructions = root
            .entry("instructions")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("instructions muss eine Liste sein")?;
        if !instructions.iter().all(Value::is_string) {
            return Err("instructions enthaelt unbekannte Eintraege".into());
        }
        for path in edit
            .instructions
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if path.chars().any(char::is_control) {
                return Err("Ungueltiger Regeldatei-Eintrag".into());
            }
            let value = json!(path);
            if !instructions.contains(&value) {
                instructions.push(value);
            }
        }
    }
    if serde_json::to_vec(&config)
        .map_err(|_| "Harness nicht serialisierbar")?
        .len()
        > 32768
    {
        return Err("Harness ueberschreitet 32 KiB".into());
    }
    Ok(Some(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit<'a>(name: &'a str, action: &'a str, endpoint: &'a str, arguments: &'a str) -> Edit<'a> {
        Edit {
            name,
            action,
            endpoint,
            arguments,
            instructions: "AGENTS.md\nRULES.md",
        }
    }
    #[test]
    fn edits_preserve_unrelated_config_and_mcp_auth_and_arguments_are_literal() {
        let base = json!({"plugins":["example"],"agents":{"reviewer":{"system":"test"}},"instructions":["AGENTS.md"],"mcp":{"timeout":{"startup":1234},"servers":{"remote":{"type":"remote","url":"https://old.invalid/mcp","oauth":{"client_id":"{env:CLIENT}"},"headers":{"X-Test":"value"}},"local":{"type":"local","command":["old"],"environment":{"ENV":"value"}}}}});
        let remote = apply(
            Some(base.clone()),
            edit("remote", "remote", "https://new.invalid/mcp", ""),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            remote["mcp"]["servers"]["remote"]["oauth"],
            base["mcp"]["servers"]["remote"]["oauth"]
        );
        assert_eq!(
            remote["mcp"]["servers"]["local"],
            base["mcp"]["servers"]["local"]
        );
        let local = apply(
            Some(remote),
            edit(
                "local",
                "local",
                "tool",
                "space in arg\n$(not-a-shell)\n--flag",
            ),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            local["mcp"]["servers"]["local"]["command"],
            json!(["tool", "space in arg", "$(not-a-shell)", "--flag"])
        );
        assert_eq!(local["instructions"], json!(["AGENTS.md", "RULES.md"]));
        assert_eq!(local["plugins"], base["plugins"]);
        assert_eq!(local["agents"], base["agents"]);
        assert_eq!(
            local["mcp"]["servers"]["local"]["environment"],
            base["mcp"]["servers"]["local"]["environment"]
        );
    }
    #[test]
    fn explicit_remove_disable_and_invalid_edits_do_not_destroy_other_entries() {
        let base = json!({"mcp":{"servers":{"one":{"type":"remote","url":"https://one.invalid"},"two":{"type":"local","command":["tool"]}}}});
        assert!(apply(Some(base.clone()), edit("one", "local", "tool", "")).is_err());
        assert!(apply(
            Some(base.clone()),
            edit("one", "remote", "https://user:secret@example.invalid", "")
        )
        .is_err());
        let disabled = apply(Some(base.clone()), edit("one", "disable", "", ""))
            .unwrap()
            .unwrap();
        assert_eq!(disabled["mcp"]["servers"]["one"]["disabled"], true);
        let removed = apply(Some(disabled), edit("one", "remove", "", ""))
            .unwrap()
            .unwrap();
        assert!(removed["mcp"]["servers"].get("one").is_none());
        assert_eq!(
            removed["mcp"]["servers"]["two"],
            base["mcp"]["servers"]["two"]
        );
        assert!(apply(
            Some(json!({"mcp":"unexpected"})),
            edit("one", "remote", "https://one.invalid", "")
        )
        .is_err());
    }
}
