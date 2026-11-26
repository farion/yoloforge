use anyhow::{ensure, Context, Result};
use clap::Parser;
use std::{env, net::SocketAddr, path::PathBuf};
mod managed_opencode;

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
    let mut mcp_key = args.database.as_os_str().to_os_string();
    mcp_key.push(".mcp-key");
    let mcp_gateway = factoryd::remote_mcp::Gateway::open(
        &PathBuf::from(mcp_key),
        store
            .has_remote_mcp_key()
            .map_err(|_| anyhow::anyhow!("Cannot check remote MCP vault"))?,
    )?;
    let mcp_gateway = match env::var("FACTORY_MCP_PUBLIC_URL") {
        Ok(origin) => mcp_gateway.with_oauth_origin(&origin)?,
        Err(env::VarError::NotPresent) => mcp_gateway,
        Err(error) => return Err(error.into()),
    };
    if token.is_empty() {
        ensure!(
            store
                .devices()
                .map_err(|_| anyhow::anyhow!("Cannot read device registry"))?
                .iter()
                .any(|d| d.role == factory_core::DeviceRole::Admin && !d.revoked),
            "An initial FACTORY_TOKEN_FILE or active admin device is required"
        );
    }
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    let (gui_closed, mut gui_shutdown) = tokio::sync::oneshot::channel();
    factory_platform::watch_gui_lifetime(move || { let _ = gui_closed.send(()); })?;
    let mut gui_shutdown = Box::pin(async move {
        if (&mut gui_shutdown).await.is_err() {
            std::future::pending::<()>().await;
        }
    });
    let binary = env::var_os("FACTORY_OPENCODE_BIN").map(PathBuf::from);
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut shutdown = Box::pin(async move {
        #[cfg(unix)]
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        #[cfg(windows)]
        let _ = tokio::signal::ctrl_c().await;
    });
    let (mut opencode, gateway) = tokio::select! {
        result = managed_opencode::Runtime::start(&args.database, binary.as_deref()) => result?,
        _ = &mut shutdown => return Ok(()),
        _ = &mut gui_shutdown => return Ok(()),
    };
    store
        .enable_model_settings()
        .map_err(|_| anyhow::anyhow!("Could not initialize Factory model settings"))?;
    println!(
        "factoryd {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let android_push = {
        let (store, stopping, token) = (store.clone(), stopping.clone(), token.clone());
        tokio::spawn(async move { factoryd::android_push::run(store, stopping, &token).await })
    };
    let push = push_config
        .map(|config| tokio::spawn(factoryd::push::run(store.clone(), config, stopping.clone())));
    let supervisor_store = store.clone();
    let scheduler_wakeup = store.scheduler_wakeup();
    let supervisor_stopping = stopping.clone();
    let supervisor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let periodic = tokio::select! {
                _ = interval.tick() => true,
                _ = scheduler_wakeup.notified() => false,
            };
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
                    store.merger_tick()?;
                    if periodic {
                        store.supervisor_tick(now)?;
                    }
                    store.scheduler_tick()
                })
                .await,
                Ok(Ok(()))
            ) {
                eprintln!("Orchestration tick failed; durable state retained for retry");
            }
        }
    });
    let stopping_on_signal = stopping.clone();
    let serving = axum::serve(
        listener,
        factoryd::router_with_model_gateway(&token, store, stopping.clone(), Some(gateway))
            .layer(axum::Extension(mcp_gateway))
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown.await;
        stopping_on_signal.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let result = tokio::select! {
        result = std::future::IntoFuture::into_future(serving) => result,
        _ = &mut gui_shutdown => {
            stopping.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        },
        _ = opencode.supervise() => unreachable!("OpenCode supervisor never returns"),
    };
    opencode.shutdown().await;
    supervisor.abort();
    android_push.abort();
    if let Some(push) = push {
        push.abort();
    }
    Ok(result?)
}
