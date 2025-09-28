# Sockudo

A high-performance, scalable WebSocket server implementing the Pusher protocol in Rust.

[![Stars](https://img.shields.io/github/stars/rustnsparks/sockudo?style=social)](https://github.com/Sockudo/sockudo)
[![Build Status](https://img.shields.io/github/actions/workflow/status/rustnsparks/sockudo/ci.yml?branch=main)](https://github.com/rustnsparks/sockudo/actions)
[![License](https://img.shields.io/github/license/rustnsparks/sockudo)](LICENSE)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=rustnsparks/sockudo&type=Date)](https://star-history.com/#Sockudo/sockudo&Date)

## Features

- **🚀 High Performance** - Handle 100K+ concurrent connections
- **🔄 Pusher Compatible** - Drop-in replacement for Pusher services
- **🏗️ Scalable Architecture** - Redis, Redis Cluster, NATS adapters
- **🛡️ Production Ready** - Rate limiting, SSL/TLS, metrics
- **⚡ Async Cleanup** - Non-blocking disconnect handling
- **📊 Real-time Metrics** - Prometheus integration

## Quick Start

### Docker (Recommended)

```bash
# Clone and start with Docker Compose
git clone https://github.com/Sockudo/sockudo.git
cd sockudo
make up

# Server runs on http://localhost:6001
# Metrics on http://localhost:9601/metrics
```

### From Source

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build and run
git clone https://github.com/Sockudo/sockudo.git
cd sockudo
cargo run --release
```

## Basic Usage

Connect using any Pusher-compatible client:

```javascript
import Pusher from 'pusher-js';

const pusher = new Pusher('app-key', {
  wsHost: 'localhost',
  wsPort: 6001,
  cluster: '',
  forceTLS: false
});

const channel = pusher.subscribe('my-channel');
channel.bind('my-event', (data) => {
  console.log('Received:', data);
});
```

## Configuration

### Environment Variables

```bash
# Basic settings
PORT=6001
HOST=0.0.0.0
DEBUG=false
DEBUG=false # Enables more verbose logging

# Database (Example for Redis, adapt for MySQL/Postgres/DynamoDB if used as primary for AppManager)
REDIS_URL=redis://redis:6379/0 # Used by multiple components if not overridden
# For MySQL AppManager:
# DATABASE_MYSQL_HOST=mysql
# DATABASE_MYSQL_USER=sockudo
# DATABASE_MYSQL_PASSWORD=your_mysql_password
# DATABASE_MYSQL_DATABASE=sockudo

# Drivers (Lowercase: "local", "redis", "redis-cluster", "nats", "memory", "mysql", "pgsql", "dynamodb", "sqs")
ADAPTER_DRIVER=redis        # For horizontal scaling
APP_MANAGER_DRIVER=memory   # For app definitions
CACHE_DRIVER=redis          # For caching
QUEUE_DRIVER=redis          # For webhooks & background tasks

# Security
SSL_ENABLED=false # Set to true for production and provide paths
# SSL_CERT_PATH=/path/to/cert.pem
# SSL_KEY_PATH=/path/to/key.pem
# REDIS_PASSWORD=your-redis-password # If Redis requires auth

# Unix Socket (Alternative to HTTP/HTTPS - useful for reverse proxy setups)
UNIX_SOCKET_ENABLED=false # Set to true to use Unix socket instead of HTTP
# UNIX_SOCKET_PATH=/var/run/sockudo/sockudo.sock # Path to Unix socket file
# UNIX_SOCKET_PERMISSION_MODE=660 # Socket file permissions in octal

# Application Defaults (if not using a persistent AppManager or for fallback)
SOCKUDO_DEFAULT_APP_ID=demo-app
SOCKUDO_DEFAULT_APP_KEY=demo-key
SOCKUDO_DEFAULT_APP_SECRET=demo-secret
SOCKUDO_ENABLE_CLIENT_MESSAGES=true # For the default app
```

*Refer to `src/options.rs` and `src/main.rs` for a comprehensive list of all ENV var overrides and their default behaviors.*

### Configuration File (`config/config.json`)

Provides detailed control over all aspects. Below is a snippet; refer to your uploaded `config/config.json` for the full structure.

```json
{
  "debug": false,
  "host": "0.0.0.0",
  "port": 6001,
  "adapter": {
    "driver": "redis", // "local", "redis", "redis-cluster", "nats"
    "redis": { // Settings for 'redis' or 'redis-cluster' if chosen
      "prefix": "sockudo_adapter:",
      "cluster_mode": false // Set true if using redis-cluster via the 'redis' driver config
      // "requests_timeout": 5000
    },
    "cluster": { // Specific settings if driver is 'redis-cluster'
      // "nodes": ["redis://node1:7000", "redis://node2:7001"],
      // "prefix": "sockudo_cluster_adapter:"
    },
    "nats": {
      // "servers": ["nats://nats-server:4222"],
      // "prefix": "sockudo_nats_adapter:"
    }
  },
  "app_manager": {
    "driver": "memory", // "memory", "mysql", "pgsql", "dynamodb"
    "array": { // Used if driver is "memory" and you want to define apps in config
      "apps": [
        {
          "id": "my-app-id",
          "key": "my-app-key",
          "secret": "my-app-secret",
          "max_connections": 1000,
          "enable_client_messages": true,
          // ... other app-specific limits from src/app/config.rs
        }
      ]
    }
    // database-specific app_manager configs would go under "database"
  },
  "cache": {
    "driver": "redis", // "memory", "redis", "redis-cluster", "none"
    "redis": {
      // "prefix": "sockudo_cache:",
      // "url_override": "redis://another-redis:6379"
    },
    "memory": {
      // "ttl": 300, "max_capacity": 10000
    }
  },
  "queue": {
    "driver": "redis", // "memory", "redis", "redis-cluster", "sqs", "none"
    "redis": {
      // "concurrency": 5, "prefix": "sockudo_queue:"
    },
    "sqs": {
      // "region": "us-east-1", "concurrency": 5
    }
  },
  "rate_limiter": {
    "enabled": true,
    "driver": "redis", // Usually "memory" or "redis"
    "api_rate_limit": {
      "max_requests": 100,
      "window_seconds": 60
    },
    "websocket_rate_limit": { // Applied on connection
      "max_requests": 20,
      "window_seconds": 60
    }
  },
  "metrics": {
    "enabled": true,
    "driver": "prometheus", // Currently only prometheus supported
    "port": 9601
  },
  "ssl": {
    "enabled": false,
    // "cert_path": "/path/to/fullchain.pem",
    // "key_path": "/path/to/privkey.pem"
  },
  "unix_socket": {
    "enabled": false, // Set to true to use Unix socket instead of HTTP/HTTPS
    "path": "/var/run/sockudo/sockudo.sock", // Path to Unix socket file
    "permission_mode": "755" // Socket file permissions in octal (preferred)
  }
  // ... many other options available, see src/options.rs
}
```

-----

## 🏗️ Architecture

### System Overview

```
┌─────────────────┐    ┌───────────────────┐    ┌─────────────────┐
│   Client Apps   │    │  Load Balancer    │    │   Sockudo Node  │
│ (Web/Mobile/IoT)│◄──►│ (e.g., Nginx,    │◄──►│     (Rust)      │
│ using PusherJS  │    │      AWS ALB)     │    │                 │
└─────────────────┘    └───────────────────┘    └─────────────────┘
                                                        │ ▲
                                                        │ │ Pub/Sub
                        ┌─────────────────┐             │ ▼
                        │ Pub/Sub Backend │◄────────────┘
                        │ (Redis/NATS for │
                        │ HORIZONTAL_ADAPTER)│
                        └─────────────────┘
                                │
      ┌─────────────────────┐   │   ┌────────────────────┐
      │ App Config Storage  │◄──┘──►│  Cache Storage     │
      │ (MySQL/PG/DynamoDB/ │       │  (Redis/Memory)    │
      │      Memory)        │       └────────────────────┘
      └─────────────────────┘
```

### Core Components

- **Connection Handler (`adapter::handler`)**: Manages individual WebSocket connections, message parsing, authentication, and routing to appropriate handlers.
- **Channel Manager (`channel::manager`)**: Handles channel subscriptions, presence state, and signature validation for channel access.
- **Adapter (`adapter`)**: Abstract interface for connection management and message broadcasting.
    - **Local Adapter (`adapter::local_adapter`)**: Single-node operation.
    - **Horizontal Adapters (`adapter::redis_adapter`, `adapter::redis_cluster_adapter`, `adapter::nats_adapter`)**: Enable multi-node setups by using Redis, Redis Cluster, or NATS for pub/sub to synchronize state and broadcast messages across instances.
- **App Manager (`app::manager`)**: Manages application configurations (keys, secrets, limits, features), with backends like Memory, MySQL, PostgreSQL, or DynamoDB.
- **Cache Manager (`cache::manager`)**: Provides caching for frequently accessed data (e.g., channel information, app settings) with Memory or Redis backends.
- **Queue Manager (`queue::manager`)**: Handles background tasks, primarily for delivering webhooks, using Memory, Redis, Redis Cluster, or SQS.
- **Rate Limiter (`rate_limiter`)**: Protects against abuse with configurable limits, using Memory or Redis.
- **Webhook System (`webhook`)**: Delivers real-time server events (e.g., channel occupied/vacated, member added/removed) to configured HTTP endpoints or AWS Lambda functions.
- **Metrics Collector (`metrics`)**: Exposes Prometheus metrics for monitoring performance and health.
- **HTTP Handler (`http_handler`)**: Provides the Pusher-compatible REST API for triggering events and querying server state.
- **WebSocket Handler (`ws_handler`)**: Manages the WebSocket upgrade process and initial connection setup.

### Supported Architectures

- **Single Node**: Ideal for development, testing, and smaller applications. Uses `local` adapter and `memory` for other components if not otherwise configured.
- **Multi-Node (Clustered)**: Achieves horizontal scalability using Redis, Redis Cluster, or NATS as the `adapter.driver` for message broadcasting and state synchronization between Sockudo instances.
- **Microservices Integration**: Sockudo can act as the real-time layer in a microservices architecture, receiving events to broadcast via its HTTP API.
- **Serverless Integration**: Webhooks can trigger AWS Lambda functions, allowing for serverless processing of Sockudo events.

-----

## 🔗 Client Integration

Sockudo implements the **Pusher WebSocket protocol**. This means you can use any Pusher-compatible **client-side library** to connect to Sockudo for real-time messaging.

For triggering events from **your application backend** (server-to-server), you would use Sockudo's HTTP API (see API Reference section), potentially with an HTTP client or a Pusher *server-side* library configured to point to Sockudo's HTTP API endpoints.

### Recommended Client-Side Libraries (for WebSocket Connection)

#### JavaScript/TypeScript (PusherJS)

This is the most common library for frontend integration.

```bash
npm install pusher-js
```

```javascript
import Pusher from 'pusher-js';

const pusher = new Pusher('your-app-key', { // Use the 'key' from your app config
  wsHost: 'localhost',      // Your Sockudo host
  wsPort: 6001,             // Your Sockudo WebSocket port
  wssPort: 6001,            // Your Sockudo WSS port (if SSL_ENABLED=true)
  forceTLS: false,          // Set true if using WSS
  enabledTransports: ['ws', 'wss'],
  cluster: 'mt1', // Required by pusher-js, value doesn't impact Sockudo directly
  disableStats: true, // Recommended
  authEndpoint: '/your-backend-auth-endpoint-for-private-channels', // For private/presence channels
  // authTransport: 'ajax' // or 'jsonp'
});

const channel = pusher.subscribe('public-channel');
channel.bind('some-event', (data) => {
  console.log('Received data:', data);
});
```

#### Other Pusher-Compatible Client Libraries

# Default app credentials
SOCKUDO_DEFAULT_APP_ID=app-id
SOCKUDO_DEFAULT_APP_KEY=app-key
SOCKUDO_DEFAULT_APP_SECRET=app-secret

# Scaling drivers
ADAPTER_DRIVER=redis          # local, redis, redis-cluster, nats
CACHE_DRIVER=redis           # memory, redis, redis-cluster, none
QUEUE_DRIVER=redis           # memory, redis, redis-cluster, sqs, none
```

### Performance Tuning

Use Sockudo's HTTP API. Examples using `curl` are in the "Usage Examples" and "API Reference" sections.
Libraries like Pusher's official PHP or Go server libraries can also be used if you configure their host/port to point to your Sockudo HTTP API endpoint (`http://localhost:6001` by default).

**PHP (Pusher Server SDK - configured for Sockudo)**

```php
<?php
require __DIR__ . '/vendor/autoload.php';

$options = [
  'host' => 'localhost', // Sockudo HTTP API host
  'port' => 6001,        // Sockudo HTTP API port
  'scheme' => 'http',    // 'https' if SSL_ENABLED=true on Sockudo
  'encrypted' => false,  // Relevant for Pusher Cloud, less so here
  'useTLS' => false,     // Set true if Sockudo is using HTTPS
  // cluster parameter is often not needed when directly targeting Sockudo
];

// Ensure 'your-app-id' matches an 'id' field in one of your configured apps in Sockudo
$pusher = new Pusher\Pusher('your-app-key', 'your-app-secret', 'your-app-id', $options);

$data = ['message' => 'hello world from PHP backend via Sockudo'];
$pusher->trigger('my-channel', 'my-event', $data);
?>
```

**Go (Pusher HTTP Go - configured for Sockudo)**

```go
package main

import (
	"fmt"
	"[github.com/pusher/pusher-http-go/v5](https://github.com/pusher/pusher-http-go/v5)"
)

func main() {
	client := pusher.Client{
		AppID:   "your-app-id",     // Matches 'id' in Sockudo app config
		Key:     "your-app-key",    // Matches 'key'
		Secret:  "your-app-secret", // Matches 'secret'
		Host:    "localhost:6001",  // Sockudo HTTP API host and port
		Secure:  false,             // Set to true if Sockudo is using HTTPS
	}

	data := map[string]string{"message": "hello from Go backend via Sockudo"}
	events, err := client.Trigger("my-channel", "my-event", data)
	if err != nil {
		fmt.Println("Error triggering event:", err)
		return
	}
	fmt.Printf("Events triggered: %+v\n", events)
}
```

-----

## 🔌 Unix Socket Support

Sockudo supports Unix domain sockets as an alternative to HTTP/HTTPS servers, providing performance benefits and security advantages when deployed behind reverse proxies.

### Benefits of Unix Sockets

- **Performance**: Eliminates TCP/IP stack overhead for local communication
- **Security**: Unix sockets are filesystem objects, not network accessible
- **Flexibility**: Easier deployment behind reverse proxies without port conflicts
- **Resource Efficiency**: Single server process instead of concurrent HTTP + Unix socket

### Configuration

**Important**: When Unix socket is enabled, the HTTP/HTTPS server will **NOT** start. This is an either/or choice, not concurrent operation.

#### Environment Variables
```bash
# Enable Unix socket (disables HTTP/HTTPS)
UNIX_SOCKET_ENABLED=true
UNIX_SOCKET_PATH=/var/run/sockudo/sockudo.sock
UNIX_SOCKET_PERMISSION_MODE=660  # Octal permissions
```

#### Configuration File
```json
{
  "unix_socket": {
    "enabled": true,
    "path": "/var/run/sockudo/sockudo.sock",
    "permission_mode": "755"  // Octal string notation (recommended)
  }
}
```

**Alternative formats supported:**
```json
"permission_mode": "755"  // Octal string (recommended)
"permission_mode": 755    // Parsed as octal if ≤ 777 and all digits 0-7
"permission_mode": 493    // Decimal equivalent (legacy support)
```

#### Common Permission Values
| Octal | JSON Value | Permissions | Use Case |
|-------|------------|-------------|----------|
| `755` | `"755"` | rwxr-xr-x | Standard (recommended) |
| `660` | `"660"` | rw-rw---- | Restricted to group |
| `644` | `"644"` | rw-r--r-- | Read-only for group/others |

### Reverse Proxy Integration

#### Nginx Configuration Example
```nginx
upstream sockudo {
    server unix:/var/run/sockudo/sockudo.sock;
}

server {
    listen 80;
    server_name your-domain.com;

    # WebSocket upgrade support
    location / {
        proxy_pass http://sockudo;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;

        # Forward real IP
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

#### Apache Configuration Example
```apache
<VirtualHost *:80>
    ServerName your-domain.com

    # Enable proxy modules: proxy, proxy_http, proxy_wstunnel
    ProxyRequests Off
    ProxyPreserveHost On

    # WebSocket support
    ProxyPass / unix:/var/run/sockudo/sockudo.sock|http://localhost/
    ProxyPassReverse / unix:/var/run/sockudo/sockudo.sock|http://localhost/

    # WebSocket upgrade
    RewriteEngine on
    RewriteCond %{HTTP:Upgrade} websocket [NC]
    RewriteCond %{HTTP:Connection} upgrade [NC]
    RewriteRule ^/?(.*) "ws://localhost/$1" [P,L]
</VirtualHost>
```

### Platform Support

Unix sockets are supported on Unix-like systems:
- ✅ Linux (all distributions)
- ✅ macOS
- ❌ Windows (configuration ignored, falls back to HTTP)

-----

## 🚢 Deployment

Refer to `README-Docker.md` for comprehensive Docker deployment instructions.

### Docker Deployment (Recommended)

#### Development

```bash
# Connection limits
SOCKUDO_DEFAULT_APP_MAX_CONNECTIONS=100000
SOCKUDO_DEFAULT_APP_MAX_CLIENT_EVENTS_PER_SECOND=10000

# Cleanup performance (for handling mass disconnects)
CLEANUP_QUEUE_BUFFER_SIZE=50000
CLEANUP_BATCH_SIZE=25
CLEANUP_WORKER_THREADS=auto

# CPU scaling
ADAPTER_BUFFER_MULTIPLIER_PER_CPU=128
```

## Deployment Scenarios

| Scenario | CPU/RAM | Adapter | Cache | Queue | Max Connections |
|----------|---------|---------|-------|-------|-----------------|
| **Development** | 1vCPU/1GB | local | memory | memory | 1K |
| **Small Production** | 2vCPU/2GB | redis | redis | redis | 10K |
| **High Traffic** | 4vCPU/4GB+ | redis | redis | redis | 50K+ |
| **Multi-Region** | 8vCPU/8GB+ | redis-cluster | redis-cluster | redis-cluster | 100K+ |

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Load Balancer │────│   Sockudo Node  │────│   Redis Cluster │
│    (Nginx)      │    │    (Rust/Tokio) │    │  (State Store)  │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         │              ┌─────────────────┐             │
         └──────────────│   Sockudo Node  │─────────────┘
                        │    (Rust/Tokio) │
                        └─────────────────┘
```

## Documentation

- **[Full Documentation](docs/)** - Complete setup and configuration guide
- **[Performance Tuning](docs/QUEUE_CONFIG.md)** - Optimize for your workload
- **[Docker Deployment](docker-compose.yml)** - Production-ready containers
- **[API Reference](docs/API.md)** - WebSocket and HTTP API details

## Testing

```bash
# Run all tests
make test

# Interactive WebSocket testing
cd test/interactive && npm install && npm start
# Open http://localhost:3000

# Load testing
make benchmark
```

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

Licensed under the [MIT License](LICENSE).

## Support

- **Issues**: [GitHub Issues](https://github.com/Sockudo/sockudo/issues)
- **Discussions**: [GitHub Discussions](https://github.com/RustNSparks/sockudo/discussions)
- **Documentation**: [sockudo.app](https://sockudo.app)
- **Discord**: [Join our Discord](https://discord.gg/ySfNxfh2gZ)
- **X**: [@sockudorealtime](https://x.com/sockudorealtime)
- **Email**: [sockudorealtime](mailto:sockudorealtime@gmail.com)
