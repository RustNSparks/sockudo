/**
 * Enhanced Mass Connection Test for Sockudo
 *
 * This script provides accurate measurements of WebSocket connection performance
 * during mass connection and reconnection scenarios. It measures true server
 * processing time by waiting for pusher:connection_established events.
 *
 * Usage:
 *   node mass-connection-test.js
 *   
 * Environment Variables:
 *   WS_URL=ws://localhost:6001/app/app-key
 *   NUM_CLIENTS=20000
 *   BATCH_SIZE=500
 *   BATCH_DELAY=50
 *   VALIDATION_PERCENT=10  # Percentage of clients to validate with subscriptions
 *   BACKGROUND_LOAD=true   # Enable background message traffic
 */

const WebSocket = require('ws');
const { performance } = require('perf_hooks');
const EventEmitter = require('events');

// Configuration
const CONFIG = {
    serverUrl: process.env.WS_URL || 'ws://localhost:6003/app/app-key',
    numClients: parseInt(process.env.NUM_CLIENTS) || 20000,
    batchSize: parseInt(process.env.BATCH_SIZE) || 500,
    batchDelay: parseInt(process.env.BATCH_DELAY) || 50,
    validationPercent: parseInt(process.env.VALIDATION_PERCENT) || 10,
    backgroundLoad: process.env.BACKGROUND_LOAD === 'true',
    connectionTimeout: 30000,
    validationTimeout: 10000,
};

// Color codes for terminal output
const colors = {
    reset: '\x1b[0m',
    bright: '\x1b[1m',
    red: '\x1b[31m',
    green: '\x1b[32m',
    yellow: '\x1b[33m',
    blue: '\x1b[34m',
    cyan: '\x1b[36m',
    magenta: '\x1b[35m'
};

// Enhanced statistics tracking
const stats = {
    successful: 0,
    failed: 0,
    validated: 0,
    validationFailed: 0,
    totalConnectionTime: 0,
    totalValidationTime: 0,
    connectionTimes: [],
    batchTimes: [],
    startTime: 0,
    firstConnectionTime: null,
    lastConnectionTime: null,
};

const clients = [];
const backgroundLoadInterval = [];

class EnhancedTestClient extends EventEmitter {
    constructor(id) {
        super();
        this.id = id;
        this.ws = null;
        this.connected = false;
        this.fullyEstablished = false;
        this.validated = false;
        this.connectionStartTime = null;
        this.validationStartTime = null;
        this.pingInterval = null;
        this.shouldValidate = Math.random() < (CONFIG.validationPercent / 100);
    }

    async connect(url) {
        return new Promise((resolve, reject) => {
            try {
                this.ws = new WebSocket(url);
                
                let isResolved = false;
                const connectionTimeout = setTimeout(() => {
                    if (!isResolved) {
                        isResolved = true;
                        this.cleanup();
                        reject(new Error(`Connection timeout after ${CONFIG.connectionTimeout}ms`));
                    }
                }, CONFIG.connectionTimeout);

                this.ws.on('open', () => {
                    this.connected = true;
                    this.connectionStartTime = performance.now(); // Start timing when connection opens
                    
                    // Send initial ping to prompt connection established
                    this.ws.send(JSON.stringify({
                        event: 'pusher:ping',
                        data: {}
                    }));
                });

                this.ws.on('message', (data) => {
                    if (isResolved) return; // Prevent multiple resolutions
                    
                    try {
                        const message = JSON.parse(data);
                        
                        // Handle connection established - this is the true server processing completion
                        if (message.event === 'pusher:connection_established' && !this.fullyEstablished) {
                            const connectionTime = performance.now() - this.connectionStartTime;
                            this.fullyEstablished = true;
                            isResolved = true;
                            clearTimeout(connectionTimeout);
                            
                            // Record timing statistics
                            stats.totalConnectionTime += connectionTime;
                            stats.connectionTimes.push(connectionTime);
                            
                            if (!stats.firstConnectionTime) {
                                stats.firstConnectionTime = performance.now();
                            }
                            stats.lastConnectionTime = performance.now();

                            // Start keep-alive pings
                            this.startKeepAlive();

                            // Start validation if needed
                            if (this.shouldValidate) {
                                this.startValidation()
                                    .then(() => resolve(connectionTime))
                                    .catch(() => resolve(connectionTime)); // Don't fail connection for validation issues
                            } else {
                                resolve(connectionTime);
                            }
                            return;
                        }

                        // Handle other message types for validation
                        this.handleOtherMessages(message);
                        
                    } catch (e) {
                        // Ignore malformed messages
                    }
                });

                this.ws.on('error', (error) => {
                    if (!isResolved) {
                        isResolved = true;
                        clearTimeout(connectionTimeout);
                        this.connected = false;
                        reject(new Error(`WebSocket error: ${error.message}`));
                    }
                });

                this.ws.on('close', (code, reason) => {
                    if (!isResolved) {
                        isResolved = true;
                        clearTimeout(connectionTimeout);
                        reject(new Error(`Connection closed before established: ${code} - ${reason}`));
                    }
                    this.connected = false;
                    this.fullyEstablished = false;
                    this.emit('disconnected', { id: this.id, code, reason });
                });

            } catch (error) {
                reject(new Error(`Failed to create WebSocket: ${error.message}`));
            }
        });
    }

    handleOtherMessages(message) {
        // Handle subscription success for validation
        if (message.event === 'pusher_internal:subscription_succeeded' && 
            message.channel === `validation-${this.id}`) {
            const validationTime = performance.now() - this.validationStartTime;
            this.validated = true;
            stats.totalValidationTime += validationTime;
            this.emit('validated', { id: this.id, time: validationTime });
        }

        // Handle pong responses
        if (message.event === 'pusher:pong') {
            this.emit('pong', { id: this.id, timestamp: Date.now() });
        }
    }

    async startValidation() {
        return new Promise((resolve, reject) => {
            this.validationStartTime = performance.now();
            const validationChannel = `validation-${this.id}`;
            
            const validationTimeout = setTimeout(() => {
                stats.validationFailed++;
                reject(new Error('Validation timeout'));
            }, CONFIG.validationTimeout);

            this.once('validated', () => {
                clearTimeout(validationTimeout);
                stats.validated++;
                resolve();
            });

            // Subscribe to validation channel
            this.ws.send(JSON.stringify({
                event: 'pusher:subscribe',
                data: { channel: validationChannel }
            }));
        });
    }

    startKeepAlive() {
        this.pingInterval = setInterval(() => {
            if (this.connected && this.ws && this.ws.readyState === WebSocket.OPEN) {
                this.ws.send(JSON.stringify({
                    event: 'pusher:ping',
                    data: { timestamp: Date.now() }
                }));
            }
        }, 30000);
    }

    startBackgroundLoad() {
        if (!CONFIG.backgroundLoad) return;

        const interval = setInterval(() => {
            if (this.connected && this.ws && this.ws.readyState === WebSocket.OPEN) {
                // Send random client events to simulate real load
                const eventTypes = ['heartbeat', 'user_activity', 'ping_test'];
                const randomEvent = eventTypes[Math.floor(Math.random() * eventTypes.length)];
                
                this.ws.send(JSON.stringify({
                    event: `client-${randomEvent}`,
                    data: {
                        client_id: this.id,
                        timestamp: Date.now(),
                        random_data: Math.random().toString(36).substr(2, 9)
                    }
                }));
            }
        }, 5000 + Math.random() * 10000); // Random interval 5-15 seconds

        backgroundLoadInterval.push(interval);
    }

    cleanup() {
        if (this.pingInterval) {
            clearInterval(this.pingInterval);
            this.pingInterval = null;
        }
        if (this.ws) {
            this.ws.terminate();
            this.ws = null;
        }
        this.connected = false;
        this.fullyEstablished = false;
    }
}

async function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

function printHeader() {
    console.log('\n' + colors.bright + colors.cyan + '='.repeat(70) + colors.reset);
    console.log(colors.bright + colors.cyan + '     Enhanced Sockudo Mass Connection Test' + colors.reset);
    console.log(colors.bright + colors.cyan + '='.repeat(70) + colors.reset);
    console.log(`${colors.yellow}Configuration:${colors.reset}`);
    console.log(`  Server URL: ${colors.bright}${CONFIG.serverUrl}${colors.reset}`);
    console.log(`  Target clients: ${colors.bright}${CONFIG.numClients}${colors.reset}`);
    console.log(`  Batch size: ${colors.bright}${CONFIG.batchSize}${colors.reset}`);
    console.log(`  Batch delay: ${colors.bright}${CONFIG.batchDelay}ms${colors.reset}`);
    console.log(`  Validation: ${colors.bright}${CONFIG.validationPercent}%${colors.reset} of clients`);
    console.log(`  Background load: ${colors.bright}${CONFIG.backgroundLoad}${colors.reset}`);
    console.log(`  Connection timeout: ${colors.bright}${CONFIG.connectionTimeout}ms${colors.reset}`);
    console.log(colors.cyan + '='.repeat(70) + colors.reset + '\n');
}

function calculatePercentiles(times) {
    if (times.length === 0) return {};
    
    const sorted = [...times].sort((a, b) => a - b);
    return {
        p50: sorted[Math.floor(sorted.length * 0.5)],
        p95: sorted[Math.floor(sorted.length * 0.95)],
        p99: sorted[Math.floor(sorted.length * 0.99)],
        min: sorted[0],
        max: sorted[sorted.length - 1]
    };
}

function printProgressUpdate() {
    const elapsed = (performance.now() - stats.startTime) / 1000;
    const completed = stats.successful + stats.failed;
    const rate = completed / elapsed;
    
    process.stdout.write(
        `\r${colors.blue}Progress: ${completed}/${CONFIG.numClients} ` +
        `(${(completed/CONFIG.numClients*100).toFixed(1)}%) ` +
        `Rate: ${rate.toFixed(0)}/s ` +
        `Success: ${colors.green}${stats.successful}${colors.reset}${colors.blue} ` +
        `Failed: ${colors.red}${stats.failed}${colors.reset}${colors.blue} ` +
        `Elapsed: ${elapsed.toFixed(1)}s${colors.reset}`
    );
}

async function connectClients() {
    console.log(`${colors.blue}Connecting ${CONFIG.numClients} clients in batches of ${CONFIG.batchSize}...${colors.reset}`);
    console.log(`${colors.yellow}Measuring accurate server processing time via pusher:connection_established${colors.reset}`);
    console.log(`${colors.yellow}(Stop with Ctrl+C and restart to test reconnection behavior)${colors.reset}\n`);

    stats.startTime = performance.now();

    // Create all clients
    for (let i = 0; i < CONFIG.numClients; i++) {
        const client = new EnhancedTestClient(i);
        
        // Add event listeners for better monitoring
        client.on('validated', () => {
            if (stats.validated % 100 === 0) {
                process.stdout.write(`${colors.magenta}[V${stats.validated}]${colors.reset}`);
            }
        });
        
        client.on('disconnected', (data) => {
            // Could track unexpected disconnections here
        });
        
        clients.push(client);
    }

    // Connect in batches with detailed timing
    let batchNumber = 0;
    for (let i = 0; i < clients.length; i += CONFIG.batchSize) {
        const batch = clients.slice(i, Math.min(i + CONFIG.batchSize, clients.length));
        const batchStartTime = performance.now();
        batchNumber++;

        console.log(`\n${colors.cyan}Batch ${batchNumber}: Connecting ${batch.length} clients...${colors.reset}`);

        const promises = batch.map(client =>
            client.connect(CONFIG.serverUrl)
                .then(connectionTime => {
                    stats.successful++;
                    
                    // Progress reporting based on test size
                    const progressInterval = CONFIG.numClients >= 1000 ? 100 : 
                                           CONFIG.numClients >= 100 ? 10 : 1;
                    
                    if (stats.successful % progressInterval === 0) {
                        printProgressUpdate();
                    }
                    
                    // Start background load for this client
                    if (CONFIG.backgroundLoad) {
                        client.startBackgroundLoad();
                    }
                    
                    return connectionTime;
                })
                .catch(err => {
                    stats.failed++;
                    
                    // Progress reporting based on test size
                    const progressInterval = CONFIG.numClients >= 1000 ? 100 : 
                                           CONFIG.numClients >= 100 ? 10 : 1;
                    
                    if ((stats.successful + stats.failed) % progressInterval === 0) {
                        printProgressUpdate();
                    }
                    
                    if (CONFIG.numClients <= 100) {
                        console.log(`\n${colors.red}Client ${client.id} failed: ${err.message}${colors.reset}`);
                    }
                    return null;
                })
        );

        const results = await Promise.allSettled(promises);
        const batchDuration = performance.now() - batchStartTime;
        stats.batchTimes.push(batchDuration);

        const batchSuccess = results.filter(r => r.status === 'fulfilled' && r.value !== null).length;
        const batchFailed = results.filter(r => r.status === 'rejected' || r.value === null).length;
        
        console.log(`\n${colors.green}Batch ${batchNumber} completed: ${batchSuccess} success, ${batchFailed} failed in ${(batchDuration/1000).toFixed(2)}s${colors.reset}`);

        // Small delay between batches (only if there are more batches and batch is large enough)
        if (i + CONFIG.batchSize < clients.length && CONFIG.batchSize >= 100) {
            await sleep(CONFIG.batchDelay);
        }
    }

    const totalDuration = performance.now() - stats.startTime;
    printDetailedResults(totalDuration);
}

function printDetailedResults(totalDuration) {
    console.log('\n\n' + colors.cyan + '='.repeat(70) + colors.reset);
    console.log(`${colors.bright}${colors.green}Connection Test Results${colors.reset}`);
    console.log(colors.cyan + '='.repeat(70) + colors.reset);
    
    // Connection Results
    console.log(`\n${colors.bright}Connection Summary:${colors.reset}`);
    console.log(`${colors.green}✓${colors.reset} Connected: ${colors.bright}${stats.successful}${colors.reset}/${CONFIG.numClients}`);
    console.log(`${colors.red}✗${colors.reset} Failed: ${colors.bright}${stats.failed}${colors.reset}`);
    console.log(`${colors.magenta}◉${colors.reset} Validated: ${colors.bright}${stats.validated}${colors.reset}/${Math.floor(CONFIG.numClients * CONFIG.validationPercent / 100)}`);
    
    // Timing Results
    console.log(`\n${colors.bright}Performance Metrics:${colors.reset}`);
    console.log(`⏱  Total time: ${colors.bright}${(totalDuration / 1000).toFixed(2)}s${colors.reset}`);
    
    if (stats.successful > 0) {
        console.log(`⏱  Avg connection time: ${colors.bright}${(stats.totalConnectionTime / stats.successful).toFixed(2)}ms${colors.reset}`);
        console.log(`⏱  Connections per second: ${colors.bright}${(stats.successful / (totalDuration / 1000)).toFixed(0)}${colors.reset}`);
        
        // Connection time distribution
        const percentiles = calculatePercentiles(stats.connectionTimes);
        console.log(`\n${colors.bright}Connection Time Distribution:${colors.reset}`);
        console.log(`  Min: ${colors.bright}${percentiles.min?.toFixed(2)}ms${colors.reset}`);
        console.log(`  P50: ${colors.bright}${percentiles.p50?.toFixed(2)}ms${colors.reset}`);
        console.log(`  P95: ${colors.bright}${percentiles.p95?.toFixed(2)}ms${colors.reset}`);
        console.log(`  P99: ${colors.bright}${percentiles.p99?.toFixed(2)}ms${colors.reset}`);
        console.log(`  Max: ${colors.bright}${percentiles.max?.toFixed(2)}ms${colors.reset}`);
        
        // Time to first/last connection
        if (stats.firstConnectionTime && stats.lastConnectionTime) {
            const totalConnectSpan = (stats.lastConnectionTime - stats.firstConnectionTime) / 1000;
            console.log(`\n${colors.bright}Connection Span:${colors.reset}`);
            console.log(`  First connection at: ${colors.bright}${((stats.firstConnectionTime - stats.startTime)/1000).toFixed(2)}s${colors.reset}`);
            console.log(`  Last connection at: ${colors.bright}${((stats.lastConnectionTime - stats.startTime)/1000).toFixed(2)}s${colors.reset}`);
            console.log(`  Connection span: ${colors.bright}${totalConnectSpan.toFixed(2)}s${colors.reset}`);
        }
    }
    
    // Validation Results
    if (CONFIG.validationPercent > 0) {
        console.log(`\n${colors.bright}Validation Results:${colors.reset}`);
        if (stats.validated > 0) {
            console.log(`  Avg validation time: ${colors.bright}${(stats.totalValidationTime / stats.validated).toFixed(2)}ms${colors.reset}`);
        }
        console.log(`  Validation success rate: ${colors.bright}${((stats.validated / (stats.validated + stats.validationFailed)) * 100).toFixed(1)}%${colors.reset}`);
    }
    
    // Batch Performance
    if (stats.batchTimes.length > 0) {
        const batchPercentiles = calculatePercentiles(stats.batchTimes);
        console.log(`\n${colors.bright}Batch Performance:${colors.reset}`);
        console.log(`  Avg batch time: ${colors.bright}${(stats.batchTimes.reduce((a,b) => a+b, 0) / stats.batchTimes.length / 1000).toFixed(2)}s${colors.reset}`);
        console.log(`  Fastest batch: ${colors.bright}${(batchPercentiles.min / 1000).toFixed(2)}s${colors.reset}`);
        console.log(`  Slowest batch: ${colors.bright}${(batchPercentiles.max / 1000).toFixed(2)}s${colors.reset}`);
    }
    
    console.log(colors.cyan + '='.repeat(70) + colors.reset);
    console.log(`\n${colors.bright}${colors.yellow}All clients connected and maintaining connection.${colors.reset}`);
    console.log(`${colors.yellow}Press Ctrl+C to disconnect all clients.${colors.reset}`);
    console.log(`${colors.yellow}Then run the script again to test reconnection behavior.${colors.reset}\n`);
}

async function cleanup() {
    console.log(`\n${colors.cyan}Cleaning up ${clients.length} clients...${colors.reset}`);
    
    // Clear background load intervals
    backgroundLoadInterval.forEach(interval => clearInterval(interval));
    backgroundLoadInterval.length = 0;
    
    // Cleanup all clients
    const cleanupPromises = clients.map(client => {
        return new Promise(resolve => {
            client.cleanup();
            // Give each client a moment to cleanup
            setTimeout(resolve, 10);
        });
    });
    
    await Promise.all(cleanupPromises);
    console.log(`${colors.green}Cleanup complete.${colors.reset}`);
}

// Handle graceful shutdown
process.on('SIGINT', async () => {
    console.log(`\n${colors.yellow}Shutting down gracefully...${colors.reset}`);
    await cleanup();
    process.exit(0);
});

process.on('SIGTERM', async () => {
    await cleanup();
    process.exit(0);
});

// Check system limits and dependencies
function checkSystemRequirements() {
    try {
        require('ws');
    } catch (e) {
        console.error(`${colors.red}Error: 'ws' module not found${colors.reset}`);
        console.log(`Please run: ${colors.bright}npm install ws${colors.reset}`);
        process.exit(1);
    }
    
    // Check if we might hit file descriptor limits
    if (CONFIG.numClients > 1000) {
        console.log(`${colors.yellow}Warning: Testing ${CONFIG.numClients} connections.${colors.reset}`);
        console.log(`${colors.yellow}Ensure ulimit -n is set appropriately (recommended: 65536).${colors.reset}`);
        console.log(`${colors.yellow}Current process limit can be checked with: ulimit -n${colors.reset}\n`);
    }
}

async function main() {
    checkSystemRequirements();
    printHeader();

    try {
        await connectClients();
        
        // Keep the process alive and monitor connections
        await new Promise(() => {}); // Never resolves - keeps process alive
        
    } catch (error) {
        console.error(`${colors.red}Test failed:${colors.reset}`, error.message);
        await cleanup();
        process.exit(1);
    }
}

// Export for potential module usage
module.exports = {
    EnhancedTestClient,
    CONFIG,
    stats,
    connectClients,
    cleanup
};

// Run the test if this file is executed directly
if (require.main === module) {
    main().catch(console.error);
}