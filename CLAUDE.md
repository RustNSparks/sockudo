# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Sockudo is a simple, fast, and secure WebSocket server for real-time applications built in Rust. It implements the Pusher protocol with support for horizontal scaling, multiple backend adapters, and real-time communication features.

## Development Commands

### Quick Start
```bash
# Complete setup with Docker
make quick-start

# Development environment
make dev

# Install Git hooks (recommended for all developers)
./scripts/install-hooks.sh

# Run tests
make test
cargo test

# Build from source
cargo build --release

# Run the server
./target/release/sockudo --config config/config.json
```

### Docker Operations
```bash
# Production deployment
make prod

# Scale instances
make scale REPLICAS=3

# View logs
make logs-sockudo

# Health check
make health
```

## Architecture

### Core Module Structure
- `src/adapter/` - Connection management and horizontal scaling (Local, Redis, RedisCluster, NATS)
- `src/app/` - Application management with multiple backend support (Memory, MySQL, PostgreSQL, DynamoDB)
- `src/cache/` - Caching layer (Memory, Redis, RedisCluster)
- `src/channel/` - Channel types and subscription management
- `src/http_handler.rs` - REST API endpoints for event triggering
- `src/websocket.rs` - WebSocket connection handling and message parsing
- `src/webhook/` - Event notification system (HTTP, Lambda)
- `src/queue/` - Background job processing (Memory, Redis, RedisCluster, SQS)
- `src/metrics/` - Prometheus metrics collection
- `src/watchlist/` - Watchlist event management
- `src/protocol/` - Pusher protocol constants and message definitions
- `src/rate_limiter/` - Rate limiting implementation

### Key Design Patterns

**Origin Validation**: Per-app origin validation provides additional WebSocket security by restricting which domains can connect:
```rust
// Example: App config with allowed origins
App {
    id: "my-app".to_string(),
    allowed_origins: Some(vec![
        "https://app.example.com".to_string(),
        "*.staging.example.com".to_string(),
        "http://localhost:3000".to_string()
    ]),
    // ... other fields
}
```

**Adapter Pattern**: All major components (connection management, caching, queuing) use an adapter pattern for backend flexibility:
```rust
// Example: adapter/mod.rs defines trait, implementations in adapter/{local,redis,nats}.rs
pub trait AdapterHandler: Send + Sync {
    async fn subscribe(&self, channel: &str) -> Result<()>;
    async fn unsubscribe(&self, channel: &str) -> Result<()>;
    // ...
}
```

**Configuration Hierarchy** (highest priority wins):
1. Default values (hardcoded)
2. Config file (`config/config.json`)
3. Command-line arguments
4. Environment variables

### WebSocket Protocol

Implements Pusher protocol with extensions:
- **Channel Types**: public, private, presence, private-encrypted
- **Authentication**: HMAC-SHA256 signatures for private/presence channels
- **Events**: Standard pusher events plus client events on private channels

Message format:
```json
{
  "event": "pusher:subscribe",
  "data": {
    "channel": "private-channel",
    "auth": "app_key:signature"
  }
}
```

## Testing Approach

### Running Tests
```bash
# Unit tests
cargo test

# Interactive frontend test
cd test/interactive && npm install && npm start

# Automated integration tests
cd test/automated && npm install && npm test

# Multi-node testing
docker-compose -f docker-compose.multinode.yml up
```

### Test Structure
- `tests/` - Rust unit tests with mocks
- `test/interactive/` - Browser-based WebSocket testing UI
- `test/automated/` - Node.js integration tests using Pusher SDK

## Configuration

### Environment Variables
Key variables (see `.env.example` for complete list):
- `PORT` - Server port (default: 6001)
- `HOST` - Server host (default: 0.0.0.0)
- `ADAPTER_DRIVER` - Connection adapter (local|redis|redis-cluster|nats)
- `APP_MANAGER_DRIVER` - App storage (memory|mysql|postgresql|dynamodb)
- `CACHE_DRIVER` - Cache backend (memory|redis|redis-cluster|none)
- `QUEUE_DRIVER` - Queue backend (memory|redis|redis-cluster|sqs|none)
- `RATE_LIMITER_DRIVER` - Rate limiter backend (memory|redis|redis-cluster|none)
- `DEBUG` or `DEBUG_MODE` - Enable debug logging (DEBUG takes precedence)
- `ENVIRONMENT` - Mode (production/development)
- `REDIS_URL` - Override all Redis configurations with single URL
- `SOCKUDO_DEFAULT_APP_ALLOWED_ORIGINS` - Comma-separated list of allowed origins for default app

#### Logging Configuration
**Environment Variables:**
- `LOG_OUTPUT_FORMAT` - Log format (human|json, default: human) **[Must be set at startup]**
- `LOG_COLORS_ENABLED` - Enable/disable colors in human format (true|false, default: true)
- `LOG_INCLUDE_TARGET` - Include module target in logs (true|false, default: true)

**Config File Options** (in `logging` section):
- `colors_enabled` - Enable/disable colors in human format (true|false, default: true)
- `include_target` - Include module target in logs (true|false, default: true)

**Important**: JSON format (`LOG_OUTPUT_FORMAT=json`) can only be configured via environment variable at startup due to tracing subscriber limitations. It cannot be set in config files.

### Redis/NATS Configuration
- Redis: Set `DATABASE_REDIS_HOST`, `DATABASE_REDIS_PORT`, `DATABASE_REDIS_PASSWORD`
- Redis Cluster: Set `REDIS_CLUSTER_NODES` as comma-separated list
- NATS: Set `NATS_SERVERS` as comma-separated list (e.g., "nats://localhost:4222")

## Development Guidelines

### Adding New Features

1. **New Adapter Implementation**: Create in `src/adapter/`, implement `AdapterHandler` trait
2. **New App Manager**: Create in `src/app/managers/`, implement `AppManagerInterface` trait
3. **New Cache/Queue Driver**: Follow existing patterns in `src/cache/` or `src/queue/`

### Code Conventions

- Use `anyhow::Result` for error handling
- Use `tracing` for logging (not `println!`)
- Keep async functions small and focused
- Use `Arc<RwLock<>>` for shared state across async tasks
- Validate all external inputs (especially WebSocket messages)

### Performance Considerations

- Connection pools are managed automatically for database/Redis connections
- Rate limiting is enforced at both API and WebSocket levels
- Use batch operations when processing multiple channels/connections
- Metrics are exposed at `/metrics` for Prometheus scraping

## Deployment

### Production Checklist
1. Set secure app credentials (`SOCKUDO_DEFAULT_APP_ID`, `SOCKUDO_DEFAULT_APP_KEY`, `SOCKUDO_DEFAULT_APP_SECRET`)
2. Configure SSL certificates (`SSL_CERT_PATH`, `SSL_KEY_PATH`, `SSL_ENABLED=true`)
3. Enable rate limiting (`RATE_LIMITER_ENABLED=true`, `RATE_LIMITER_DRIVER`)
4. Configure webhooks if needed (`WEBHOOK_BATCHING_ENABLED`, `WEBHOOK_BATCHING_DURATION`)
5. Set appropriate limits via app configuration
6. Configure structured logging for external systems (see [Production Logging](#production-logging))

### Production Logging
For production environments with external log aggregation systems (Fluentd, Logstash, etc.):

```bash
# JSON output for parsing-friendly logs (must be set via environment variable)
LOG_OUTPUT_FORMAT=json ./target/release/sockudo

# Human format with no colors (can be set via config file)
LOG_COLORS_ENABLED=false ./target/release/sockudo
```

**Configuration file example:**
```json
{
  "logging": {
    "colors_enabled": false,
    "include_target": true
  }
}
```

**Note**: To use JSON format, you must set `LOG_OUTPUT_FORMAT=json` as an environment variable at startup. JSON format cannot be configured via config files due to technical limitations in the tracing library.

**Benefits of JSON logging:**
- Single-line JSON objects per log entry
- No color codes that interfere with log parsing
- Structured data for better filtering and analysis
- Compatible with log aggregation tools

### Monitoring
- Health endpoint: `GET /up/{app_id}` (WebSocket health check)
- Metrics endpoint: `GET /metrics` (Prometheus format on port 9601)
- WebSocket stats: `GET /apps/{app_id}/channels` (REST API)

## Common Tasks

### Debug WebSocket Issues
```bash
# Enable debug logging via environment variable
DEBUG=true ./target/release/sockudo  # Takes precedence over DEBUG_MODE

# Or use Rust log levels
RUST_LOG=debug ./target/release/sockudo

# Check specific module
RUST_LOG=sockudo::websocket=debug ./target/release/sockudo
```

### Test Channel Authentication
```bash
# Generate auth signature for testing
echo -n "socket_id:channel_name" | openssl dgst -sha256 -hmac "app_secret" -hex
```

### Scale Horizontally
1. Choose adapter: Redis, Redis Cluster, or NATS
2. Configure all instances with same adapter settings
3. Load balance WebSocket connections (use sticky sessions)
4. Share app configuration via database backend


## Code Review Guidelines

- **Comments**: 
  * Never add comment to note that you have removed a code block
  * Don't add comment with no value or that over explain what you are doing or what you changed
  * Add comments when it will help the developer understand your intent or complex code

## Development Principles

- **Implementation Guidelines**:
  * Do not add placeholder implementations
  * A comment that an implementation is not done yet or will be done later is fine
  * Do not add config / env values for features that do not exist yet, unless asked directly