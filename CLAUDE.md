# CLAUDE.md — Stoat Backend

This file provides guidance for AI assistants working on the Stoat backend codebase (the services and libraries that power the Revolt chat service).

## Project Overview

Stoat is a Rust-based backend for the Revolt chat platform. It is structured as a Cargo workspace containing multiple services, daemons, and shared core libraries. The codebase is licensed AGPL-3.0-or-later for service crates and MIT for core library crates.

**Minimum Supported Rust Version:** 1.92.0 (pinned via `rust-toolchain.toml`)

---

## Repository Structure

```
stoatchat/
├── crates/
│   ├── delta/               # REST API server (port 14702)
│   ├── bonfire/             # WebSocket events server (port 14703)
│   ├── core/
│   │   ├── config/          # Configuration loading (revolt-config)
│   │   ├── database/        # Database abstraction layer (revolt-database)
│   │   ├── files/           # S3 and encryption subroutines (revolt-files)
│   │   ├── models/          # API models (revolt-models)
│   │   ├── permissions/     # Permission logic (revolt-permissions)
│   │   ├── presence/        # User presence tracking (revolt-presence)
│   │   ├── result/          # Result and Error types (revolt-result)
│   │   ├── coalesced/       # Coalescion service (revolt-coalesced)
│   │   ├── parser/          # Revolt markup parser (revolt-parser)
│   │   └── ratelimits/      # Rate limiting (revolt-ratelimits)
│   ├── services/
│   │   ├── autumn/          # File upload server (port 14704)
│   │   ├── january/         # URL proxy server (port 14705)
│   │   └── gifbox/          # Tenor GIF proxy (port 14706)
│   └── daemons/
│       ├── crond/           # Scheduled cleanup daemon
│       ├── pushd/           # Push notification daemon
│       └── voice-ingress/   # Voice ingress daemon
├── docs/                    # Docusaurus documentation site
├── scripts/                 # Build and utility scripts
├── .mise/                   # Mise task runner configuration
│   └── tasks/               # Task definitions (build, test, service:*, docker:*, docs:*)
├── compose.yml              # Docker Compose for development infrastructure
├── Revolt.toml              # Default development configuration
├── Cargo.toml               # Workspace manifest
├── clippy.toml              # Clippy lint configuration
├── deny.toml                # cargo-deny dependency audit config
└── rust-toolchain.toml      # Pinned Rust toolchain version
```

---

## Tech Stack

| Layer | Technology |
|---|---|
| Language | Rust (stable 1.92.0) |
| HTTP framework (delta) | Rocket |
| HTTP framework (services) | Axum |
| WebSocket server | async-std + tungstenite |
| Database | MongoDB (production), Reference/in-memory (testing) |
| Cache / Pub-Sub | Redis (KeyDB in dev) |
| Message Broker | RabbitMQ (AMQP) |
| File Storage | S3-compatible (MinIO in dev) |
| Authentication | authifier crate |
| Task Runner | mise |
| Testing | cargo-nextest |
| Observability | OpenTelemetry + tracing |

---

## Development Environment Setup

### Prerequisites

- [mise](https://mise.jdx.dev/) — tool version manager and task runner
- Docker — for infrastructure services
- Git
- mold (optional, speeds up linking)

> Nix users: run `nix-shell` to activate mise automatically.

### First-time Setup

```bash
git clone <repo> revolt-backend
cd revolt-backend
mise build          # install tools and build all crates
```

### Starting Infrastructure

```bash
mise docker:start   # starts MongoDB, Redis, MinIO, RabbitMQ, Maildev, LiveKit
mise docker:stop    # stops all containers
```

### Running Services

```bash
mise service:api     # REST API (delta) on :14702
mise service:events  # WebSocket server (bonfire) on :14703
mise service:files   # File server (autumn) on :14704
mise service:proxy   # URL proxy (january) on :14705
mise service:gifbox  # GIF proxy on :14706
mise service:crond   # Cron daemon
mise service:pushd   # Push notification daemon
mise start           # Start Docker + all services
```

Or run individual binaries directly:
```bash
cargo run --bin revolt-delta
cargo run --bin revolt-bonfire
cargo run --bin revolt-autumn
cargo run --bin revolt-january
cargo run --bin revolt-gifbox
cargo run --bin revolt-crond
cargo run --bin revolt-pushd
```

### Port Reference

| Service | Port |
|---|---|
| MongoDB | 27017 |
| Redis | 6379 |
| MinIO | 14009 |
| Maildev SMTP | 14025 |
| Maildev Web UI | 14080 |
| Revolt Web App | 14701 |
| delta (REST API) | 14702 |
| bonfire (WebSocket) | 14703 |
| autumn (files) | 14704 |
| january (proxy) | 14705 |
| gifbox (GIF proxy) | 14706 |
| RabbitMQ AMQP | 5672 |
| RabbitMQ Management | 15672 |

---

## Building

```bash
mise build                        # debug build
mise build --release              # release build
cargo build --bin revolt-delta    # build specific binary
```

---

## Testing

Tests use **cargo-nextest** with two database backends:

```bash
mise test                                    # reference (in-memory) DB
TEST_DB=REFERENCE mise test                  # explicitly use reference DB
TEST_DB=MONGODB MONGODB=mongodb://localhost mise test  # use real MongoDB
```

In CI, both `REFERENCE` and `MONGODB` backends are tested on every push.

---

## Code Quality

```bash
mise check          # runs cargo clippy
cargo clippy        # lint the workspace
cargo deny check    # audit dependencies for license/security issues
```

### Clippy Configuration (`clippy.toml`)

Certain lower-level database methods are disallowed to enforce higher-level APIs:

- **Use `Object::create()`** instead of raw `insert_*` ops (e.g. `insert_bot`, `insert_channel`, `insert_message`, etc.)
- **Use `Object::update(&self)`** instead of raw `update_*` ops
- **Use `Object::delete(&self)`** instead of raw `delete_*` ops
- Do not call `Bot::remove_field`, `Message::attach_sendable_embed`, `User::set_relationship`, or `User::apply_relationship` directly

---

## Configuration

The main config file is `Revolt.toml` (development defaults). To override settings without modifying the tracked file, create `Revolt.overrides.toml`:

```toml
# Revolt.overrides.toml — local overrides only, gitignored
[sentry]
api = "https://..."

[api.smtp]
host = "your-smtp-host"
```

Key configuration sections:
- `[database]` — MongoDB and Redis URLs
- `[rabbit]` — RabbitMQ connection
- `[hosts]` — Public URLs for each service
- `[api.smtp]` — Email/verification settings
- `[api.livekit]` — Voice server nodes
- `[files.s3]` — S3/MinIO settings

---

## Architecture Patterns

### Database Abstraction

`revolt-database` provides a trait-based abstraction over database backends:

- **`Database` enum** — wraps either `ReferenceDb` (in-memory, for tests) or `MongoDb`
- **`Abstract*` traits** — e.g. `AbstractUsers`, `AbstractChannels` — define all DB operations as async trait methods
- Each model in `crates/core/database/src/models/<entity>/` has:
  - `model.rs` — struct definition using `auto_derived_partial!` macro
  - `ops.rs` / `ops/` — abstract trait + implementations for each backend
  - `axum.rs` / `rocket.rs` — HTTP-layer implementations (where applicable)

### Model Conventions

- IDs use ULID format stored as strings
- MongoDB documents use `_id` field (mapped via `#[serde(rename = "_id")]`)
- Optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`
- The `auto_derived_partial!` macro generates a `Partial<Model>` type for partial updates
- `FieldsModel` enums enumerate which fields can be cleared/removed

### Error Handling

`revolt-result` defines the shared `Result<T>` and `Error` types:
- `Error` has an `error_type: ErrorType` (tagged enum) and a `location: String`
- Use `create_error!` macro to construct typed errors with source location
- HTTP adapters for Rocket (`revolt_result::rocket`) and Axum (`revolt_result::axum`) convert errors to HTTP responses automatically

### Events System

Real-time events flow through:
1. **RabbitMQ (AMQP)** — services publish `EventV1` messages
2. **bonfire** — WebSocket server consumes events and forwards to connected clients
3. `EventV1` enum covers all event types (message create/update/delete, channel updates, server events, auth events, etc.)

### Permission System

`revolt-permissions` implements permission checking. Always use the provided permission calculation functions rather than checking raw bitfields manually.

---

## Documentation Site

The `docs/` directory contains a Docusaurus site:

```bash
mise docs:install   # install pnpm dependencies
mise docs           # start dev server
mise docs:build     # build static site
```

---

## CI/CD

GitHub Actions workflows (`.github/workflows/`):

| Workflow | Trigger | Purpose |
|---|---|---|
| `rust.yaml` | push to main, PRs | Build, lint, test (Reference + MongoDB), generate OpenAPI spec |
| `docker.yaml` | releases | Build and push Docker images |
| `publish-crates.yml` | releases | Publish core crates to crates.io |
| `release-please.yml` | push to main | Automated release PR management |
| `docs.yml` | push to main | Deploy documentation site |
| `validate-pr-title.yml` | PRs | Enforce conventional commit PR titles |

On merge to `main`, the API server is started and the OpenAPI specification is automatically exported and committed to the `javascript-client-api` repository.

---

## Docker Images

Each service has its own `Dockerfile` (`crates/<service>/Dockerfile`). Images are published to `ghcr.io/stoatchat/`:

- `ghcr.io/stoatchat/server` — delta (REST API)
- `ghcr.io/stoatchat/bonfire` — WebSocket server
- `ghcr.io/stoatchat/autumn` — file server
- `ghcr.io/stoatchat/january` — proxy server
- `ghcr.io/stoatchat/gifbox` — GIF proxy
- `ghcr.io/stoatchat/crond` — cron daemon
- `ghcr.io/stoatchat/pushd` — push daemon

The `Dockerfile` at the root uses a two-stage build with stub source files to cache dependency compilation separately from application code (see `scripts/build-image-layer.sh`).

---

## Key Conventions

1. **Prefer `Object::create()` over raw insert ops** — the high-level methods handle event emission and other side effects.
2. **Prefer `Object::update(&self)` over raw update ops** — similarly handles event dispatch.
3. **Use `revolt_result::Result`** everywhere for error propagation; never use `unwrap()` in production paths.
4. **Feature flags** — `revolt-database` uses Cargo features (`mongodb`, `axum-impl`, `rocket-impl`) to gate backend-specific code; keep feature-gated code behind the appropriate `#[cfg(feature = "...")]`.
5. **No direct field mutation for relationship tracking** — use the provided methods on `User` for relationship management.
6. **Configuration overrides** — never commit `Revolt.overrides.toml`; it is gitignored.
7. **compose.override.yml** — use for local Docker port overrides; also gitignored.
8. **Conventional commits** — PR titles must follow the conventional commits specification (enforced by CI).

---

## Useful References

- Contribution guidelines: https://developers.revolt.chat/contrib.html
- Technical documentation: https://revoltchat.github.io/backend/
- Revolt self-hosted setup: https://github.com/revoltchat/self-hosted
