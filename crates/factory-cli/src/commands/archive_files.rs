use anyhow::{ensure, Context, Result};
use factory_core::*;
use std::{
    collections::BTreeSet,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};
pub(crate) struct Export {
    directory: PathBuf,
    manifest: File,
    seen: BTreeSet<String>,
    pub count: usize,
    pub unavailable: usize,
}
impl Export {
    pub async fn supplemental(
        &mut self,
        client: &factory_protocol::client::FactoryClient,
        id: CommandId,
        through: Option<u64>,
        message_id: Option<&str>,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};
        let index = match client.archive_files(id, through).await {
            Ok(index) => index,
            Err(factory_protocol::client::ClientError::Http { status, .. })
                if status == reqwest::StatusCode::NOT_FOUND =>
            {
                return Ok(())
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(index.files.len() <= 16, "Invalid supplemental file index");
        for file in index.files {
            if message_id.is_some_and(|m| m != file.message_id) {
                continue;
            }
            ensure!(
                file.sha256.len() == 64
                    && file
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && file.bytes > 0
                    && file.bytes <= ARCHIVE_BLOB_LIMIT,
                "Invalid supplemental file metadata"
            );
            if !self.seen.contains(&file.sha256) {
                let mut output = tempfile::NamedTempFile::new_in(&self.directory)?;
                let mut offset = 0;
                let mut digest = Sha256::new();
                while offset < file.bytes {
                    let chunk = client.archive_file_chunk(id, &file.sha256, offset).await?;
                    ensure!(
                        chunk.file == file && chunk.offset == offset,
                        "Supplemental file identity changed"
                    );
                    let bytes = chunk.bytes().map_err(anyhow::Error::msg)?;
                    ensure!(
                        offset + bytes.len() as u64 <= file.bytes,
                        "Supplemental file exceeds declared size"
                    );
                    output.write_all(&bytes)?;
                    digest.update(&bytes);
                    offset += bytes.len() as u64;
                }
                ensure!(
                    format!("{:x}", digest.finalize()) == file.sha256,
                    "Supplemental file hash mismatch"
                );
                output.as_file().sync_all()?;
                output
                    .persist_noclobber(self.directory.join(&file.sha256))
                    .map_err(|e| e.error)?;
                self.seen.insert(file.sha256.clone());
                self.count += 1;
            }
            writeln!(
                self.manifest,
                "{}",
                serde_json::json!({"status":"captured_supplement","file":file.sha256,"metadata":file})
            )?;
        }
        Ok(())
    }
    pub fn new(directory: &Path) -> Result<Self> {
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder
            .create(directory)
            .context("Attachment export directory must not already exist")?;
        Ok(Self {
            directory: directory.into(),
            manifest: super::private_file(&directory.join("manifest.jsonl"))?,
            seen: BTreeSet::new(),
            count: 0,
            unavailable: 0,
        })
    }
    pub fn entry(&mut self, entry: &ArchivedMessage) -> Result<()> {
        let refs = archive_file_references(&entry.message);
        let mut files: Vec<ArchiveFile> = if let Some(value) = entry.message.get(ARCHIVE_FILES_KEY)
        {
            ensure!(
                value.as_array().is_some_and(|a| a.len() <= 16),
                "Invalid archived attachment index"
            );
            serde_json::from_value(value.clone())?
        } else {
            Vec::new()
        };
        self.unavailable += refs.len().saturating_sub(files.len());
        files.extend(archive_inline_files(&entry.message));
        for file in files {
            let bytes = file.bytes().map_err(anyhow::Error::msg)?;
            let filename = if let Some(bytes) = bytes {
                let hash = file.sha256.as_ref().context("Archive file hash missing")?;
                // A verified digest is the filename; upstream names never become paths.
                if self.seen.insert(hash.clone()) {
                    let mut output = super::private_file(&self.directory.join(hash))?;
                    output.write_all(&bytes)?;
                    output.sync_all()?;
                    self.count += 1;
                }
                Some(hash.clone())
            } else {
                self.unavailable += 1;
                None
            };
            writeln!(
                self.manifest,
                "{}",
                serde_json::json!({"message_id":entry.message["id"],"sample_revision":entry.sample_revision,"uri":file.uri,"name":file.name,"mime":file.mime,"status":file.status,"sha256":file.sha256,"file":filename})
            )?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        self.manifest.sync_all()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_deduplicates_hashes_and_never_uses_untrusted_filenames() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("files");
        let mut export = Export::new(&dest).unwrap();
        let file = ArchiveFile::captured(
            "file:///work/file".into(),
            "../../escape".into(),
            "application/octet-stream".into(),
            b"\0binary",
        )
        .unwrap();
        let message = serde_json::json!({"id":"msg_test",ARCHIVE_FILES_KEY:[file.clone()]});
        let entry = ArchivedMessage {
            sequence: 1,
            sample_revision: 1,
            captured_unix_ms: 1,
            message,
        };
        export.entry(&entry).unwrap();
        export.entry(&entry).unwrap();
        export.finish().unwrap();
        assert_eq!(export.count, 1);
        assert_eq!(
            std::fs::read(dest.join(file.sha256.unwrap())).unwrap(),
            b"\0binary"
        );
        assert!(!root.path().join("escape").exists());
        assert!(Export::new(&dest).is_err());
    }
}
