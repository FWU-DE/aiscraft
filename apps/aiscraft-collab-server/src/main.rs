//! `aiscraft-collab-server`: the relay for live collaboration in AIS.ChatCraft.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Relay for live collaboration on AIS.ChatCraft documents and whiteboards")]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "COLLAB_LISTEN", default_value = "127.0.0.1:47910")]
    listen: String,
    /// Data directory (rooms, snapshots, logs).
    #[arg(long, env = "COLLAB_DATA", default_value = "./collab-data")]
    data: PathBuf,
    /// Key needed to make rooms (`Authorization: Bearer …`). Required unless --open-create.
    /// Pass it by environment, not on the command line.
    #[arg(long, env = "COLLAB_CREATE_KEY", hide_env_values = true)]
    create_key: Option<String>,
    /// Run without a create key: anyone who reaches the server can make rooms.
    #[arg(long, env = "COLLAB_OPEN_CREATE")]
    open_create: bool,
    #[arg(long, env = "COLLAB_MAX_ROOMS", default_value_t = 5000)]
    max_rooms: usize,
    /// Largest room in MiB.
    #[arg(long, env = "COLLAB_MAX_ROOM_MB", default_value_t = 32)]
    max_room_mb: usize,
    #[arg(long, env = "COLLAB_MAX_PEERS", default_value_t = 64)]
    max_peers: usize,
    /// Delete rooms nobody opened for this many days (0 = never).
    #[arg(long, env = "COLLAB_IDLE_DAYS", default_value_t = 180)]
    idle_days: u64,
    /// Take client addresses from X-Forwarded-For (behind Traefik, Caddy, nginx).
    #[arg(long, env = "COLLAB_TRUST_PROXY")]
    trust_proxy: bool,
    /// Reverse proxies in front that append to X-Forwarded-For; the client is that many entries
    /// from the right.
    #[arg(long, env = "COLLAB_PROXY_HOPS", default_value_t = 1)]
    proxy_hops: usize,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let mut cfg = aiscraft_collab_server::Config::new(cli.data);
    cfg.create_key = cli.create_key.filter(|k| !k.trim().is_empty());
    cfg.max_rooms = cli.max_rooms;
    cfg.max_room_bytes = cli.max_room_mb.clamp(1, 512) * 1024 * 1024;
    cfg.max_peers = cli.max_peers.max(1);
    cfg.idle_days = cli.idle_days;
    cfg.trust_proxy = cli.trust_proxy;
    cfg.proxy_hops = cli.proxy_hops;
    cfg.open_create = cli.open_create;
    let open = cfg.create_key.is_none();
    let listener = match tokio::net::TcpListener::bind(&cli.listen).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("event=bind_failed addr={} error={e}", cli.listen);
            return std::process::ExitCode::FAILURE;
        }
    };
    let running = match aiscraft_collab_server::start(listener, cfg).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("event=start_failed error={e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    log::info!("event=listening addr={} open={open}", running.addr);
    wait_for_signal().await;
    log::info!("event=stopping");
    running.shutdown().await;
    std::process::ExitCode::SUCCESS
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
