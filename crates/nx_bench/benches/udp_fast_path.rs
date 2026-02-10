use std::net::UdpSocket;
use std::net::{IpAddr, Ipv4Addr};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use nx_netio::{MsgBuf, RecvBatchState};
use nx_proxy::anomaly::AnomalyDetector;
use nx_proxy::config::{AnomalyModel, AnomalySection, RateLimitSection};

const PACKETS: usize = 20_000;
const BATCH: usize = 32;
const ANOMALY_SAMPLES: usize = 20_000;

fn main() {
    let fallback_pps = bench_fallback_pps();
    println!("fallback_recv_from_pps={fallback_pps:.0}");

    #[cfg(target_os = "linux")]
    {
        let mmsg_pps = bench_mmsg_pps();
        println!("mmsg_recvmmsg_pps={mmsg_pps:.0}");
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("mmsg_recvmmsg_pps=unsupported");
    }

    let (p50_us, p99_us) = bench_queue_latency_us();
    println!("enqueue_forward_latency_p50_us={p50_us:.2}");
    println!("enqueue_forward_latency_p99_us={p99_us:.2}");

    let (anomaly_p50_us, anomaly_p99_us, anomaly_drop_ratio) = bench_anomaly_latency_us();
    println!("anomaly_latency_p50_us={anomaly_p50_us:.2}");
    println!("anomaly_latency_p99_us={anomaly_p99_us:.2}");
    println!("anomaly_drop_ratio={anomaly_drop_ratio:.4}");
}

fn bench_fallback_pps() -> f64 {
    let recv_socket = UdpSocket::bind("127.0.0.1:0").expect("bind recv");
    let recv_addr = recv_socket.local_addr().expect("recv addr");
    let send_socket = UdpSocket::bind("127.0.0.1:0").expect("bind send");
    send_socket.connect(recv_addr).expect("connect send");

    let payload = [0u8; 64];
    let mut recv_buf = [0u8; 1500];

    let start = Instant::now();
    for _ in 0..PACKETS {
        send_socket.send(&payload).expect("send");
        let _ = recv_socket.recv_from(&mut recv_buf).expect("recv");
    }
    let elapsed = start.elapsed().as_secs_f64();
    PACKETS as f64 / elapsed.max(1e-9)
}

#[cfg(target_os = "linux")]
fn bench_mmsg_pps() -> f64 {
    let recv_socket = UdpSocket::bind("127.0.0.1:0").expect("bind recv");
    let recv_addr = recv_socket.local_addr().expect("recv addr");
    let send_socket = UdpSocket::bind("127.0.0.1:0").expect("bind send");
    send_socket.connect(recv_addr).expect("connect send");

    let payload = [0u8; 64];
    let mut msg_bufs = (0..BATCH)
        .map(|_| MsgBuf::with_capacity(1500))
        .collect::<Vec<_>>();
    let mut recv_state = RecvBatchState::new(BATCH);
    let recv_fd = recv_socket.as_raw_fd();

    let start = Instant::now();
    let mut received = 0usize;
    while received < PACKETS {
        let to_send = (PACKETS - received).min(BATCH);
        for _ in 0..to_send {
            send_socket.send(&payload).expect("send");
        }

        let mut got = 0usize;
        while got < to_send {
            let n = nx_netio::recv_batch_with_state(recv_fd, &mut msg_bufs, &mut recv_state)
                .expect("recvmmsg");
            got += n;
        }
        received += got;
    }

    let elapsed = start.elapsed().as_secs_f64();
    PACKETS as f64 / elapsed.max(1e-9)
}

fn bench_queue_latency_us() -> (f64, f64) {
    let (tx, rx) = flume::bounded::<Instant>(1024);
    let (ack_tx, ack_rx) = flume::bounded::<Instant>(1024);

    let worker = std::thread::spawn(move || {
        while let Ok(sent_at) = rx.recv() {
            let _ = ack_tx.send(sent_at);
        }
    });

    let mut latencies_us = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let sent_at = Instant::now();
        tx.send(sent_at).expect("queue send");
        let echoed = ack_rx.recv().expect("queue recv");
        latencies_us.push(echoed.elapsed().as_secs_f64() * 1_000_000.0);
    }

    drop(tx);
    let _ = worker.join();

    latencies_us.sort_by(|a, b| a.partial_cmp(b).expect("valid float compare"));
    let p50 = percentile(&latencies_us, 0.50);
    let p99 = percentile(&latencies_us, 0.99);
    (p50, p99)
}

fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let idx = ((samples.len() - 1) as f64 * p).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

fn bench_anomaly_latency_us() -> (f64, f64, f64) {
    let mut detector = AnomalyDetector::new(&anomaly_cfg(), &rate_cfg());
    let src = IpAddr::V4(Ipv4Addr::new(10, 77, 0, 1));
    let base = Instant::now();
    let mut latencies_us = Vec::with_capacity(ANOMALY_SAMPLES);
    let mut drops = 0usize;

    for i in 0..ANOMALY_SAMPLES {
        let now = base + Duration::from_millis((i / 20) as u64);
        let packet_len = if i % 13 == 0 { 1200 } else { 256 };
        let start = Instant::now();
        if detector.check_anomaly(src, packet_len, now).is_some() {
            drops = drops.saturating_add(1);
        }
        latencies_us.push(start.elapsed().as_secs_f64() * 1_000_000.0);
    }

    latencies_us.sort_by(|a, b| a.partial_cmp(b).expect("valid float compare"));
    let p50 = percentile(&latencies_us, 0.50);
    let p99 = percentile(&latencies_us, 0.99);
    let drop_ratio = drops as f64 / ANOMALY_SAMPLES as f64;
    (p50, p99, drop_ratio)
}

fn anomaly_cfg() -> AnomalySection {
    AnomalySection {
        enabled: true,
        model: AnomalyModel::Heuristic,
        anomaly_threshold: 0.80,
        ddos_limit: 500.0,
        window_millis: 200,
        ema_alpha: 0.35,
        min_packets_per_window: 8,
        max_tracked_ips: 1024,
        idle_timeout_secs: 60,
        torch_model_path: None,
    }
}

fn rate_cfg() -> RateLimitSection {
    RateLimitSection {
        per_ip_packets_per_second: 500.0,
        per_ip_burst_packets: 1_000.0,
        per_ip_bytes_per_second: 500_000.0,
        per_ip_burst_bytes: 1_000_000.0,
        global_packets_per_second: 50_000.0,
        global_burst_packets: 100_000.0,
        global_bytes_per_second: 128_000_000.0,
        global_burst_bytes: 256_000_000.0,
        subnet_enabled: false,
        subnet_ipv4_prefix: 24,
        subnet_ipv6_prefix: 64,
        subnet_packets_per_second: 8_000.0,
        subnet_burst_packets: 16_000.0,
        subnet_bytes_per_second: 64_000_000.0,
        subnet_burst_bytes: 128_000_000.0,
        max_ip_buckets: 1024,
        max_subnet_buckets: 256,
        idle_timeout_secs: 120,
    }
}
