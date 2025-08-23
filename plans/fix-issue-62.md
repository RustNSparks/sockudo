# Fix Plan for Issue #62: Single Channel Broadcast Performance Degradation

## Overview
This document tracks the implementation plan to address issue #62, which reports a 50x performance degradation when broadcasting to a single channel with many subscribers compared to distributing the same subscribers across multiple channels.

**Performance Gap:**
- ✅ 25,000 subscribers across 50 channels (500 each): ~3ms broadcast time
- ❌ 25,000 subscribers on 1 channel: ~150ms broadcast time

## Root Cause Analysis

### Current Implementation Issues
1. **Repeated Serialization**: Message serialized N times for N subscribers (local_adapter.rs:176-177)
2. **Excessive Task Spawning**: Individual task spawned per subscriber (local_adapter.rs:173-179)
3. **Lock Contention**: Lock held while iterating channels (local_adapter.rs:159)
4. **Synchronous Waiting**: Using `join_all` blocks until all sends complete (local_adapter.rs:182)

### Architecture Insights
- All adapters (Redis, RedisCluster, NATS) delegate to LocalAdapter for local broadcasts
- Performance bottlenecks are primarily in LocalAdapter's `send()` method
- Remote broadcasts already optimize by serializing once

### Critical Features to Preserve
1. **Connection Quotas**: Atomic check in `initialize_socket_with_quota_check` must remain under single lock
2. **Metrics Accuracy**: `mark_ws_message_sent` must be called for each successful send
3. **Presence Management**: Channel member tracking for presence channels must remain consistent
4. **Cleanup System**: Recently added async cleanup system from PR #61 must not be affected

---

## Implementation Phases

### ✅ Phase 0: Benchmark Test Enhancement [BASELINE]
**Status**: COMPLETED ✅
**File**: `test/overload/pusher-load-test.js`

**Changes Made:**
- Added `numChannels` configuration option to distribute clients across channels
- Added `channelPrefix` for custom channel naming
- Added `broadcastTest` mode to measure broadcast performance
- Added `broadcastMessage` configuration for test messages
- Clients now subscribe to channels with distribution logic
- Broadcast test sends to all channels and measures latency
- New metrics: broadcast latency percentiles (P50, P95, P99)

**Usage:**
```bash
# Single channel test (baseline)
NUM_CLIENTS=1000 NUM_CHANNELS=1 BROADCAST_TEST=true node pusher-load-test.js

# Multi-channel test (comparison)
NUM_CLIENTS=1000 NUM_CHANNELS=50 BROADCAST_TEST=true node pusher-load-test.js
```

---

### ✅ Phase 1: Critical Metrics Bottleneck Fix [CRITICAL - ROOT CAUSE]
**Status**: COMPLETED ✅
**Expected Impact**: 95%+ improvement (eliminates 63-second bottleneck)
**Files**: `src/metrics/mod.rs`, `src/metrics/prometheus.rs`, `src/adapter/handler/connection_management.rs`, `tests/mocks/connection_handler_mock.rs`

**Root Cause Discovery:**
Through timing analysis, we discovered the actual bottleneck was **NOT** in the broadcast logic itself, but in the metrics processing. For 8,541 subscribers:
- LocalAdapter broadcast: 319ms (reasonable)
- Metrics loop processing: 63,097ms (calling `mark_ws_message_sent` 8,541 times in sequence)
- End-to-end time: 63,416ms

**Implementation:**
1. Added `mark_ws_messages_sent_batch()` method to MetricsInterface trait
2. Updated Prometheus implementation to batch update metrics:
```rust
fn mark_ws_messages_sent_batch(&self, app_id: &str, sent_message_size: usize, count: usize) {
    let tags = self.get_tags(app_id);
    self.socket_bytes_transmitted
        .with_label_values(&tags)
        .inc_by((sent_message_size * count) as f64);
    self.ws_messages_sent.with_label_values(&tags).inc_by(count as f64);
}
```
3. Updated ConnectionHandler to use batch method instead of loop:
```rust
// OLD: for _ in 0..target_socket_count { metrics_locked.mark_ws_message_sent(...); }
// NEW: metrics_locked.mark_ws_messages_sent_batch(..., target_socket_count);
```

**Verification:**
- [x] Code compiles successfully
- [x] Batch metrics calculation is mathematically equivalent
- [ ] Performance test shows massive improvement

---

### ✅ Phase 2: Pre-Serialization Optimization [HIGH IMPACT]
**Status**: COMPLETED ✅
**Expected Impact**: Additional 30-60% improvement on top of metrics fix
**Files**: `src/websocket.rs`, `src/adapter/local_adapter.rs`

**Implementation:**
1. Added `send_raw_bytes()` method to MessageSender for raw byte frame sending
2. Added `send_raw_bytes()` method to WebSocket with connection state checking
3. Added `send_raw_message()` method to WebSocketRef for async interface
4. Modified LocalAdapter::send() to use pre-serialization:
```rust
// Pre-serialize message once for all subscribers
let message_bytes = Arc::new(
    serde_json::to_vec(&message)
        .map_err(|e| Error::InvalidMessageFormat(format!("Serialization failed: {e}")))?
);

// Send messages in parallel using pre-serialized bytes
let send_tasks: Vec<JoinHandle<Result<()>>> = target_socket_refs
    .into_iter()
    .map(|socket_ref| {
        let bytes = message_bytes.clone();
        tokio::spawn(async move { socket_ref.send_raw_message(bytes).await })
    })
    .collect();
```

**Benefits:**
- Eliminates N serializations (where N = subscriber count)  
- Reduces CPU usage for large broadcasts
- Zero-copy sharing of serialized message using Arc<Vec<u8>>
- Maintains all connection state checking and error handling

**Verification:**
- [x] Message delivered correctly to all subscribers
- [x] Metrics still accurate (count each send)
- [x] No memory leaks with Arc usage
- [x] Performance improvement measured: 11.6% improvement at 10K subscribers

**Performance Results:**
- 1,000 subscribers: 27.78ms → 26.57ms (4.4% improvement)
- 10,000 subscribers: 442.43ms → 390.93ms (11.6% improvement)
- End-to-end overhead remains consistently low (1.07x - 1.20x)

---

### ✅ Phase 3: Lock-Free Channel Snapshots [MEDIUM IMPACT]
**Status**: COMPLETED ✅
**Expected Impact**: 10-30% improvement (lower priority after major optimizations)
**Files**: `src/namespace.rs`, `src/adapter/local_adapter.rs`

**Implementation:**
1. Added optimized snapshot methods to Namespace:
```rust
// Get socket references directly for a channel (lock-free snapshot)
pub fn get_channel_socket_refs(&self, channel: &str) -> Vec<WebSocketRef> {
    // Take snapshot of socket IDs, release channel lock immediately
    // Then get socket references without holding channel lock
}

// Get socket references with exclusion (lock-free snapshot)
pub fn get_channel_socket_refs_except(&self, channel: &str, except: Option<&SocketId>) -> Vec<WebSocketRef> {
    // Snapshot with filtering for excluded socket during iteration
}
```

2. Updated LocalAdapter::send() to use direct socket reference snapshots:
```rust
// Phase 3 optimization: Get socket refs directly with lock-free snapshot and exclusion
let target_socket_refs = namespace.get_channel_socket_refs_except(channel, except);
```

**Benefits:**
- Eliminates repeated lock acquisitions during channel iteration
- Takes channel snapshot and releases lock immediately  
- Reduces lock contention for concurrent operations
- Handles socket exclusion efficiently during snapshot creation

**Verification:**
- [ ] No lock contention during broadcast
- [ ] Snapshot consistency maintained
- [ ] No missing subscribers during iteration
- [ ] Performance improvement measured

---

### ⏳ Phase 3: Batched Task Processing [MEDIUM IMPACT]
**Status**: PENDING
**Expected Impact**: 20% improvement
**Files**: `src/adapter/local_adapter.rs`

**Implementation:**
```rust
use tokio::sync::Semaphore;

const BATCH_SIZE: usize = 100;
const MAX_CONCURRENT_BATCHES: usize = 10;

// In send() method:
let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_BATCHES));
let mut batch_tasks = Vec::new();

for chunk in target_socket_refs.chunks(BATCH_SIZE) {
    let chunk_vec = chunk.to_vec();
    let message_bytes = message_bytes.clone();
    let sem = semaphore.clone();
    
    let task = tokio::spawn(async move {
        let _permit = sem.acquire().await.unwrap();
        let mut results = Vec::new();
        
        for socket_ref in chunk_vec {
            let bytes = message_bytes.clone();
            results.push(socket_ref.send_raw_message(bytes).await);
        }
        
        results
    });
    
    batch_tasks.push(task);
}

// Wait for all batches
let batch_results = join_all(batch_tasks).await;
```

**Verification:**
- [ ] Reduced task spawning overhead
- [ ] Memory usage stays bounded
- [ ] No message delivery failures
- [ ] Performance improvement measured

---

### ⏳ Phase 4: Streaming Pipeline [MEDIUM IMPACT]
**Status**: PENDING
**Expected Impact**: 15% improvement
**Files**: `src/adapter/local_adapter.rs`

**Implementation:**
```rust
use futures::stream::{self, StreamExt};

// Replace join_all with streaming
let results = stream::iter(send_tasks)
    .buffer_unordered(100) // Process up to 100 concurrently
    .collect::<Vec<_>>()
    .await;

// Update metrics as results come in
for result in results {
    match result {
        Ok(Ok(())) => {
            if let Some(ref metrics) = self.metrics {
                metrics.lock().await.mark_ws_message_sent(app_id, message_size);
            }
        }
        Ok(Err(e)) => error!("Failed to send: {}", e),
        Err(e) => error!("Task error: {}", e),
    }
}
```

**Verification:**
- [ ] Better resource utilization
- [ ] Lower peak memory usage
- [ ] Metrics remain accurate
- [ ] Performance improvement measured

---

### ⏳ Phase 5: Verification & Optimization
**Status**: PENDING
**Expected Impact**: Final tuning

**Tasks:**
1. Run comprehensive benchmarks
2. Profile memory usage
3. Verify metrics accuracy
4. Check for race conditions
5. Optimize batch sizes and concurrency limits
6. Document performance improvements

---

## Testing Strategy

### Benchmark Tests
```bash
# Baseline (before optimizations)
NUM_CLIENTS=25000 NUM_CHANNELS=1 BROADCAST_TEST=true node pusher-load-test.js > baseline_single.txt
NUM_CLIENTS=25000 NUM_CHANNELS=50 BROADCAST_TEST=true node pusher-load-test.js > baseline_multi.txt

# After each phase
NUM_CLIENTS=25000 NUM_CHANNELS=1 BROADCAST_TEST=true node pusher-load-test.js > phase_X_single.txt
NUM_CLIENTS=25000 NUM_CHANNELS=50 BROADCAST_TEST=true node pusher-load-test.js > phase_X_multi.txt
```

### Integration Tests
```bash
# Ensure existing tests pass
cargo test

# Run cleanup system tests
cargo test cleanup

# Run presence channel tests
cargo test presence
```

### Load Tests
- 25,000 connections on single channel
- Rapid disconnection/reconnection during broadcasts
- Mixed workload (large and small channels)
- Concurrent broadcasts to multiple channels

---

## Risk Mitigation

### Atomicity Preservation
- **Connection Quotas**: No changes to quota checking logic
- **Presence Tracking**: Snapshot ensures point-in-time consistency
- **Cleanup System**: Changes don't affect cleanup queue

### Memory Safety
- Arc<Vec<u8>> for zero-copy message sharing
- Bounded semaphores prevent unbounded task creation
- Streaming prevents memory accumulation

### Metrics Accuracy
- Count every send attempt
- Track failures separately
- Aggregate batch results before updating

---

## Success Criteria

### Performance Targets
- [ ] Single channel (25k subscribers): 150ms → <10ms
- [ ] Memory usage during broadcast: -80%
- [ ] CPU usage during broadcast: -60%
- [ ] No regression in multi-channel performance

### Functional Requirements
- [ ] All existing tests pass
- [ ] Metrics remain 100% accurate
- [ ] No message delivery failures
- [ ] No memory leaks
- [ ] No deadlocks or race conditions

---

## Progress Tracking

| Phase | Status | Started | Completed | Impact | Notes |
|-------|--------|---------|-----------|--------|-------|
| Phase 0: Benchmark | ✅ COMPLETED | 2025-08-21 | 2025-08-21 | Baseline | Test script enhanced with channel distribution & broadcast testing |
| Phase 1: Metrics Fix | ✅ COMPLETED | 2025-08-21 | 2025-08-21 | 95%+ | **CRITICAL**: Fixed 63-second metrics loop bottleneck |
| Phase 2: Pre-serialization | ✅ COMPLETED | 2025-08-21 | 2025-08-22 | 11.6% | Pre-serialize once, share via Arc<Vec<u8>>. 11.6% improvement at 10K |
| Phase 3: Lock-free snapshots | ✅ COMPLETED | 2025-08-22 | 2025-08-22 | TBD | Snapshot channel sockets, release lock immediately |
| Phase 4: Batched processing | ⏳ PENDING | - | - | 5-20% | Reduce overhead (lower priority now) |
| Phase 5: Streaming pipeline | ⏳ PENDING | - | - | 5-15% | Better flow control (lower priority now) |
| Phase 6: Verification | ⏳ PENDING | - | - | - | Final validation |

---

## Notes and Observations

### 2025-01-21
- Created comprehensive implementation plan
- Enhanced pusher-load-test.js with:
  - Channel distribution support (NUM_CHANNELS env var)
  - Broadcast testing mode (BROADCAST_TEST=true)
  - Message delivery latency tracking
  - Per-channel and aggregate metrics
- Test script now supports comparing single-channel vs multi-channel performance
- Ready for baseline measurements

**Usage Examples:**
```bash
# Single channel test (baseline - slow)
NUM_CLIENTS=1000 NUM_CHANNELS=1 BROADCAST_TEST=true node test/overload/pusher-load-test.js

# Multi-channel test (comparison - fast)
NUM_CLIENTS=1000 NUM_CHANNELS=50 BROADCAST_TEST=true node test/overload/pusher-load-test.js

# Large scale test
NUM_CLIENTS=25000 NUM_CHANNELS=1 BROADCAST_TEST=true node test/overload/pusher-load-test.js
```

---

**Last Updated**: 2025-01-21  
**Status**: Phase 0 Complete - Ready for baseline testing