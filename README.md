# RocketLeagueServerFix

Defensive UDP edge mitigation layer and guard components for operator-owned game server deployments.

## Scope
- Provides a vendor-agnostic UDP mitigation/proxy path (`nx_proxy`) with rate limiting, queue backpressure, and optional challenge/cookie gating.
- Includes supporting guard/tooling components in this workspace.
- Intended for deployment by server operators on infrastructure they control.

## Threat Model
- Volumetric DDoS ve server lag mitigation odaklıdır; ani UDP paket patlamalarında servis sürekliliğini korumayı hedefler.
- Epic netcode/server optimizasyon eksikliği kaynaklı volumetrik yüklerde outage riskine karşı operatör tarafında savunma katmanı sağlar.
- Uygulanan kontroller: per-IP token bucket, bounded queues, anomaly scoring (heuristic/Torch model).
- Daha gelişmiş model isteyen operatörler için TorchScript LSTM/autoencoder modelini `anomaly.model = "torch"` ve `anomaly.torch_model_path` ile bağlayabilir.

## Out Of Scope
- Patching or controlling third-party ranked/matchmaking infrastructure.
- Offensive tooling, attack simulation against third-party services, or exploit development.

## Quick Start

### Rust Workspace
```bash
cargo build --workspace
cargo test --all
```

Run the UDP proxy:
```bash
cargo run -p nx_proxy -- --config config/dev.toml
```

### CMake Guard Build
```bash
cmake -S . -B build
cmake --build build -j
ctest --test-dir build --output-on-failure
```

Direct test binaries:
```bash
./build/guard_tests
./build/guard_property_tests
```

## Configuration
- `config/example.toml`: safer production-oriented template.
- `config/dev.toml`: local development defaults.
- DDoS token bucket shortcut: `[rate_limit].ddos_limit = 500.0` (alias for per-IP packets/sec).
- Spike detection: `[anomaly]` section with `enabled`, `anomaly_threshold`, and `model`.

Example:
```bash
cargo run -p nx_proxy -- --config config/example.toml
```

Local flood simulation (authorized local testing only):
```bash
cargo run -p nx_proxy --bin nx_flood_sim -- --target 127.0.0.1:7000 --pps 10000 --duration-secs 5
```

## Security Note
Use this project only on systems and networks you own or are explicitly authorized to operate. Do not run abuse/flood testing against third-party infrastructure.

## Documentation
- Build notes: `docs/Build.md`
- Architecture: `docs/ARCHITECTURE.md`
- Deployment: `docs/DEPLOYMENT.md`
