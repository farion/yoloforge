use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_protocol::{client::FactoryClient, ReportArchiveBlob};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pending {
    pub file: ArchiveBlob,
    pub offset: u64,
    path: PathBuf,
}

fn spool(
    uri: &str,
    roots: &[PathBuf],
    directory: &Path,
    budget: u64,
) -> Result<(String, u64, PathBuf)> {
    let url = reqwest::Url::parse(uri)?;
    ensure!(
        url.scheme() == "file" && url.host_str().is_none_or(|h| h == "localhost"),
        "Only explicitly rooted local sources are read"
    );
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid file URI"))?
        .canonicalize()?;
    ensure!(
        roots.iter().any(|r| path.starts_with(r)),
        "Source outside allowed roots"
    );
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)?;
    let before = source.metadata()?;
    ensure!(
        before.is_file() && before.len() > 0 && before.len() <= ARCHIVE_BLOB_LIMIT.min(budget),
        "Source exceeds blob budget or is not a regular file"
    );
    let opened = std::fs::canonicalize(format!("/proc/self/fd/{}", source.as_raw_fd()))?;
    ensure!(
        roots.iter().any(|r| opened.starts_with(r)),
        "Opened source outside allowed roots"
    );
    std::fs::create_dir_all(directory)?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(size <= before.len(), "Source changed while capturing");
        hash.update(&buffer[..n]);
        temp.write_all(&buffer[..n])?;
    }
    let after = source.metadata()?;
    ensure!(
        size == before.len()
            && before.len() == after.len()
            && before.modified()? == after.modified()?,
        "Source changed while capturing"
    );
    let hash = format!("{:x}", hash.finalize());
    let path = directory.join(&hash);
    temp.as_file().sync_all()?;
    if !path.try_exists()? {
        temp.persist_noclobber(&path).map_err(|e| e.error)?;
        std::fs::File::open(directory)?.sync_all()?;
    } else {
        let mut existing = std::fs::File::open(&path)?;
        let mut digest = Sha256::new();
        std::io::copy(&mut existing, &mut digest)?;
        ensure!(
            format!("{:x}", digest.finalize()) == hash,
            "Spool digest differs"
        );
    }
    Ok((hash, size, path))
}
pub fn capture(
    record: &mut Record,
    args: &Args,
    messages: &[serde_json::Value],
    revision: u64,
) -> Result<()> {
    let execution = record.agent.as_ref().context("Execution missing")?;
    let mut roots = args
        .archive_source_root
        .iter()
        .map(|p| p.canonicalize())
        .collect::<std::io::Result<Vec<_>>>()?;
    if let Ok(root) = Path::new(&execution.workspace.path).canonicalize() {
        roots.push(root);
    }
    let journal = args
        .journal
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!(".factory-data/runner-{}.sqlite3", args.id)));
    let directory = journal
        .parent()
        .unwrap_or(Path::new("."))
        .join("archive-blobs")
        .join(record.command.id.to_string());
    let mut budget = (256 * 1024 * 1024u64)
        .saturating_sub(record.archive.blobs.iter().map(|b| b.file.bytes).sum());
    for message in messages {
        let Some(message_id) = message["id"].as_str() else {
            continue;
        };
        for (uri, name, mime) in archive_file_references(message) {
            if uri.chars().any(char::is_control) || mime.chars().any(char::is_control) {
                continue;
            }
            let mut name = name
                .chars()
                .map(|c| if c.is_control() { '_' } else { c })
                .collect::<String>();
            if name.trim().is_empty() {
                name = "attachment.bin".into();
            }
            let mime = if mime.trim().is_empty() {
                "application/octet-stream".into()
            } else {
                mime
            };
            if record.archive.blobs.len() >= 16
                || record
                    .archive
                    .blobs
                    .iter()
                    .any(|b| b.file.uri == uri && b.file.source_revision == revision)
            {
                continue;
            }
            if message[ARCHIVE_FILES_KEY].as_array().is_some_and(|files| {
                files
                    .iter()
                    .any(|f| f["uri"] == uri && f["status"] == "captured")
            }) {
                continue;
            }
            let Ok((sha256, bytes, path)) = spool(&uri, &roots, &directory, budget) else {
                continue;
            };
            if record.archive.blobs.iter().any(|b| b.file.sha256 == sha256) {
                continue;
            }
            budget -= bytes;
            record.archive.blobs.push(Pending {
                file: ArchiveBlob {
                    sha256,
                    bytes,
                    uri,
                    name,
                    mime,
                    message_id: message_id.into(),
                    source_revision: revision,
                    captured_unix_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_millis() as u64,
                },
                offset: 0,
                path,
            });
        }
    }
    Ok(())
}
pub async fn step(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    registration: RegistrationId,
) -> Result<bool> {
    let Some(index) =
        record.archive.blobs.iter().position(|b| {
            b.offset < b.file.bytes && b.file.source_revision <= record.archive.revision
        })
    else {
        return Ok(true);
    };
    let pending = record.archive.blobs[index].clone();
    let bytes = tokio::task::block_in_place(|| -> Result<Vec<u8>> {
        let mut file = std::fs::File::open(&pending.path)?;
        ensure!(
            file.metadata()?.len() == pending.file.bytes,
            "Archive spool size changed"
        );
        file.seek(SeekFrom::Start(pending.offset))?;
        let mut bytes =
            vec![0; (pending.file.bytes - pending.offset).min(ARCHIVE_CHUNK_LIMIT as u64) as usize];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    })?;
    let result = factory
        .report_archive_file(
            record.command.runner_id,
            registration,
            record.command.id,
            &ReportArchiveBlob {
                ownership_epoch: record.command.ownership_epoch,
                chunk: ArchiveBlobChunk::new(pending.file.clone(), pending.offset, &bytes),
            },
        )
        .await?;
    ensure!(
        result.next_offset >= pending.offset + bytes.len() as u64
            && result.next_offset <= pending.file.bytes
            && result.complete == (result.next_offset == pending.file.bytes),
        "Invalid blob receipt"
    );
    record.archive.blobs[index].offset = result.next_offset;
    tokio::task::block_in_place(|| journal.save(record))?;
    Ok(record
        .archive
        .blobs
        .iter()
        .all(|b| b.offset == b.file.bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_and_explicit_external_sources_are_spooled_without_following_escapes() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let path = outside.path().join("large.bin");
        let bytes = vec![0x7f; 5 * 1024 * 1024 + 3];
        std::fs::write(&path, &bytes).unwrap();
        let link = root.path().join("escape");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let uri = reqwest::Url::from_file_path(&link).unwrap().to_string();
        assert!(spool(
            &uri,
            &[root.path().into()],
            destination.path(),
            ARCHIVE_BLOB_LIMIT
        )
        .is_err());
        let (hash, size, stored) = spool(
            &uri,
            &[outside.path().into()],
            destination.path(),
            ARCHIVE_BLOB_LIMIT,
        )
        .unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
        assert_eq!(std::fs::read(&stored).unwrap(), bytes);
        assert!(spool(
            &uri,
            &[outside.path().into()],
            destination.path(),
            4 * 1024 * 1024
        )
        .is_err());
        assert!(spool(
            "http://127.0.0.1/private",
            &[root.path().into()],
            destination.path(),
            ARCHIVE_BLOB_LIMIT
        )
        .is_err());
        std::fs::remove_file(path).unwrap();
        assert_eq!(std::fs::read(stored).unwrap(), bytes);
    }
}
