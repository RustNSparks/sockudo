# Ongoing Work: Broadcast Performance Optimization

## Context

Working on the `feat/broadcast-performance-improvements` branch to address a critical 100% latency regression in broadcast performance for 20k+ connections that was introduced in PR #61. The branch had successfully improved performance for all test scenarios except the 20k connection test (282ms vs 144ms on master).

**Key Background:**
- WebSocket server (Sockudo) implementing Pusher protocol
- Lock-free broadcasting architecture already implemented using BytesMut for zero-copy message sharing
- Main bottleneck: Multi-channel broadcasting causing multiplicative CPU overload
- Goal: Fix 40-channel test failures while maintaining single-channel performance gains

## Problem Identified

**Root Issue:** Per-broadcast buffer sizing causing multiplicative CPU overload in multi-channel scenarios:
- 40 channels × 500 connections each = 20,000 concurrent tasks
- System capacity: 8 cores × 64 = 512 tasks
- Result: 39x CPU overload (133%+ CPU usage, 0% delivery rate on 40ch test)

**Original broken test results:**
```
40ch_20000clients_2500batch: 0% delivery rate, 133% CPU
```

## Approaches Tried

### 1. Global Semaphore with Chunked Processing (First Fix)
**Implementation:** Added global `Arc<Semaphore>` to LocalAdapter with chunked processing using `acquire_many()`.

**Results:**
```
40ch_20000clients_2500batch: 100% delivery, 92.69% CPU, 28.32ms latency
```
- ✅ **Excellent performance** - best results achieved
- ✅ **100% delivery rate** restored
- ✅ **Optimal CPU usage** 
- ❌ **Complex code** - chunking calculations and buffer size logic

### 2. Tokio Spawn Individual Tasks (Simplification Attempt)
**Implementation:** Replaced chunking with `tokio::spawn()` for each individual connection using global semaphore.

**Results:**
```
40ch_20000clients_2500batch: 100% delivery, 323.55% CPU, 92.11ms latency
```
- ❌ **CPU explosion** - 3.5x worse CPU usage (323% vs 92%)
- ❌ **Latency degradation** - 3.2x worse latency (92ms vs 28ms) 
- ❌ **Task scheduler overwhelm** - 20,000 simultaneous spawned tasks
- ✅ **Simple code** but terrible performance

### 3. Adaptive Batch Spawning (Current Implementation)
**Implementation:** Hybrid approach using adaptive batch sizes:
```rust
let batch_size = if socket_count <= 50 {
    socket_count // Single batch for small broadcasts
} else if socket_count <= 1000 {
    32 // Efficient batching for medium broadcasts  
} else {
    64 // Larger batches for big broadcasts
};
```
Spawn batch tasks using `acquire_many()` with ~312 tasks for 40ch scenario.

**Preliminary Results (20k tests only):**
```
1ch_20000clients: 100% delivery, 128.69% CPU, 55.57ms latency
20ch_20000clients: 100% delivery, 125.99% CPU, 17.71ms latency  
40ch_20000clients: 100% delivery, 135.03% CPU, 32.90ms latency
```

**Analysis:**
- ✅ **Major improvement** over tokio spawn approach
- ✅ **CPU controlled** (135% vs 323%)
- ✅ **Good latency** but 16% regression vs best solution (32.90ms vs 28.32ms)
- ✅ **Much simpler code** than approach #1
- ⚠️ **Performance trade-off** - accepting regression for code simplicity

## Current State

**Branch:** `feat/broadcast-performance-improvements`  
**Current commit:** HEAD (adaptive batching implementation)  
**Best performance commit:** 4c574afea848880bf9ba6e1edaf1f7f4a747b5b4 (approach #1)

**Code locations:**
- `src/adapter/local_adapter.rs` - Main implementation with adaptive batching
- `src/websocket.rs` - BytesMut integration for zero-copy messaging
- `src/options.rs` - Configuration with `buffer_multiplier_per_cpu` (default: 64)

**Pending:** Full test suite running for current adaptive batching approach across all scenarios (1ch, 2ch, 5ch, 10ch, 20ch, 40ch).

## Decision Point

**Options:**
1. **Keep adaptive batching** if full results show consistent 100% delivery and reasonable performance
2. **Tune batch sizes** (32→64, 64→128) to close performance gap with best solution  
3. **Revert to approach #1** (commit 4c574af) since it achieved optimal performance across all metrics

**Key consideration:** The previous solution (approach #1) was "insanely good" with no weaknesses:
- 28.32ms latency (vs current 32.90ms)
- 92.69% CPU (vs current 135.03% CPU)  
- 100% delivery rate across all tests
- Proven scaling across all channel counts

**Next steps:** Analyze full test results and decide whether the 16% latency regression and 46% CPU increase are acceptable trade-offs for code simplicity, or if reverting to the optimal solution makes more sense.

## Technical Details

**Global concurrency limit:** `cpu_cores * buffer_multiplier_per_cpu` (e.g., 8 × 64 = 512 max concurrent operations)
**Environment variable:** `ADAPTER_BUFFER_MULTIPLIER_PER_CPU=64`
**Lock-free architecture:** Uses BytesMut and dedicated broadcast channels to eliminate mutex contention

**Performance target:** Fix multi-channel CPU overload while maintaining single-channel improvements from the lock-free architecture.