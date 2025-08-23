# Metrics Lock Contention Analysis & Solution Plan

## Issue Analysis

During connection establishment and channel subscriptions, I've identified that **metrics operations are creating lock contention** that blocks new connections. The flow shows:

1. **Connection establishment** (in `handle_websocket_upgrade`):
   - Connection added to namespace
   - **Metrics lock acquired** → `mark_new_connection()`
   - Each connection waits for previous metrics update

2. **Channel subscription** (in `handle_subscription`):
   - Channel subscription processed
   - **Metrics lock acquired** → `mark_channel_subscription()`
   - Log message: "Metrics: Channel subscription for app app-id, channel type: public"
   - This blocks the next connections from proceeding

## Root Cause

The metrics system uses a **shared Mutex** that is locked for EVERY operation:
- New connection → lock metrics → update → unlock
- Channel subscription → lock metrics → update → unlock
- Disconnection → lock metrics → update → unlock

With 10k connections subscribing to channels, this creates **10k sequential lock acquisitions** for metrics updates.

## Solution Options

### Option 1: Batch Metrics Updates (Recommended)
- Queue metrics updates in a lock-free structure
- Process them in batches asynchronously
- Similar to the cleanup worker pattern

### Option 2: Lock-Free Metrics (Alternative)
- Use atomic counters for metrics
- Eliminate the mutex entirely
- Use `AtomicU64` for counters, `Arc<DashMap>` for labeled metrics

### Option 3: Fire-and-Forget Metrics
- Use unbounded channel to send metrics updates
- Dedicated worker processes them asynchronously
- Zero blocking on the main connection path

## Recommended Implementation Plan

### 1. Create Metrics Queue System
```rust
// Add to metrics module
pub struct MetricsQueue {
    sender: mpsc::UnboundedSender<MetricsEvent>,
}

pub enum MetricsEvent {
    NewConnection { app_id: String, socket_id: SocketId },
    Disconnection { app_id: String, socket_id: SocketId },
    ChannelSubscription { app_id: String, channel_type: String },
    // ... other events
}
```

### 2. Replace Synchronous Metrics Calls
Instead of:
```rust
let metrics_locked = metrics.lock().await;
metrics_locked.mark_channel_subscription(&app_config.id, channel_type_str);
```

Use:
```rust
metrics_queue.send(MetricsEvent::ChannelSubscription {
    app_id: app_config.id.clone(),
    channel_type: channel_type_str.to_string(),
});
```

### 3. Process Metrics Asynchronously
- Dedicated worker consumes the queue
- Batches updates for efficiency
- Updates actual metrics without blocking connections

## Expected Impact

- **Connection establishment**: No metrics blocking
- **Channel subscriptions**: Instant, no lock contention
- **Performance**: Near-zero overhead for metrics
- **10k connections on 1 channel**: Should connect as fast as multi-channel scenario

## Files to Modify

1. `src/metrics/mod.rs` - Add queue system
2. `src/metrics/queue.rs` - New file for metrics queue worker
3. `src/adapter/handler/mod.rs` - Replace metrics lock calls
4. `src/adapter/handler/subscription_management.rs` - Replace metrics lock calls
5. `src/adapter/handler/core.rs` - Replace metrics lock calls
6. `src/main.rs` - Initialize metrics queue worker

## Testing Plan

1. Load test with 10k connections on single channel
2. Monitor connection rate with/without metrics
3. Verify metrics accuracy with batched updates
4. Test cleanup and shutdown scenarios

## Notes

This issue was discovered while investigating single-channel vs multi-channel disconnection performance. The metrics lock contention affects both connection and disconnection flows, creating significant bottlenecks when thousands of clients operate on the same resources simultaneously.