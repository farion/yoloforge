use anyhow::{ensure, Context, Result};
use clap::Parser;
use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Factory control service with SQLite project and ticket storage"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7432")]
    listen: SocketAddr,
    #[arg(long, default_value = ".factory-data/factory.sqlite3")]
    database: PathBuf,
    /// Permit a container interface bind. Publish its port on host loopback only.
    #[arg(long)]
    allow_container_network: bool,
    /// Optional private ntfy configuration for durable background question notifications.
    #[arg(long)]
    push_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let push_config = args
        .push_config
        .as_deref()
        .map(factoryd::push::Config::load)
        .transpose()?;
    let token = match (env::var("FACTORY_TOKEN"), env::var_os("FACTORY_TOKEN_FILE")) {
        (Ok(_), Some(_)) => anyhow::bail!("Set only FACTORY_TOKEN or FACTORY_TOKEN_FILE"),
        (Ok(token), None) => token,
        (Err(_), Some(path)) => std::fs::read_to_string(path)
            .context("Cannot read FACTORY_TOKEN_FILE")?
            .trim_end_matches(['\n', '\r'])
            .to_string(),
        _ => String::new(),
    };
    ensure!(
        token.is_empty() || (token.len() >= 32 && token.bytes().all(|b| b.is_ascii_graphic())),
        "FACTORY_TOKEN must be at least 32 printable ASCII bytes"
    );
    ensure!(
        args.listen.ip().is_loopback() || args.allow_container_network,
        "foundation server requires loopback binding; use an SSH tunnel"
    );
    let store = factoryd::store::Store::open(&args.database)
        .context("cannot initialize Factory database")?;
    if token.is_empty() {
        ensure!(
            store
                .devices()
                .map_err(|_| anyhow::anyhow!("Cannot read device registry"))?
                .iter()
                .any(|d| d.role == factory_core::DeviceRole::Admin && !d.revoked),
            "An initial FACTORY_TOKEN_FILE or enrolled admin device is required"
        );
    }
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    println!(
        "factoryd {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let push = push_config
        .map(|config| tokio::spawn(factoryd::push::run(store.clone(), config, stopping.clone())));
    let supervisor_store = store.clone();
    let supervisor_stopping = stopping.clone();
    let supervisor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            if supervisor_stopping.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let store = supervisor_store.clone();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if !matches!(
                tokio::task::spawn_blocking(move || {
                    store.supervisor_tick(now)?;
                    store.scheduler_tick()
                })
                .await,
                Ok(Ok(()))
            ) {
                eprintln!("Orchestration tick failed; durable state retained for retry");
            }
        }
    });
    axum::serve(
        listener,
        factoryd::router_with_shutdown(&token, store, stopping.clone()),
    )
    .with_graceful_shutdown(async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("signal handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        stopping.store(true, std::sync::atomic::Ordering::Relaxed);
    })
    .await?;
    supervisor.abort();
    if let Some(push) = push {
        push.abort();
    }
    Ok(())
}
