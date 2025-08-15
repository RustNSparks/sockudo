# Cleanup Queue Configuration Guide

The async cleanup queue system processes WebSocket disconnections in the background to prevent mass disconnections from blocking new connections. This guide explains how to configure it for different deployment scenarios.

## Overview

When WebSocket clients disconnect en masse (e.g., during network issues), the traditional synchronous cleanup can block new connections for minutes. The async cleanup queue solves this by:

1. **Fast Disconnect Detection** - Marks disconnected clients immediately (<1ms)
2. **Background Processing** - Processes cleanup operations in batches
3. **Non-blocking Architecture** - New connections proceed while cleanup runs

## Configuration Options

### Config File (`config.json`)
```json
{
  "cleanup": {
    "async_enabled": true,
    "fallback_to_sync": true,
    "queue_buffer_size": 2000,
    "batch_size": 25,
    "batch_timeout_ms": 50,
    "worker_threads": 1,
    "max_retry_attempts": 2
  }
}
```

### Environment Variables (Override Config File)
```bash
CLEANUP_ASYNC_ENABLED=true
CLEANUP_FALLBACK_TO_SYNC=true
CLEANUP_QUEUE_BUFFER_SIZE=2000
CLEANUP_BATCH_SIZE=25
CLEANUP_BATCH_TIMEOUT_MS=50
CLEANUP_WORKER_THREADS=auto  # or specific number like "2"
CLEANUP_MAX_RETRY_ATTEMPTS=2
```

## Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `async_enabled` | `true` | Enable/disable async cleanup globally |
| `fallback_to_sync` | `true` | Fall back to sync cleanup if queue fails |
| `queue_buffer_size` | `2000` | Maximum queued disconnect tasks per worker |
| `batch_size` | `25` | Tasks processed per batch per worker |
| `batch_timeout_ms` | `50` | Max wait time to fill batch per worker |
| `worker_threads` | `1` | Number of cleanup worker threads (or "auto") |
| `max_retry_attempts` | `2` | Retries before giving up per worker |

### Worker Threads Configuration

The `worker_threads` setting supports two formats:

**Fixed Number**: Specify exact number of workers
```json
"worker_threads": 2
```
```bash
CLEANUP_WORKER_THREADS=2
```

**Auto-Detection**: Use "auto" to automatically scale based on CPU cores
```json
"worker_threads": "auto"
```
```bash
CLEANUP_WORKER_THREADS=auto
```

When using "auto", the system uses **25% of available CPU cores** (minimum 1, maximum 4):
- 1-3 CPUs → 1 worker
- 4-7 CPUs → 1 worker  
- 8-11 CPUs → 2 workers
- 12-15 CPUs → 3 workers
- 16+ CPUs → 4 workers

**Multiple Workers**: When using multiple workers, each worker operates with the configured `batch_size` independently. Work is distributed among workers using round-robin scheduling for optimal load balancing.

### Per-Worker Configuration

**Important**: All configuration values (except `worker_threads`) are applied **per worker**, not as total system capacity.

**Example**: With `worker_threads: 2` and `batch_size: 50`:
- **Each worker** processes batches of 50 tasks
- **Total system capacity** = 2 workers × 50 tasks = 100 tasks per batch round
- **Queue capacity per worker** = `queue_buffer_size` value
- **Total queue capacity** = 2 workers × `queue_buffer_size` value

This design allows for:
- **Predictable scaling**: Adding workers increases total capacity proportionally
- **Consistent performance**: Each worker maintains the same performance characteristics
- **Easy tuning**: Configuration directly reflects individual worker behavior

## Deployment Scenario Configurations

### 1. Small Deployment (1vCPU/1GB RAM)
**Use Case**: Development, testing, small production instances
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 500,
    "batch_size": 10,
    "batch_timeout_ms": 100,
    "worker_threads": 1,
    "max_retry_attempts": 1
  }
}
```
- **Memory Usage**: ~4KB queue buffer per worker (total: ~4KB with 1 worker)
- **CPU Impact**: Minimal (1 worker thread)
- **Latency**: 100ms max cleanup delay per worker

### 2. Standard Deployment (2vCPU/2GB RAM) ⭐ **RECOMMENDED DEFAULT**
**Use Case**: Most common production deployments
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 2000,
    "batch_size": 25,
    "batch_timeout_ms": 50,
    "worker_threads": 1,
    "max_retry_attempts": 2
  }
}
```
- **Memory Usage**: ~16KB queue buffer per worker (total: ~16KB with 1 worker)
- **CPU Impact**: Low (1 worker, leaves CPU for main server)
- **Latency**: 50ms max cleanup delay per worker

### 3. High-Traffic Deployment (4vCPU/4GB+ RAM)
**Use Case**: High concurrent connection loads (>10K connections)
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 10000,
    "batch_size": 100,
    "batch_timeout_ms": 25,
    "worker_threads": 2,
    "max_retry_attempts": 3
  }
}
```
- **Memory Usage**: ~80KB queue buffer per worker (total: ~160KB with 2 workers)
- **CPU Impact**: Moderate (2 workers)
- **Latency**: 25ms max cleanup delay per worker

### 4. Ultra High-Traffic Deployment (8vCPU/8GB+ RAM)
**Use Case**: Massive scale deployments (>50K connections)
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 50000,
    "batch_size": 500,
    "batch_timeout_ms": 10,
    "worker_threads": 4,
    "max_retry_attempts": 3
  }
}
```
- **Memory Usage**: ~400KB queue buffer per worker (total: ~1.6MB with 4 workers)
- **CPU Impact**: High (4 workers)
- **Latency**: 10ms max cleanup delay per worker

### 5. Real-time Critical Applications
**Use Case**: Applications requiring immediate presence updates
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 1000,
    "batch_size": 5,
    "batch_timeout_ms": 5,
    "worker_threads": 2,
    "max_retry_attempts": 2
  }
}
```
- **Memory Usage**: ~8KB queue buffer per worker (total: ~16KB with 2 workers)
- **CPU Impact**: Moderate (2 workers, frequent small batches)
- **Latency**: 5ms max cleanup delay per worker

### 6. Memory-Constrained Deployment
**Use Case**: Limited memory environments
```json
{
  "cleanup": {
    "async_enabled": true,
    "queue_buffer_size": 100,
    "batch_size": 5,
    "batch_timeout_ms": 200,
    "worker_threads": 1,
    "max_retry_attempts": 1
  }
}
```
- **Memory Usage**: ~800B queue buffer per worker (total: ~800B with 1 worker)
- **CPU Impact**: Minimal (1 worker)
- **Latency**: 200ms max cleanup delay per worker

## Measuring Configuration Effectiveness

### 1. Key Performance Metrics

Monitor these metrics to validate your configuration:

#### **Queue Health**
```bash
# Check queue size via logs
grep "Cleanup batch completed" /var/log/sockudo.log | tail -10

# Expected: Consistent processing, no queue overflow warnings
```

#### **Connection Establishment Latency**
```bash
# Test new connection speed during mass disconnects
time curl -H "Connection: Upgrade" -H "Upgrade: websocket" ws://localhost:6001/app/app-id

# Target: <100ms even during mass disconnections
```

#### **Cleanup Processing Rate**
```bash
# Monitor tasks/second from logs
grep "tasks/sec" /var/log/sockudo.log | tail -5

# Expected: Rate matches your batch_size / batch_timeout_ms
```

### 2. Monitoring Commands

#### **Queue Status Check**
```bash
# Environment variable for instant toggle
export CLEANUP_ASYNC_ENABLED=false  # Emergency disable
systemctl reload sockudo

# Re-enable after issue resolved
export CLEANUP_ASYNC_ENABLED=true
systemctl reload sockudo
```

#### **Real-time Monitoring**
```bash
# Watch cleanup performance
tail -f /var/log/sockudo.log | grep -E "(Cleanup batch|Queue.*full|Failed to queue)"

# Watch for queue overflow warnings
tail -f /var/log/sockudo.log | grep "queue.*capacity"
```

### 3. Health Indicators

#### ✅ **Healthy Configuration**
- Cleanup batches process consistently
- Queue never reaches 80% capacity
- New connections establish <100ms during mass disconnects
- No "Failed to queue" errors
- Tasks/second rate is stable

#### ⚠️ **Needs Tuning**
- Queue frequently near capacity (>80%)
- Batch timeout frequently reached before batch fills
- Cleanup processing rate is inconsistent
- Memory usage growing over time

#### 🚨 **Critical Issues**
- "Queue full" errors in logs
- New connections timing out during disconnects
- "Failed to queue cleanup" errors
- Falling back to sync cleanup frequently

### 4. Load Testing

#### **Mass Disconnection Test**
```bash
# Simulate 1000 simultaneous disconnects
for i in {1..1000}; do
  echo "Disconnecting client $i"
  pkill -f "websocket-client-$i" &
done

# Measure new connection latency during cleanup
time wscat -c ws://localhost:6001/app/app-id
```

#### **Queue Saturation Test**
```bash
# Test queue limits by reducing buffer size temporarily
export CLEANUP_QUEUE_BUFFER_SIZE=10  # Very small for testing
systemctl reload sockudo

# Generate load and watch for queue overflow
./load-test-script.sh

# Restore normal buffer size
unset CLEANUP_QUEUE_BUFFER_SIZE
systemctl reload sockudo
```

## Troubleshooting

### Common Issues

#### **Queue Filling Up**
**Symptoms**: "queue near capacity" warnings
**Solutions**:
- Increase `queue_buffer_size`
- Decrease `batch_timeout_ms` (faster processing)
- Increase `batch_size` (more per batch)
- Add more `worker_threads`

#### **High Memory Usage**
**Symptoms**: Memory growth during high disconnect volume
**Solutions**:
- Decrease `queue_buffer_size`
- Decrease `batch_size`
- Check for memory leaks in webhooks

#### **Slow Webhook Delivery**
**Symptoms**: Webhook delays during mass disconnects
**Solutions**:
- Webhooks are processed async - this is expected
- Increase `max_retry_attempts` if webhooks are failing
- Monitor webhook endpoint performance

#### **Cleanup Not Working**
**Symptoms**: Connections not being cleaned up
**Solutions**:
1. Check `async_enabled` is `true`
2. Verify worker threads are running: `grep "Cleanup worker started" /var/log/sockudo.log`
3. Enable fallback: `CLEANUP_FALLBACK_TO_SYNC=true`

### Emergency Procedures

#### **Disable Async Cleanup (Emergency)**
```bash
# Immediate disable via environment variable
export CLEANUP_ASYNC_ENABLED=false
systemctl reload sockudo
# Or restart the service for immediate effect
systemctl restart sockudo
```

#### **Drain Queue Gracefully**
```bash
# Let existing queue process, but disable new queuing
export CLEANUP_ASYNC_ENABLED=false
# Wait for queue to drain (check logs)
grep "Cleanup worker shutting down" /var/log/sockudo.log
# Then restart with proper config
systemctl restart sockudo
```

## Best Practices

1. **Start Conservative**: Begin with standard deployment settings and tune up
2. **Monitor Actively**: Watch queue health during initial deployment
3. **Test Load**: Run mass disconnection tests before production
4. **Have Emergency Plan**: Know how to quickly disable async cleanup
5. **Environment Variables**: Use env vars for quick production tuning
6. **Gradual Changes**: Adjust one parameter at a time when tuning

## Configuration Examples by Server Size

| Server Spec | queue_buffer_size | batch_size | batch_timeout_ms | worker_threads |
|-------------|-------------------|------------|------------------|----------------|
| 1vCPU/1GB   | 500              | 10         | 100              | 1              |
| 2vCPU/2GB   | 2000             | 25         | 50               | 1              |
| 4vCPU/4GB   | 10000            | 100        | 25               | 2              |
| 8vCPU/8GB   | 50000            | 500        | 10               | 4              |

Remember: These are starting points. Monitor and adjust based on your specific traffic patterns and performance requirements.