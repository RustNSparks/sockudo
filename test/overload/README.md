# Sockudo Overload Tests

Enhanced performance and load testing tools for Sockudo WebSocket server, specifically designed to accurately measure cleanup system performance during mass reconnection scenarios.

## Features

### Accurate Measurements
- **True Server Processing Time**: Measures from WebSocket creation to `pusher:connection_established` event
- **Connection Validation**: Subscribes to channels to verify full server functionality
- **Detailed Timing Metrics**: P50, P95, P99 percentiles and connection time distribution
- **Batch Performance Tracking**: Measures performance characteristics of each connection batch

### Realistic Load Simulation
- **Background Traffic**: Optional ongoing message traffic to simulate real-world conditions
- **Connection Validation**: Percentage of clients subscribe to channels to test full functionality
- **Configurable Batch Processing**: Tunable batch sizes and delays to control connection rate

### Enhanced Monitoring
- **Real-time Progress**: Live updates of connection success/failure rates
- **Comprehensive Statistics**: Detailed breakdown of all performance metrics
- **Graceful Cleanup**: Proper WebSocket termination and resource cleanup

## Installation

```bash
cd test/overload
npm install
```

## Usage

### Basic Test (20k connections)
```bash
npm run test
```

### Predefined Test Sizes
```bash
# Small test (1k connections)
npm run test:small

# Medium test (5k connections) 
npm run test:medium

# Large test (20k connections)
npm run test:large

# Extra large test (50k connections)
npm run test:xl

# Validation test with background load
npm run test:validate
```

### Custom Configuration via Environment Variables

```bash
# Custom server URL
WS_URL=ws://localhost:6003/app/app-key npm run test

# Custom client count and batching
NUM_CLIENTS=10000 BATCH_SIZE=200 BATCH_DELAY=30 npm run test

# Enable connection validation and background load
VALIDATION_PERCENT=20 BACKGROUND_LOAD=true npm run test

# Full custom configuration
WS_URL=ws://localhost:6001/app/app-key \
NUM_CLIENTS=15000 \
BATCH_SIZE=300 \
BATCH_DELAY=40 \
VALIDATION_PERCENT=15 \
BACKGROUND_LOAD=true \
node mass-connection-test.js
```

## Configuration Options

| Variable | Default | Description |
|----------|---------|-------------|
| `WS_URL` | `ws://localhost:6001/app/app-key` | WebSocket server URL |
| `NUM_CLIENTS` | `20000` | Total number of clients to connect |
| `BATCH_SIZE` | `500` | Clients to connect simultaneously per batch |
| `BATCH_DELAY` | `50` | Milliseconds to wait between batches |
| `VALIDATION_PERCENT` | `10` | Percentage of clients to validate with subscriptions |
| `BACKGROUND_LOAD` | `false` | Enable ongoing message traffic during test |

## Understanding the Results

### Connection Metrics
- **Connected/Failed**: Success/failure counts
- **Total time**: Time from start to last connection established
- **Avg connection time**: Average time for `pusher:connection_established`
- **Connections per second**: Overall connection establishment rate

### Timing Distribution
- **Min/Max**: Fastest and slowest individual connections
- **P50/P95/P99**: Percentile distribution showing consistency
- **Connection span**: Time from first to last successful connection

### Validation Results
- **Validated**: Clients that successfully subscribed to channels
- **Validation time**: Time to complete channel subscription
- **Success rate**: Percentage of validation attempts that succeeded

## Testing Reconnection Scenarios

### Step 1: Initial Connection Test
```bash
# Start the test
npm run test

# Wait for all connections to establish
# Note the performance metrics
```

### Step 2: Force Disconnection
```bash
# Press Ctrl+C to terminate all clients
# This simulates mass disconnection (network failure, etc.)
```

### Step 3: Immediate Reconnection Test
```bash
# Immediately run the test again (within 5 seconds)
npm run test

# Compare reconnection performance to initial connection performance
# Key metrics to compare:
# - Connections per second (should be similar or better with async cleanup)
# - Average connection time (should not increase significantly)
# - P95/P99 latency (should remain consistent)
```

## Interpreting Performance Impact

### Successful Async Cleanup System
- **Reconnection rate** >= 90% of initial connection rate
- **P95 latency** increase < 50% during reconnection
- **Zero connection failures** during mass reconnection
- **Consistent batch performance** across reconnection tests

### Problematic Performance
- **Reconnection rate** < 70% of initial connection rate  
- **P95 latency** increase > 100% during reconnection
- **Connection timeouts** during reconnection phase
- **Degrading batch performance** in later batches

## System Requirements

### File Descriptors
```bash
# Check current limit
ulimit -n

# Set appropriate limit for large tests
ulimit -n 65536
```

### Memory Requirements
- **Small (1k)**: ~50MB Node.js heap
- **Medium (5k)**: ~200MB Node.js heap  
- **Large (20k)**: ~800MB Node.js heap
- **XL (50k)**: ~2GB Node.js heap

### Docker Testing
When testing against Dockerized Sockudo:
```bash
# Ensure Docker has sufficient resources
docker stats

# Monitor container resource usage during tests
docker exec -it <container> top
```

## Troubleshooting

### Connection Timeouts
- Increase `ulimit -n` on both client and server machines
- Reduce `BATCH_SIZE` and increase `BATCH_DELAY`
- Check server resource utilization (CPU, memory, file descriptors)

### High Failure Rate
- Verify server is running and accessible
- Check WebSocket URL format and authentication
- Monitor server logs for errors during testing

### Inconsistent Results
- Run multiple test iterations and average results
- Ensure system is not under other load during testing
- Use dedicated test environment when possible

## Example Results Analysis

```
Connection Results:
✓ Connected: 20000/20000
✗ Failed: 0
⏱  Total time: 16.87s
⏱  Avg connection time: 262.51ms
⏱  Connections per second: 1186

Connection Time Distribution:
  Min: 45.23ms
  P50: 234.56ms     <- 50% of connections faster than this
  P95: 445.78ms     <- 95% of connections faster than this  
  P99: 567.89ms     <- 99% of connections faster than this
  Max: 678.90ms
```

**Analysis**: 
- **Good P95/P99 consistency** (< 2x P50) indicates predictable performance
- **High connection rate** (>1000/sec) shows good server performance
- **Zero failures** indicates robust connection handling

## Integration with CI/CD

```bash
# Performance regression test
if [[ $(WS_URL=$TEST_SERVER_URL NUM_CLIENTS=5000 node mass-connection-test.js | grep "Connections per second" | awk '{print $4}') -lt 800 ]]; then
  echo "Performance regression detected"
  exit 1
fi
```