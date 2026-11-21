use anyhow::{ensure, Context, Result};
use factory_core::ExcerptMessage;
use serde_json::Value;

pub(super) fn project(value: Value) -> Result<(Vec<ExcerptMessage>, bool)> {
    let items = value["data"]
        .as_array()
        .context("Invalid messages response")?;
    let cursor = value
        .get("cursor")
        .and_then(Value::as_object)
        .context("Missing messages cursor")?;
    let next = cursor.get("next").unwrap_or(&Value::Null);
    ensure!(next.is_null() || next.is_string(), "Invalid next cursor");
    let mut older = !next.is_null() || items.len() > 32;
    let mut budget = 131072;
    let mut messages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for item in items.iter().take(32) {
        if budget == 0 {
            older = true;
            break;
        }
        let id = item["id"].as_str().context("Message ID missing")?;
        let kind = item["type"].as_str().context("Message type missing")?;
        ensure!(
            !id.is_empty()
                && id.len() <= 200
                && !id.chars().any(char::is_control)
                && ids.insert(id),
            "Invalid message ID"
        );
        ensure!(
            !kind.is_empty() && kind.len() <= 80 && !kind.chars().any(char::is_control),
            "Invalid message type"
        );
        let mut text = String::new();
        let mut omitted = false;
        if let Some(t) = item["text"].as_str() {
            text.push_str(t);
        }
        if let Some(parts) = item["content"].as_array() {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => {
                        if let Some(t) = part["text"].as_str() {
                            text.push_str(t);
                            text.push('\n');
                        }
                    }
                    Some("tool") => {
                        let state = &part["state"];
                        text.push_str(&format!(
                            "\n[Tool: {} | {}]\nInput: {}\n",
                            part["name"].as_str().unwrap_or("?"),
                            state["status"].as_str().unwrap_or("?"),
                            state["input"]
                        ));
                        if let Some(content) = state["content"].as_array() {
                            for output in content {
                                if let Some(t) =
                                    output["text"].as_str().filter(|_| output["type"] == "text")
                                {
                                    text.push_str(t);
                                    text.push('\n');
                                } else {
                                    text.push_str("[Nicht-textuelle Toolausgabe ausgelassen]\n");
                                    omitted = true;
                                }
                            }
                        }
                        if let Some(error) = state.get("error") {
                            text.push_str(&format!("Fehler: {error}\n"));
                        }
                    }
                    _ => {
                        text.push_str("[Nicht dargestellter Nachrichteninhalt]\n");
                        omitted = true;
                    }
                }
            }
        }
        if item["files"].as_array().is_some_and(|v| !v.is_empty()) {
            text.push_str("\n[Dateianhaenge nicht eingebettet]");
            omitted = true;
        }
        if text.is_empty() {
            text = "[Kein darstellbarer Text; Metadaten nicht eingebettet]".into();
            omitted = true;
        }
        let mut end = text.len().min(8192).min(budget);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = omitted || end < text.len();
        text.truncate(end);
        budget -= text.len();
        messages.push(ExcerptMessage {
            id: id.into(),
            kind: kind.into(),
            text,
            truncated,
        });
    }
    messages.reverse();
    Ok((messages, older))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn projects_tool_text_without_provider_state_and_bounds_unicode() {
        let (messages,older)=project(json!({"cursor":{"next":"older"},"data":[
            {"id":"msg_new","type":"assistant","providerState":"opaque","content":[{"type":"tool","name":"shell","state":{"status":"completed","input":{"command":"echo hello"},"content":[{"type":"text","text":"hello"},{"type":"file","data":"binary"}]}}]},
            {"id":"msg_old","type":"user","text":"界".repeat(4000)}
        ]})).unwrap();
        assert!(older);
        assert_eq!(messages[0].id, "msg_old");
        assert!(messages[0].truncated);
        assert!(messages[0].text.len() <= 8192);
        assert!(messages[1].text.contains("hello"));
        assert!(messages[1].text.contains("shell"));
        assert!(!messages[1].text.contains("opaque"));
        assert!(!messages[1].text.contains("binary"));
        assert!(messages[1].truncated);
    }
    #[test]
    fn failed_shapes_are_not_empty_and_oversized_pages_are_marked() {
        assert!(project(json!({"data":[]})).is_err());
        assert!(project(json!({"cursor":{"next":null},"data":[{"type":"user"}]})).is_err());
        let data: Vec<_> = (0..32)
            .map(|n| json!({"id":format!("msg_{n}"),"type":"user","text":"x".repeat(8192)}))
            .collect();
        let (messages, older) = project(json!({"data":data,"cursor":{"next":null}})).unwrap();
        assert!(older);
        assert_eq!(messages.len(), 16);
        assert_eq!(messages.iter().map(|m| m.text.len()).sum::<usize>(), 131072);
    }
}
