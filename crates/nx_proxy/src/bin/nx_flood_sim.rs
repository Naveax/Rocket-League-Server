use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use clap::Parser;
use nx_proxy::ProxyConfig;
use tokio::net::UdpSocket;
use tokio::time::MissedTickBehavior;

#[derive(Debug, Parser)]
#[command(name = "nx_flood_sim")]
#[command(
    about = "Local UDP flood simulator for authorized defensive testing (loopback by default)"
)]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value = "127.0.0.1:7000")]
    target: SocketAddr,
    #[arg(long, default_value_t = 10_000)]
    pps: u32,
    #[arg(long, default_value_t = 5)]
    duration_secs: u64,
    #[arg(long, default_value_t = 64)]
    payload_bytes: usize,
    #[arg(long, default_value_t = false)]
    allow_non_local: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.pps == 0 {
        bail!("pps must be > 0");
    }
    if args.duration_secs == 0 {
        bail!("duration_secs must be > 0");
    }
    if args.payload_bytes == 0 {
        bail!("payload_bytes must be > 0");
    }
    let allow_non_local_from_config = if let Some(path) = &args.config {
        ProxyConfig::from_file(path)
            .with_context(|| format!("failed to read flood simulator config {}", path.display()))?
            .flood_sim
            .allow_non_local
    } else {
        false
    };
    let allow_non_local = args.allow_non_local || allow_non_local_from_config;

    if !allow_non_local && !args.target.ip().is_loopback() {
        bail!(
            "refusing non-loopback target {} without --allow-non-local",
            args.target
        );
    }

    let bind_addr = if args.target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind sender socket on {bind_addr}"))?;
    socket
        .connect(args.target)
        .await
        .with_context(|| format!("failed to connect to target {}", args.target))?;

    let payload = vec![0xA5u8; args.payload_bytes];
    let total_packets = (args.pps as u64).saturating_mul(args.duration_secs);
    let tick_nanos = (1_000_000_000u64 / (args.pps as u64)).max(1);
    let mut ticker = tokio::time::interval(Duration::from_nanos(tick_nanos));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut sent = 0u64;
    let mut send_errors = 0u64;
    let started = Instant::now();
    while sent < total_packets {
        ticker.tick().await;
        match socket.send(&payload).await {
            Ok(_) => sent = sent.saturating_add(1),
            Err(_) => send_errors = send_errors.saturating_add(1),
        }
    }
    let elapsed = started.elapsed().as_secs_f64().max(1e-9);
    let achieved_pps = sent as f64 / elapsed;

    println!(
        "nx_flood_sim target={} sent={} errors={} elapsed_s={:.3} achieved_pps={:.0} allow_non_local={}",
        args.target, sent, send_errors, elapsed, achieved_pps, allow_non_local
    );
    Ok(())
}
