# Origin Validation Feature

## Overview

The origin validation feature provides an additional security layer for WebSocket connections by allowing per-app configuration of allowed origins. This feature works alongside CORS to restrict which domains can establish WebSocket connections to specific Pusher apps.

## Configuration Methods

The `allowed_origins` field can be configured through multiple methods depending on your app management driver.

### 1. Memory/Array Configuration (JSON)

Add the `allowed_origins` field to your app configuration in `config.json`:

```json
{
  "app_manager": {
    "driver": "memory",
    "array": {
      "apps": [
        {
          "id": "my-app",
          "key": "my-app-key",
          "secret": "my-app-secret",
          "allowed_origins": [
            "https://app.example.com",
            "https://*.staging.example.com",
            "http://localhost:3000"
          ]
        }
      ]
    }
  }
}
```

### 2. Environment Variables (Default App)

For the default app, configure allowed origins via environment variable:

```bash
SOCKUDO_DEFAULT_APP_ALLOWED_ORIGINS=https://app.example.com,*.staging.example.com,http://localhost:3000
```

### 3. MySQL Database Configuration

Store apps in MySQL with the `allowed_origins` JSON column:

```sql
-- Create/update an app with allowed origins
INSERT INTO applications (
  id, `key`, secret, max_connections, 
  enable_client_messages, enabled, 
  max_client_events_per_second, allowed_origins
) VALUES (
  'my-app', 'my-app-key', 'my-app-secret', 1000,
  true, true, 100,
  JSON_ARRAY('https://app.example.com', '*.staging.example.com', 'http://localhost:3000')
) ON DUPLICATE KEY UPDATE allowed_origins = VALUES(allowed_origins);
```

Run the migration to add the column to existing tables:
```bash
mysql -u username -p database_name < migrations/mysql/001_add_allowed_origins.sql
```

### 4. PostgreSQL Database Configuration

Store apps in PostgreSQL with the `allowed_origins` JSONB column:

```sql
-- Create/update an app with allowed origins
INSERT INTO applications (
  id, key, secret, max_connections, 
  enable_client_messages, enabled, 
  max_client_events_per_second, allowed_origins
) VALUES (
  'my-app', 'my-app-key', 'my-app-secret', 1000,
  true, true, 100,
  '["https://app.example.com", "*.staging.example.com", "http://localhost:3000"]'::jsonb
) ON CONFLICT (id) DO UPDATE SET allowed_origins = EXCLUDED.allowed_origins;
```

Run the migration to add the column to existing tables:
```bash
psql -U username -d database_name -f migrations/postgresql/001_add_allowed_origins.sql
```

### 5. DynamoDB Configuration

Store the allowed origins as a List attribute in DynamoDB:

```json
{
  "id": { "S": "my-app" },
  "key": { "S": "my-app-key" },
  "secret": { "S": "my-app-secret" },
  "allowed_origins": {
    "L": [
      { "S": "https://app.example.com" },
      { "S": "*.staging.example.com" },
      { "S": "http://localhost:3000" }
    ]
  }
}
```

Using AWS CLI:
```bash
aws dynamodb put-item \
    --table-name sockudo-applications \
    --item '{
        "id": {"S": "my-app"},
        "key": {"S": "my-app-key"},
        "secret": {"S": "my-app-secret"},
        "allowed_origins": {
            "L": [
                {"S": "https://app.example.com"},
                {"S": "*.staging.example.com"},
                {"S": "http://localhost:3000"}
            ]
        }
    }'
```

## Wildcard Support

The origin validation supports several wildcard patterns:

### 1. Allow All Origins
```json
"allowed_origins": ["*"]
```

### 2. Subdomain Wildcards
```json
"allowed_origins": [
  "*.example.com",              // Matches any subdomain of example.com (any protocol)
  "https://*.example.com"        // Matches only HTTPS subdomains of example.com
]
```

### 3. Port Wildcards
```json
"allowed_origins": [
  "http://localhost:*"           // Matches localhost on any port
]
```

## Validation Rules

1. **Empty or Missing Configuration**: If `allowed_origins` is not configured or is empty, all origins are allowed (backward compatible).

2. **Exact Matching**: Origins must match exactly including protocol, domain, and port (if specified).

3. **Wildcard Matching**: 
   - `*.domain.com` matches any subdomain including nested subdomains
   - Wildcards without protocol match any protocol
   - Wildcards with protocol only match that specific protocol

4. **Missing Origin Header**: If the Origin header is missing, the connection is rejected when origin validation is configured.

## Error Response

When a connection is rejected due to origin validation, the client receives a Pusher protocol error:

```json
{
  "event": "pusher:error",
  "data": {
    "code": 4009,
    "message": "Origin not allowed"
  }
}
```

Error code 4009 indicates an unauthorized connection, following the Pusher protocol specification.

## Security Considerations

1. **Browser-Only Protection**: Origin headers can only be trusted from browser clients. Non-browser clients can spoof the Origin header.

2. **Defense in Depth**: This feature provides an additional security layer but should not be the only security measure.

3. **Works with CORS**: This feature complements CORS configuration and provides app-specific origin restrictions.

## Examples

### Example 1: Production App with Specific Origins

```json
{
  "id": "production-app",
  "allowed_origins": [
    "https://www.myapp.com",
    "https://app.myapp.com"
  ]
}
```

### Example 2: Development App with Localhost

```json
{
  "id": "dev-app",
  "allowed_origins": [
    "http://localhost:3000",
    "http://localhost:3001",
    "http://127.0.0.1:3000"
  ]
}
```

### Example 3: Staging with Wildcard Subdomains

```json
{
  "id": "staging-app",
  "allowed_origins": [
    "*.staging.myapp.com",
    "https://preview-*.myapp.com"
  ]
}
```

## Testing

### Unit Tests
Run the origin validation unit tests:
```bash
cargo test origin_validation
```

### Integration Tests
The integration tests require setting up test apps with specific origin configurations:

```javascript
// test/automated/origin_validation.js
npm install
node origin_validation.js
```

## Migration Guide

### From No Origin Validation

1. Start with logging mode - configure allowed origins but monitor rejected connections
2. Gradually add all legitimate origins to the configuration
3. Enable strict validation once all origins are identified

### Backward Compatibility

- Apps without `allowed_origins` configured continue to work as before
- No changes required for existing deployments
- Origin validation is opt-in per app