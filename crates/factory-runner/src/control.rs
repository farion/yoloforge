//! Cooperative cross-platform shutdown without process enumeration or Unix signals.
use crate::Args;
use anyhow::{ensure, Context, Result};
use factory_platform::runner_control::Endpoint;
use std::{io::Write, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

pub struct Control {
    listener: TcpListener,
    endpoint: Endpoint,
    _lock: std::fs::File,
}

pub async fn listen(args: &Args) -> Result<Option<Control>> {
    let Some(path) = &args.control_file else {
        return Ok(None);
    };
    let parent = path
        .parent()
        .context("Control file needs a parent directory")?;
    factory_platform::private_dir(parent)?;
    let lock_path = path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    ensure!(
        fs2::FileExt::try_lock_exclusive(&lock).is_ok(),
        "Another desktop runner owns this control file"
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let journal = args
        .journal
        .as_ref()
        .context("Runner control requires a journal")?;
    let journal = if journal.is_absolute() {
        journal.clone()
    } else {
        std::env::current_dir()?.join(journal)
    };
    let endpoint = Endpoint {
        port: listener.local_addr()?.port(),
        secret: format!(
            "{}{}",
            factory_core::RequestId::new(),
            factory_core::RequestId::new()
        ),
        pid: std::process::id(),
        runner: args.id.to_string(),
        server: args.server.clone(),
        journal,
    };
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec(&endpoint)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(Some(Control {
        listener,
        endpoint,
        _lock: lock,
    }))
}

pub async fn serve(control: Option<Control>) -> Result<()> {
    let Some(control) = control else {
        return std::future::pending().await;
    };
    loop {
        let (stream, _) = control.listener.accept().await?;
        let mut stream = BufReader::new(stream);
        let mut request = String::new();
        // Bound both time and size so a local untrusted peer cannot hold shutdown hostage.
        let read = async {
            use tokio::io::AsyncReadExt;
            (&mut stream).take(128).read_line(&mut request).await
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(1), read).await,
            Ok(Ok(_))
        ) {
            continue;
        }
        let stop = request == format!("{} stop\n", control.endpoint.secret);
        if stop || request == format!("{} ping\n", control.endpoint.secret) {
            let response = format!("OK {}\n", control.endpoint.pid);
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                stream.get_mut().write_all(response.as_bytes()),
            )
            .await;
            if stop {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[tokio::test(flavor = "multi_thread")]
    async fn authenticated_control_stops_only_its_own_runner() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let path = root.join("control.json");
        let args = Args::try_parse_from([
            "factory-runner",
            "--id",
            &factory_core::RunnerId::new().to_string(),
            "--name",
            "test",
            "--journal",
            root.join("runner.sqlite3").to_str().unwrap(),
            "--control-file",
            path.to_str().unwrap(),
        ])
        .unwrap();
        let control = listen(&args).await.unwrap();
        assert!(listen(&args).await.is_err());
        let mut endpoint: Endpoint = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let task = tokio::spawn(serve(control));
        let endpoint = tokio::task::spawn_blocking(move || {
            endpoint.request("ping").unwrap();
            let real = endpoint.secret.clone();
            endpoint.secret = "0".repeat(72);
            assert!(endpoint.request("stop").is_err());
            endpoint.secret = real;
            endpoint.request("ping").unwrap();
            endpoint.request("stop").unwrap();
            endpoint
        })
        .await
        .unwrap();
        task.await.unwrap().unwrap();
        assert!(endpoint.request("ping").is_err());
    }
}
