use anyhow::{ensure, Context, Result};
use factory_core::*;
use serde_json::Value;
use std::{
    io::Read,
    path::Path,
};

fn read(root: &Path, uri: &str, budget: usize) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(uri)?;
    ensure!(
        url.scheme() == "file" && url.host_str().is_none_or(|h| h == "localhost"),
        "Only local file attachments can be captured"
    );
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid file URI"))?
        .canonicalize()?;
    ensure!(
        path.starts_with(root),
        "Attachment is outside execution workspace"
    );
    let file = factory_platform::open_rooted(&path, &[root])?;
    let maximum = budget.min(ARCHIVE_FILE_LIMIT);
    ensure!(
        file.metadata()?.len() <= maximum as u64,
        "Attachment exceeds capture budget"
    );
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= maximum,
        "Attachment grew beyond capture budget"
    );
    Ok(bytes)
}
pub fn capture(messages: &mut [Value], workspace: &str) -> Result<()> {
    let root = Path::new(workspace).canonicalize().ok();
    let size = serde_json::to_vec(messages)?.len();
    // Keep the existing 8 MiB archive-page contract, including envelope and metadata.
    let mut budget =
        ((8 * 1024 * 1024usize).saturating_sub(size + 256 * 1024) / 4 * 3).min(ARCHIVE_FILE_LIMIT);
    if size > 8 * 1024 * 1024 - 256 * 1024 {
        return Ok(());
    }
    let mut remaining = 16;
    for message in messages {
        ensure!(
            message.get(ARCHIVE_FILES_KEY).is_none(),
            "Archive snapshot namespace already present in upstream message"
        );
        let references = archive_file_references(message);
        if references.is_empty() {
            continue;
        }
        let mut snapshots = Vec::new();
        for (uri, name, mime) in references.into_iter().take(remaining) {
            remaining -= 1;
            let bytes = root
                .as_ref()
                .context("Workspace missing")
                .and_then(|root| read(root, &uri, budget));
            let file = match bytes {
                Ok(bytes) => {
                    budget -= bytes.len();
                    ArchiveFile::captured(uri, name, mime, &bytes).map_err(anyhow::Error::msg)?
                }
                Err(_) => ArchiveFile {
                    uri,
                    name,
                    mime,
                    status: "unavailable_outside_workspace_or_over_limit".into(),
                    sha256: None,
                    data: None,
                },
            };
            snapshots.push(file);
        }
        message
            .as_object_mut()
            .context("Archive message is not an object")?
            .insert(ARCHIVE_FILES_KEY.into(), serde_json::to_value(snapshots)?);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_snapshots_preserve_raw_messages_and_never_follow_outside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = root.path().join("result.bin");
        std::fs::write(&path, b"\0\xffbinary").unwrap();
        let secret = outside.path().join("private");
        std::fs::write(&secret, b"private data").unwrap();
        let link = root.path().join("escape");
        crate::test_symlink_file(&secret, &link).unwrap();
        let uri = reqwest::Url::from_file_path(&path).unwrap().to_string();
        let escape = reqwest::Url::from_file_path(&link).unwrap().to_string();
        let raw = serde_json::json!({"id":"msg_binary","content":[{"type":"file","uri":uri,"mime":"application/octet-stream"},{"type":"file","uri":escape,"mime":"text/plain"},{"type":"file","uri":"http://127.0.0.1/private","mime":"text/plain"}]});
        let mut messages = vec![raw.clone()];
        capture(&mut messages, root.path().to_str().unwrap()).unwrap();
        let files: Vec<ArchiveFile> =
            serde_json::from_value(messages[0][ARCHIVE_FILES_KEY].clone()).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].bytes().unwrap().unwrap(), b"\0\xffbinary");
        assert!(files[1].bytes().unwrap().is_none());
        assert!(files[2].bytes().unwrap().is_none());
        messages[0]
            .as_object_mut()
            .unwrap()
            .remove(ARCHIVE_FILES_KEY);
        assert_eq!(messages[0], raw);
        std::fs::remove_file(path).unwrap();
        assert_eq!(files[0].bytes().unwrap().unwrap(), b"\0\xffbinary");
    }
}
