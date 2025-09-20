# Sockudo 🚀

[![Crates.io](https://img.shields.io/crates/v/sockudo)](https://crates.io/crates/sockudo)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.85+-blue.svg)](https://www.rust-lang.org)
[![WebSocket](https://img.shields.io/badge/websocket-RFC%206455-green.svg)](https://tools.ietf.org/html/rfc6455)

**High-performance, scalable WebSocket server built in Rust** 🦀

Sockudo is a production-ready WebSocket server that provides real-time communication capabilities with built-in support for channels, presence, authentication, rate limiting, and horizontal scaling. Designed for modern applications that need reliable, fast, and scalable real-time features. It aims for Pusher protocol compatibility, making it a potential backend for clients like Laravel Echo.

📚 **[Full Documentation](https://sockudo.app/)** - Visit our comprehensive documentation site for detailed guides, API references, and examples.

---

## ✨ Features

### 🚀 **Core Capabilities**
- **High Performance**: Built in Rust for maximum speed and efficiency.
- **WebSocket Protocol**: Full RFC 6455 compliance.
- **Channel System**: Organize connections with public, private, and presence channels. Support for private-encrypted channels is also included.
- **Real-time Messaging**: Instant bidirectional communication.
- **Horizontal Scaling**: Multi-node clustering with adapters for Redis, Redis Cluster, and NATS.

### 🔐 **Security & Authentication**
- **Channel Authentication**: Secure private and presence channels using token-based authentication.
- **User Authentication**: User sign-in support with custom data, typically validated with signatures.
- **Rate Limiting**: Configurable limits for API and WebSocket connections, with backends like Memory or Redis.
- **SSL/TLS Support**: Production-ready HTTPS and WSS.
- **CORS Configuration**: Flexible cross-origin request handling.

### 📊 **Monitoring & Observability**
- **Prometheus Metrics**: Comprehensive performance monitoring for connections, messages, API calls, and adapter performance.
- **Health Checks**: Built-in health endpoints (`/up/{appId}`) for load balancers.
- **Structured Logging**: Configurable logging with multiple levels (DEBUG/INFO).
- **Debug Mode**: Enhanced debugging for development.

### 🎛️ **Flexible Architecture**
- **Multiple Adapters**: Choose between Local (single-node), Redis, Redis Cluster, or NATS for message broadcasting and horizontal scaling.
- **Database Support for App Management**: Options include Memory, MySQL, PostgreSQL, and DynamoDB for storing application configurations.
- **Queue Systems**: Supports Memory, Redis, Redis Cluster, and SQS for background job processing (e.g., webhooks).
- **Cache Backends**: Options for Memory, Redis, or Redis Cluster for caching channel data and other information.

### 🔗 **Integration Features**
- **Webhooks**: Real-time event notifications (e.g., `channel_occupied`, `channel_vacated`, `member_added`, `member_removed`, `client_event`) to your services.
- **AWS Lambda**: Direct Lambda function invocation for webhooks.
- **REST API**: Full HTTP API for triggering events, batch events, and managing channels/users.
- **Client Libraries**: Compatible with Pusher client libraries due to protocol adherence.

---

## 🚀 Quick Start

### Using Docker (Recommended)

```bash
# Clone the repository
git clone https://github.com/RustNSparks/sockudo.git
cd sockudo

# Ensure you have an .env file (e.g., from .env.example)
# cp .env.example .env
# nano .env # Customize if needed

# Quick setup and start (assumes make command is configured)
make quick-start
# Or, using docker-compose directly:
# docker-compose up -d

# you can also use 
docker pull sockudo/sockudo:latest
````

Sockudo should now be running. Default endpoints (configurable):

- **WebSocket Server**: `ws://localhost:6001/app/your-app-key` (e.g., `demo-key` if using default app)
- **REST API**: `http://localhost:6001` (e.g., `http://localhost:6001/apps/demo-app/events`)
- **Metrics**: `http://localhost:9601/metrics`
- **Health Check**: `http://localhost:6001/up/your-app-id`

### Manual Installation

#### Prerequisites

- Rust 1.85+ (`rustup install stable`)
- Redis (optional, for scaling and certain drivers)
- MySQL/PostgreSQL (optional, for persistent app storage if using those drivers)

#### Install from Crates.io

```bash
# Install using cargo
cargo install sockudo

# Or install using cargo-binstall (faster)
cargo binstall sockudo

# Run Sockudo (ensure config/config.json exists or use ENV variables)
sockudo --config config/config.json
```

#### Build from Source

```bash
# Clone the repository
git clone [https://github.com/RustNSparks/sockudo.git](https://github.com/RustNSparks/sockudo.git)
cd sockudo

# Build the project
cargo build --release

# Run with a configuration file (recommended)
./target/release/sockudo --config config/config.json
# Or rely on environment variables and default app settings
# ./target/release/sockudo
```

-----

## 📖 Usage Examples

### WebSocket Client Connection (JavaScript)

```javascript
// Connect to a public channel
const ws = new WebSocket('ws://localhost:6001/app/your-app-key'); // Replace your-app-key

ws.onopen = () => {
    console.log('Connected to Sockudo!');
    // Subscribe to a channel
    ws.send(JSON.stringify({
        event: 'pusher:subscribe',
        data: { channel: 'my-channel' }
    }));
};

ws.onmessage = (event) => {
    const data = JSON.parse(event.data);
    console.log('Received:', data);

    // Example: Handle connection established
    if (data.event === 'pusher:connection_established') {
        const socketId = JSON.parse(data.data).socket_id;
        console.log('My Socket ID is:', socketId);
    }
    // Example: Handle subscription success
    if (data.event === 'pusher_internal:subscription_succeeded' && data.channel === 'my-channel') {
        console.log('Successfully subscribed to my-channel!');
    }
};

ws.onerror = (error) => {
    console.error('WebSocket Error:', error);
};

ws.onclose = () => {
    console.log('Disconnected from Sockudo.');
};
```

### Private Channel with Authentication (Conceptual Client-Side)

*Actual signature generation must happen on your backend.*

```javascript
const ws = new WebSocket('ws://localhost:6001/app/your-app-key');

ws.onopen = () => {
    // Assume 'generatedAuthSignature' is obtained from your backend
    // after authenticating the user for this channel and socket_id.
    // See Security section for more details on signature generation.
    const generatedAuthSignature = "your-app-key:backend_generated_signature_for_private_channel";

    ws.send(JSON.stringify({
        event: 'pusher:subscribe',
        data: {
            channel: 'private-orders',
            auth: generatedAuthSignature
        }
    }));
};
```

### Presence Channel (Conceptual Client-Side)

*Actual signature generation must happen on your backend.*

```javascript
const ws = new WebSocket('ws://localhost:6001/app/your-app-key');

ws.onopen = () => {
    // Assume 'generatedAuthSignatureForPresence' is obtained from your backend.
    // Channel data includes user_id and user_info for presence events.
    const channelData = JSON.stringify({
        user_id: 'user123',
        user_info: { name: 'John Doe', avatar: 'avatar.jpg' }
    });
    const generatedAuthSignatureForPresence = "your-app-key:backend_generated_signature_for_presence_channel";


    ws.send(JSON.stringify({
        event: 'pusher:subscribe',
        data: {
            channel: 'presence-chat',
            auth: generatedAuthSignatureForPresence,
            channel_data: channelData
        }
    }));
};

// Listen for presence events
ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.event === 'pusher_internal:member_added' && message.channel === 'presence-chat') {
        console.log('Member added:', message.data);
    }
    if (message.event === 'pusher_internal:member_removed' && message.channel === 'presence-chat') {
        console.log('Member removed:', message.data);
    }
};
```

### REST API - Trigger Events (from your Backend)

Requires authentication (see Security section).

```bash
# Send event to a channel
# Replace your-app-id, your-app-key, and generate a valid auth_signature
curl -X POST "http://localhost:6001/apps/your-app-id/events?auth_key=your-app-key&auth_timestamp=$(date +%s)&auth_version=1.0&auth_signature=your_generated_signature&body_md5=$(echo -n '{"name":"order-update","channels":["private-orders"],"data":{"order_id":123,"status":"shipped"}}' | md5sum | awk '{print $1}')" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "order-update",
    "channels": ["private-orders"],
    "data": {"order_id": 123, "status": "shipped"}
  }'
```

### Client Events (from an Authenticated WebSocket Client)

Client events can only be sent on private or presence channels by authenticated clients for whom the app has `enable_client_messages` set to `true`.

```javascript
// Ensure the WebSocket 'ws' is connected and subscribed to 'presence-chat' (a private or presence channel)
ws.send(JSON.stringify({
    event: 'client-message', // Must start with 'client-'
    channel: 'presence-chat',
    data: { message: 'Hello everyone!' }
}));
```

-----

## 🔧 Configuration

Sockudo can be configured via a JSON file (default: `config/config.json`) or environment variables. Environment variables generally override file settings for basic options, but the file provides more detailed structure.

### Environment Variables (Key Examples)

Create a `.env` file (or set them in your environment):

```bash
# Basic Settings
HOST=0.0.0.0
PORT=6001
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

Most Pusher client libraries for other languages (Python, Ruby, Java, .NET/C\#, Swift, Android, etc.) can be configured to connect to a custom host and port, allowing them to work with Sockudo. You'll typically need to set:

- App Key
- Host (your Sockudo server address)
- Port (your Sockudo WebSocket port)
- `forceTLS` (or equivalent for secure connections)
- An `authEndpoint` if using private or presence channels.

#### Native WebSocket

You can also connect directly using any standard WebSocket client, but you'll need to implement the Pusher message format manually (e.g., for `pusher:subscribe`, `pusher:ping`).

```javascript
const ws = new WebSocket('ws://localhost:6001/app/your-app-key');

ws.onopen = () => {
  ws.send(JSON.stringify({
    event: 'pusher:subscribe',
    data: { channel: 'my-channel' }
  }));
};
// ... handle other WebSocket events
```

### Triggering Events from Your Backend (Server-Side)

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
# Start development environment (uses docker-compose.yml and docker-compose.dev.yml)
make dev # (If Makefile provides this shortcut)
# OR
docker-compose -f docker-compose.yml -f docker-compose.dev.yml up -d

# View logs
docker-compose logs -f sockudo

# Run tests (if configured in a Makefile or script)
# make test
```

#### Development Setup

**For contributors and developers:**

1. **Install Git hooks** (recommended for all developers):
   ```bash
   ./scripts/install-hooks.sh
   ```
   This installs a pre-commit hook that automatically formats Rust code before commits.

2. **Manual hook installation** (alternative):
   ```bash
   # Copy the hook manually
   cp scripts/git-hooks/pre-commit .git/hooks/pre-commit
   chmod +x .git/hooks/pre-commit
   ```

3. **Disable hooks temporarily** (if needed):
   ```bash
   git commit --no-verify
   ```

The pre-commit hook ensures your code is properly formatted and matches CI requirements.

#### Production

```bash
# Production deployment (uses docker-compose.yml and docker-compose.prod.yml)
make prod # (If Makefile provides this shortcut)
# OR
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d

# Scale to multiple instances (example)
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d --scale sockudo=3

# Monitor performance (example for Docker stats)
docker stats
```

### Kubernetes

A basic Kubernetes deployment might look like this (adapt as needed):

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: sockudo
spec:
  replicas: 3
  selector:
    matchLabels:
      app: sockudo
  template:
    metadata:
      labels:
        app: sockudo
    spec:
      containers:
      - name: sockudo
        image: sockudo/sockudo:latest
        ports:
        - name: websocket
          containerPort: 6001 # Sockudo's application port
        - name: metrics
          containerPort: 9601 # Sockudo's metrics port
        env:
        - name: ADAPTER_DRIVER 
          value: "redis" # or "nats", "redis-cluster"
        - name: REDIS_URL
          value: "redis://your-redis-service:6379"
        # Add other necessary environment variables or configmap mounts
        # - name: CONFIG_FILE_PATH
        #   value: "/app/config/config.json"
        # volumeMounts:
        # - name: sockudo-config
        #   mountPath: /app/config
      # volumes:
      # - name: sockudo-config
      #   configMap:
      #     name: sockudo-configmap
```

*Ensure your Kubernetes setup includes appropriate services, ingress, and configuration for Redis/NATS if using horizontal scaling.*

### Cloud Platforms

Sockudo can be deployed to various cloud platforms:

#### AWS ECS / EKS

- Use the production Docker image.
- Configure Application Load Balancer (ALB) or Network Load Balancer (NLB) with WebSocket support (sticky sessions might be needed depending on adapter and client behavior, though ideally not with a proper horizontal adapter).
- Use ElastiCache for Redis (standalone or cluster) if using Redis adapter/cache/queue.
- Use RDS for MySQL/PostgreSQL if using those for AppManager.
- Use Amazon SQS if using the SQS queue driver.

#### Google Cloud Run / GKE

- Enable WebSocket support in Cloud Run or use GKE.
- Use Cloud Memorystore for Redis.
- Configure Cloud SQL for persistent app storage.

#### Azure Container Instances / AKS

- Use Azure Cache for Redis.
- Configure Azure Database for MySQL/PostgreSQL.

-----

## 📊 Monitoring

### Metrics

Sockudo exposes Prometheus metrics at the `/metrics` endpoint (default port 9601).
Key metrics include:

- **Connection Metrics**:
    * `sockudo_connected`: Current number of active WebSocket connections.
    * `sockudo_new_connections_total`: Total number of new connections established.
    * `sockudo_new_disconnections_total`: Total number of disconnections.
- **Message Metrics**:
    * `sockudo_ws_messages_sent_total`: Total WebSocket messages sent from server to clients.
    * `sockudo_ws_messages_received_total`: Total WebSocket messages received by server from clients.
    * `sockudo_socket_transmitted_bytes`: Total bytes transmitted over WebSockets.
    * `sockudo_socket_received_bytes`: Total bytes received over WebSockets.
- **HTTP API Metrics**:
    * `sockudo_http_calls_received_total`: Total HTTP API calls received.
    * `sockudo_http_transmitted_bytes`: Total bytes sent in HTTP API responses.
    * `sockudo_http_received_bytes`: Total bytes received in HTTP API requests.
- **Horizontal Adapter Metrics** (if using Redis/NATS adapter):
    * `sockudo_horizontal_adapter_resolve_time`: Histogram of request resolution time between nodes.
    * `sockudo_horizontal_adapter_resolved_promises`: Total promises fulfilled by other nodes.
    * `sockudo_horizontal_adapter_sent_requests`: Total requests sent to other nodes.
    * `sockudo_horizontal_adapter_received_requests`: Total requests received from other nodes.
    * `sockudo_horizontal_adapter_received_responses`: Total responses received from other nodes.
- **Channel Statistics** (can be inferred or might require specific metrics not listed but common):
    * `sockudo_channel_count` (conceptual, often derived from `get_channels_with_socket_count`)
    * `sockudo_active_subscriptions` (conceptual)

### Grafana Dashboard

If using the Docker setup with Grafana (`docker-compose.prod.yml`), a pre-configured dashboard might be available. Otherwise, you can import/create dashboards in Grafana using Prometheus as a data source.
Access Grafana at `http://localhost:3000` (default credentials may vary, check `docker-compose.prod.yml` or `README-Docker.md`).

### Health Checks

- **Application Health**: `GET /up/{app_id}` - Checks if a specific application is responsive.
- **Metrics Endpoint**: `GET /metrics` (default on port 9601) - Exposes Prometheus metrics.
- **General Usage/Memory**: `GET /usage` - Provides memory usage statistics of the Sockudo server.

-----

## 🔒 Security

### Authentication

#### Channel Authentication (Private & Presence Channels)

Sockudo validates `auth` tokens for private and presence channel subscriptions. The signature is typically generated on your backend.
The string to sign depends on the channel type:

- **Private channels**: `socket_id:channel_name`
- **Presence channels**: `socket_id:channel_name:channel_data_json_string` (where `channel_data_json_string` is the JSON string provided in the `channel_data` field of the subscription request).

**Example Server-Side Signature Generation (Node.js conceptual):**

```javascript
const crypto = require('crypto');

function generateChannelAuth(socketId, channelName, appSecret, channelDataString = null) {
    let stringToSign = `${socketId}:${channelName}`;
    if (channelDataString) { // For presence channels
        stringToSign += `:${channelDataString}`;
    }
    const signature = crypto.createHmac('sha256', appSecret)
                              .update(stringToSign)
                              .digest('hex');
    return signature; // The client would send appKey:signature
}

// For a private channel:
// const appKey = "your-app-key";
// const signature = generateChannelAuth(clientSocketId, "private-mychannel", "your-app-secret");
// const authString = `${appKey}:${signature}`;

// For a presence channel:
// const channelData = JSON.stringify({ user_id: "123", user_info: { name: "Alice" } });
// const presenceSignature = generateChannelAuth(clientSocketId, "presence-mychannel", "your-app-secret", channelData);
// const presenceAuthString = `${appKey}:${presenceSignature}`;
```

*Refer to `src/channel/manager.rs` (method `get_data_to_sign_for_signature`) for the exact server-side string construction.*

#### User Authentication (pusher:signin)

If `enable_user_authentication` is true for an app, clients can send a `pusher:signin` event.
The string to sign for user authentication is `socket_id::user::user_data_json_string`.

**Example Server-Side User Auth Token Generation (Node.js conceptual):**

```javascript
const crypto = require('crypto');

function generateUserAuth(socketId, userDataJsonString, appSecret) {
    const stringToSign = `${socketId}::user::${userDataJsonString}`;
    const signature = crypto.createHmac('sha256', appSecret)
                              .update(stringToSign)
                              .digest('hex');
    return signature; // The client would send this as part of the pusher:signin 'auth' field
}

// const appKey = "your-app-key";
// const userData = JSON.stringify({ id: "user456", name: "Bob" });
// const userAuthSignature = generateUserAuth(clientSocketId, userData, "your-app-secret");
// Client sends: { event: "pusher:signin", data: { auth: `${appKey}:${userAuthSignature}`, user_data: userData }}
// Note: The `src/app/auth.rs` `validate_channel_auth` and `sing_in_token_for_user_data` suggest the client sends only the signature part in `auth`, and `user_data` separately. Your client example shows this correctly.
```

*Refer to `src/app/auth.rs` (method `sing_in_token_for_user_data`) for server-side logic.*

### Rate Limiting

Configure rate limits in `config/config.json` or via environment variables to protect against abuse. Sockudo supports separate limits for HTTP API requests and WebSocket connections.
Backends like Memory or Redis can be used for rate limiting.

```json
{
  "rate_limiter": {
    "enabled": true,
    "driver": "redis", // "memory" or "redis"
    "api_rate_limit": {
      "max_requests": 1000, // e.g., per minute for all API calls from an IP
      "window_seconds": 60
    },
    "websocket_rate_limit": { // Applied per IP at connection time
      "max_requests": 60,   // e.g., max 60 connection attempts per minute from an IP
      "window_seconds": 60
    },
    "redis": { // If Redis driver is used for rate limiting
        "prefix": "sockudo_rl:"
        // url_override can also be set here if different from main Redis
    }
  }
}
```

*Client-specific event rate limits are configured per-application (e.g., `max_client_events_per_second`).*

### SSL/TLS

Enable HTTPS and WSS in production by setting `ssl.enabled = true` in `config/config.json` or `SSL_ENABLED=true` (env var) and providing paths to your SSL certificate and key.
Refer to `README-Docker.md` for Docker-specific SSL setup with Nginx or Let's Encrypt.

```json
  "ssl": {
    "enabled": true,
    "cert_path": "/etc/letsencrypt/live/[your-domain.com/fullchain.pem](https://your-domain.com/fullchain.pem)",
    "key_path": "/etc/letsencrypt/live/[your-domain.com/privkey.pem](https://your-domain.com/privkey.pem)",
    "redirect_http": true, // Redirect HTTP traffic to HTTPS
    "http_port": 8080 // Port for HTTP redirect server if Sockudo handles SSL
  }
```

-----

## 🔌 API Reference

### WebSocket Events (Pusher Protocol)

Sockudo aims for compatibility with the Pusher WebSocket protocol. Key events include:

#### Connection Events

- `pusher:connection_established`: Sent to the client upon successful connection. Contains `socket_id` and `activity_timeout`.
- `pusher:error`: Sent to the client when an error occurs (e.g., auth failure, invalid message). Contains `code` and `message`.
- `pusher:ping`: Sent by the server to check if the client is alive. Client should respond with `pusher:pong`.
- `pusher:pong`: Sent by the client in response to a `pusher:ping`.

#### Subscription Events

- `pusher:subscribe`: Sent by the client to subscribe to a channel. `data` includes `channel` name and optionally `auth` string (for private/presence) and `channel_data` (for presence).
- `pusher:unsubscribe`: Sent by the client to unsubscribe from a channel. `data` includes `channel` name.
- `pusher_internal:subscription_succeeded`: Sent to the client by the server confirming successful subscription to a channel. For presence channels, `data` includes current member list (`presence.hash`, `presence.ids`, `presence.count`).
- `pusher:cache_miss`: Sent to the client if they subscribe to a cache channel and no data is initially cached.

#### Presence Events (Sent by Server to Subscribed Clients)

- `pusher_internal:member_added`: Sent to clients in a presence channel when a new member joins. `data` includes `user_id` and `user_info`.
- `pusher_internal:member_removed`: Sent to clients in a presence channel when a member leaves. `data` includes `user_id`.

#### User Authentication Events

- `pusher:signin`: Sent by the client to authenticate themselves as a user. `data` includes `auth` (app\_key:signature) and `user_data` (JSON string with user details like `id`).
- `pusher:signin_success`: Sent to the client by the server upon successful user sign-in.

#### Client Events (Sent by Client, Broadcast by Server)

- `client-*` (e.g., `client-message`): Custom events sent by one client, broadcast by the server to other clients on the same private or presence channel (excluding the sender by default, if `socket_id` is provided when triggering via HTTP API). The app must have `enable_client_messages: true`.

### REST API Endpoints

All endpoints are prefixed with `/apps/{app_id}/`. Authentication is required for these endpoints.

#### Trigger Event

- **Endpoint**: `POST /apps/{app_id}/events`
- **Description**: Triggers an event on one or more channels.
- **Body**:
  ```json
  {
    "name": "your-event-name",
    "channels": ["channel-1", "channel-2"], // Or "channel": "single-channel"
    "data": {"key": "value"}, // JSON string or object for event data
    "socket_id": "optional_socket_id_to_exclude" // Optional
    // "info": "user_count,subscription_count" // Optional: to request info about channels in response
  }
  ```
- **Response**: `200 OK` with `{"ok":true}` or channel info if requested.

#### Batch Events

- **Endpoint**: `POST /apps/{app_id}/batch_events`
- **Description**: Triggers multiple events in a single request.
- **Body**:
  ```json
  {
    "batch": [
      {"name": "event1", "channel": "channel1", "data": "data1"},
      {"name": "event2", "channels": ["channel2", "channel3"], "data": {"complex":"data2"}}
      // Each item can also have "socket_id" and "info"
    ]
  }
  ```
- **Response**: `200 OK` with `{}` or `{"batch": [...]}` containing info for each event if requested.

#### Get Channel Information

- **Endpoint**: `GET /apps/{app_id}/channels/{channel_name}`
- **Description**: Retrieves information about a specific channel.
- **Query Parameters**:
    * `info`: Comma-separated list of attributes to retrieve (e.g., `user_count`, `subscription_count`, `cache`).
- **Response**: `200 OK` with JSON containing channel status (e.g., `{"occupied": true, "user_count": 5, "subscription_count": 10}`).

#### List Channels

- **Endpoint**: `GET /apps/{app_id}/channels`
- **Description**: Retrieves a list of active channels in an application.
- **Query Parameters**:
    * `filter_by_prefix`: Filters channels by the given prefix.
    * `info`: Comma-separated list of attributes to retrieve for each channel (e.g., `user_count`).
- **Response**: `200 OK` with JSON `{"channels": {"channel_name": {"user_count": 2}, ...}}`.

#### Get Users in a Presence Channel

- **Endpoint**: `GET /apps/{app_id}/channels/{channel_name}/users`
- **Description**: Retrieves the list of users subscribed to a specific presence channel.
- **Response**: `200 OK` with JSON `{"users": [{"id": "user_id_1"}, {"id": "user_id_2"}]}`.

#### Terminate User Connections

- **Endpoint**: `POST /apps/{app_id}/users/{user_id}/terminate_connections`
- **Description**: Forcibly disconnects all WebSocket connections associated with a given `user_id`.
- **Response**: `200 OK` with `{"ok":true}`.

### Other Endpoints

- **Health Check**: `GET /up/{app_id}` - Returns `200 OK` if the app is known (even if no connections).
- **Server Usage**: `GET /usage` - Returns server memory statistics.
- **Prometheus Metrics**: `GET /metrics` (default port 9601) - Exposes metrics in Prometheus format.

-----

## 🧪 Testing

Sockudo includes a suite of tests to ensure reliability.

### Running Tests

```bash
# Run all unit and integration tests (ensure dependencies like Redis might be needed for some)
cargo test

# For Docker-based testing environment, refer to README-Docker.md
# Example: make ci-test (if Makefile provides this)
```

### Interactive Tester

The project includes an interactive tester in `test/interactive/` which can be used to manually test WebSocket connections, subscriptions, client events, and webhooks against a running Sockudo instance.

### Automated Tests

Automated tests can be found in `test/automated/` which might include scripts for load testing or specific feature validation.

-----

## 🤝 Contributing

We welcome contributions! Please see our [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines on how to get started, coding standards, and the PR review process.

-----

## 📚 Documentation

For more in-depth information:

- Explore the `src/` directory for detailed module implementations.
- Configuration options are detailed in `src/options.rs`.
- Docker setup is described in `README-Docker.md`.
- (If a dedicated documentation site exists, link it here, e.g., `https://sockudo.app` as mentioned in the original README).

-----

## 📄 License

This project is licensed under the **MIT License** - see the [LICENSE](https://github.com/RustNSparks/sockudo/blob/master/LICENSE) file for details.

-----

## 🙏 Acknowledgments

- Built with [Tokio](https://tokio.rs/) for async runtime.
- Uses [fastwebsockets](https://github.com/denoland/fastwebsockets) for WebSocket implementation.
- Inspired by [Pusher](https://pusher.com/) and similar real-time messaging platforms like [Laravel Reverb](https://reverb.laravel.com/).
- Thanks to the Rust community for its robust ecosystem and excellent crates.

-----

## 📞 Support

### Community Support
- **GitHub Issues**: [Report bugs and request features](https://github.com/RustNSparks/sockudo/issues)
- **Crates.io**: [View package details](https://crates.io/crates/sockudo)
- **Discord**: [Join the RustNSparks server](https://discord.gg/GUuxFbD8pU)
- **X**: [Follow us on X](https://x.com/sockudorealtime)

**⭐ Star us on GitHub if Sockudo helps your project\! ⭐**

[GitHub Repository](https://github.com/RustNSparks/sockudo) • [Crates.io](https://crates.io/crates/sockudo))
