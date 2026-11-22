use anyhow::{ensure, Context, Result};
use factory_core::*;
use std::{
    collections::BTreeSet,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};
pub(super) struct Export {
    directory: PathBuf,
    manifest: File,
    seen: BTreeSet<String>,
    pub count: usize,
    pub unavailable: usize,
}
impl Export {
    pub fn new(directory: &Path) -> Result<Self> {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
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
