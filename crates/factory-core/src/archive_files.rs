use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const ARCHIVE_FILES_KEY: &str = "__yoloforge_files_v1";
pub const ARCHIVE_FILE_LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveFile {
    pub uri: String,
    pub name: String,
    pub mime: String,
    pub status: String,
    pub sha256: Option<String>,
    pub data: Option<String>,
}
impl ArchiveFile {
    pub fn captured(
        uri: String,
        name: String,
        mime: String,
        bytes: &[u8],
    ) -> Result<Self, &'static str> {
        if bytes.len() > ARCHIVE_FILE_LIMIT {
            return Err("Archive file exceeds 4 MiB");
        }
        Ok(Self {
            uri,
            name,
            mime,
            status: "captured".into(),
            sha256: Some(format!("{:x}", Sha256::digest(bytes))),
            data: Some(STANDARD.encode(bytes)),
        })
    }
    pub fn bytes(&self) -> Result<Option<Vec<u8>>, &'static str> {
        if self.status != "captured" {
            return Ok(None);
        }
        let data = self.data.as_deref().ok_or("Archive file data missing")?;
        if data.len() > ARCHIVE_FILE_LIMIT.div_ceil(3) * 4 {
            return Err("Archive file exceeds size limit");
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "Invalid archive file encoding")?;
        if bytes.len() > ARCHIVE_FILE_LIMIT
            || self.sha256.as_deref() != Some(format!("{:x}", Sha256::digest(&bytes)).as_str())
        {
            return Err("Archive file digest mismatch");
        }
        Ok(Some(bytes))
    }
}
/// Only typed URI attachments are considered, never URLs found in ordinary text/tool logs.
pub fn archive_file_references(value: &serde_json::Value) -> Vec<(String, String, String)> {
    fn walk(v: &serde_json::Value, out: &mut Vec<(String, String, String)>, depth: usize) {
        if depth > 32 || out.len() >= 16 {
            return;
        }
        if let (Some(uri), Some(mime)) = (v["uri"].as_str(), v["mime"].as_str()) {
            if uri.len() <= 8192
                && mime.len() <= 200
                && !uri.starts_with("data:")
                && !out.iter().any(|(u, _, _)| u == uri)
            {
                out.push((
                    uri.into(),
                    v["name"]
                        .as_str()
                        .unwrap_or("attachment.bin")
                        .chars()
                        .take(255)
                        .collect(),
                    mime.into(),
                ));
            }
        }
        match v {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if key != ARCHIVE_FILES_KEY {
                        walk(value, out, depth + 1);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    walk(value, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out, 0);
    out
}
/// Inline OpenCode attachments already reside in the raw archive; decode them only on export.
pub fn archive_inline_files(value: &serde_json::Value) -> Vec<ArchiveFile> {
    fn walk(v: &serde_json::Value, out: &mut Vec<ArchiveFile>, depth: usize) {
        if depth > 32 || out.len() >= 16 {
            return;
        }
        if let Some(mime) = v["mime"].as_str().filter(|s| s.len() <= 200) {
            let data = v["uri"]
                .as_str()
                .and_then(|u| u.strip_prefix("data:"))
                .and_then(|u| u.split_once(";base64,"))
                .map(|(_, data)| data)
                .or_else(|| v["data"].as_str());
            if let Some(data) = data {
                let name = v["name"]
                    .as_str()
                    .unwrap_or("attachment.bin")
                    .chars()
                    .take(255)
                    .collect::<String>();
                let file = if data.len() <= ARCHIVE_FILE_LIMIT.div_ceil(3) * 4 {
                    STANDARD.decode(data).ok().and_then(|bytes| {
                        ArchiveFile::captured("inline".into(), name.clone(), mime.into(), &bytes)
                            .ok()
                    })
                } else {
                    None
                };
                out.push(file.unwrap_or(ArchiveFile {
                    uri: "inline".into(),
                    name,
                    mime: mime.into(),
                    status: "invalid_inline_data_or_over_limit".into(),
                    sha256: None,
                    data: None,
                }));
            }
        }
        match v {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if key != ARCHIVE_FILES_KEY {
                        walk(value, out, depth + 1);
                    }
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    walk(value, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out, 0);
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_digest_is_required_and_corruption_never_exports() {
        let mut file = ArchiveFile::captured(
            "file:///work/a.pdf".into(),
            "a.pdf".into(),
            "application/pdf".into(),
            b"\0\xffPDF",
        )
        .unwrap();
        assert_eq!(file.bytes().unwrap().unwrap(), b"\0\xffPDF");
        file.sha256 = Some("a".repeat(64));
        assert!(file.bytes().is_err());
    }
    #[test]
    fn inline_file_content_is_decoded_but_never_recurses_into_snapshot_metadata() {
        let value = serde_json::json!({"content":[{"type":"file","uri":"data:application/pdf;base64,AP9QREY=","mime":"application/pdf","name":"proof.pdf"}]});
        let files = archive_inline_files(&value);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes().unwrap().unwrap(), b"\0\xffPDF");
        assert!(archive_file_references(&value).is_empty());
        let value = serde_json::json!({ARCHIVE_FILES_KEY:files});
        assert!(archive_inline_files(&value).is_empty());
    }
}
