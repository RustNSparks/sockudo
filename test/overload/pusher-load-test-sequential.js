/**
 * Realistic Pusher Load Test
 * 
 * Uses pusher-js client library for authentic connection behavior
 * Measures actual connection performance without artificial batching delays
 */

const Pusher = require('pusher-js');
const { performance } = require('perf_hooks');
const cliProgress = require('cli-progress');
const chalk = require('chalk');

// Configuration
const CONFIG = {
    appId: process.env.APP_ID || 'app-id', // App ID for HTTP API path
    appKey: process.env.APP_KEY || 'app-key', // App key for authentication
    appSecret: process.env.APP_SECRET || 'app-secret', // Required for HTTP API authentication
    wsHost: process.env.WS_HOST || 'localhost',
    wsPort: parseInt(process.env.WS_PORT) || 6003, // Both WebSocket and HTTP API use same port
    numClients: parseInt(process.env.NUM_CLIENTS) || 20,
    numChannels: parseInt(process.env.NUM_CHANNELS) || 0, // 0 = no channels, >0 = distribute across channels
    channelPrefix: process.env.CHANNEL_PREFIX || 'test-channel-',
    enableLogging: process.env.ENABLE_LOGGING === 'true',
    connectionTimeout: parseInt(process.env.CONNECTION_TIMEOUT) || 10000,
    useTLS: process.env.USE_TLS === 'true',
    keepAlive: process.env.KEEP_ALIVE === 'true' || false, // Keep connections alive after test
    keepAliveTime: parseInt(process.env.KEEP_ALIVE_TIME) || 5000, // How long to keep connections alive (ms)
    broadcastTest: process.env.BROADCAST_TEST === 'true', // Test broadcast performance
    broadcastMessage: process.env.BROADCAST_MESSAGE || JSON.stringify({ test: 'message', timestamp: Date.now() }),
};

// Statistics
const stats = {
    successful: 0,
    failed: 0,
    connectionTimes: [],
    totalTime: 0,
    startTime: 0,
    firstConnectionTime: null,
    lastConnectionTime: null,
    errors: {},
    channelSubscriptions: new Map(), // Track channel subscriptions
    broadcastLatencies: [], // Track message delivery times
    broadcastStartTime: null,
    messagesReceived: 0,
    expectedMessages: 0,
};

const clients = [];
let progressBar = null;

class PusherTestClient {
    constructor(id) {
        this.id = id;
        this.pusher = null;
        this.connected = false;
        this.fullyReady = false;
        this.connectionStartTime = null;
        this.connectionTime = null;
        this.channel = null;
        this.channelName = null;
        this.messageReceivedTime = null;
    }

    async connect() {
        return new Promise((resolve, reject) => {
            const connectionTimeout = setTimeout(() => {
                if (!this.connected || !this.fullyReady) {
                    this.cleanup();
                    reject(new Error(`Connection timeout after ${CONFIG.connectionTimeout}ms`));
                }
            }, CONFIG.connectionTimeout);

            try {
                this.connectionStartTime = performance.now();
                this.fullyReady = false;
                
                // Create Pusher client with realistic options
                this.pusher = new Pusher(CONFIG.appKey, {
                    wsHost: CONFIG.wsHost,
                    wsPort: CONFIG.wsPort,
                    wssPort: CONFIG.wsPort,
                    forceTLS: CONFIG.useTLS,
                    enabledTransports: ['ws', 'wss'],
                    disableStats: true,
                    enableLogging: CONFIG.enableLogging,
                    activityTimeout: 120000,
                    pongTimeout: 30000,
                    cluster: 'mt1', // Disable default cluster behavior
                    encrypted: CONFIG.useTLS,
                });

                // Listen for connection state changes
                this.pusher.connection.bind('connected', () => {
                    this.connectionTime = performance.now() - this.connectionStartTime;
                    this.connected = true;
                    
                    // Record statistics
                    stats.connectionTimes.push(this.connectionTime);
                    stats.successful++;
                    
                    if (!stats.firstConnectionTime) {
                        stats.firstConnectionTime = performance.now();
                    }
                    stats.lastConnectionTime = performance.now();
                    
                    // Subscribe to channel if configured, then mark as ready
                    if (CONFIG.numChannels > 0) {
                        // Distribute clients across channels
                        const channelIndex = this.id % CONFIG.numChannels;
                        this.channelName = `${CONFIG.channelPrefix}${channelIndex}`;
                        this.subscribeToChannelAndResolve(resolve, connectionTimeout);
                    } else {
                        // No channels, so we're ready immediately
                        this.fullyReady = true;
                        clearTimeout(connectionTimeout);
                        
                        // Update progress
                        if (progressBar) {
                            progressBar.increment();
                        }
                        
                        resolve(this.connectionTime);
                    }
                });

                this.pusher.connection.bind('error', (error) => {
                    clearTimeout(connectionTimeout);
                    this.connected = false;
                    
                    // Track error types
                    const errorType = error?.error?.type || 'unknown';
                    stats.errors[errorType] = (stats.errors[errorType] || 0) + 1;
                    stats.failed++;
                    
                    if (progressBar) {
                        progressBar.increment();
                    }
                    
                    reject(new Error(`Pusher error: ${error?.error?.message || 'Unknown error'}`));
                });

                this.pusher.connection.bind('disconnected', () => {
                    this.connected = false;
                });

                this.pusher.connection.bind('failed', () => {
                    clearTimeout(connectionTimeout);
                    this.connected = false;
                    stats.failed++;
                    
                    if (progressBar) {
                        progressBar.increment();
                    }
                    
                    reject(new Error('Pusher connection failed'));
                });

            } catch (error) {
                clearTimeout(connectionTimeout);
                stats.failed++;
                
                if (progressBar) {
                    progressBar.increment();
                }
                
                reject(new Error(`Failed to create Pusher client: ${error.message}`));
            }
        });
    }

    subscribeToChannelAndResolve(resolve, connectionTimeout) {
        if (!this.channelName || !this.pusher) {
            this.fullyReady = true;
            clearTimeout(connectionTimeout);
            if (progressBar) progressBar.increment();
            resolve(this.connectionTime);
            return;
        }
        
        this.channel = this.pusher.subscribe(this.channelName);
        
        // Wait for successful subscription
        this.channel.bind('pusher:subscription_succeeded', () => {
            // Track channel subscription
            if (!stats.channelSubscriptions.has(this.channelName)) {
                stats.channelSubscriptions.set(this.channelName, new Set());
            }
            stats.channelSubscriptions.get(this.channelName).add(this.id);
            
            // Listen for broadcast test messages
            if (CONFIG.broadcastTest) {
                this.channel.bind('test-event', (data) => {
                    if (stats.broadcastStartTime) {
                        const latency = performance.now() - stats.broadcastStartTime;
                        stats.broadcastLatencies.push(latency);
                        stats.messagesReceived++;
                        
                        if (CONFIG.enableLogging) {
                            console.log(`Client ${this.id} received message in ${latency.toFixed(2)}ms`);
                        }
                    }
                });
            }
            
            // Now we're fully ready
            this.fullyReady = true;
            clearTimeout(connectionTimeout);
            
            // Update progress
            if (progressBar) {
                progressBar.increment();
            }
            
            resolve(this.connectionTime);
        });
        
        // Handle subscription errors
        this.channel.bind('pusher:subscription_error', (error) => {
            clearTimeout(connectionTimeout);
            this.fullyReady = false;
            console.error(`Channel subscription failed for ${this.channelName}: ${error?.error || 'Unknown error'}`);
            // Still resolve, but mark as partially failed
            stats.failed++;
            resolve(this.connectionTime);
        });
    }
    
    subscribeToChannel() {
        // Keep this method for compatibility, but it's unused in sequential mode
        if (!this.channelName || !this.pusher) return;
        
        this.channel = this.pusher.subscribe(this.channelName);
        
        // Track channel subscription
        if (!stats.channelSubscriptions.has(this.channelName)) {
            stats.channelSubscriptions.set(this.channelName, new Set());
        }
        stats.channelSubscriptions.get(this.channelName).add(this.id);
        
        // Listen for broadcast test messages
        if (CONFIG.broadcastTest) {
            this.channel.bind('test-event', (data) => {
                if (stats.broadcastStartTime) {
                    const latency = performance.now() - stats.broadcastStartTime;
                    stats.broadcastLatencies.push(latency);
                    stats.messagesReceived++;
                    
                    if (CONFIG.enableLogging) {
                        console.log(`Client ${this.id} received message in ${latency.toFixed(2)}ms`);
                    }
                }
            });
        }
    }
    
    cleanup() {
        if (this.channel && this.pusher) {
            this.pusher.unsubscribe(this.channelName);
            this.channel = null;
        }
        if (this.pusher) {
            this.pusher.disconnect();
            this.pusher = null;
        }
        this.connected = false;
    }
}

function calculatePercentiles(times) {
    if (times.length === 0) return {};
    
    const sorted = [...times].sort((a, b) => a - b);
    return {
        min: sorted[0],
        p50: sorted[Math.floor(sorted.length * 0.50)],
        p95: sorted[Math.floor(sorted.length * 0.95)],
        p99: sorted[Math.floor(sorted.length * 0.99)],
        max: sorted[sorted.length - 1],
        avg: sorted.reduce((a, b) => a + b, 0) / sorted.length,
    };
}

function printHeader() {
    console.log('\n' + chalk.cyan('═'.repeat(60)));
    console.log(chalk.cyan.bold('  Pusher.js Sequential Load Test'));
    console.log(chalk.cyan('═'.repeat(60)));
    console.log(chalk.yellow('Configuration:'));
    console.log(`  App ID: ${chalk.white.bold(CONFIG.appId)}`);
    console.log(`  App Key: ${chalk.white.bold(CONFIG.appKey)}`);
    console.log(`  Server: ${chalk.white.bold(`${CONFIG.useTLS ? 'wss' : 'ws'}://${CONFIG.wsHost}:${CONFIG.wsPort}`)}`);
    console.log(`  Clients: ${chalk.white.bold(CONFIG.numClients)} (sequential)`);
    console.log(`  Channels: ${chalk.white.bold(CONFIG.numChannels || 'No channels')}`);
    console.log(`  Broadcast Test: ${chalk.white.bold(CONFIG.broadcastTest ? 'Enabled' : 'Disabled')}`);
    console.log(`  Connection Delay: ${chalk.white.bold((parseInt(process.env.CONNECTION_DELAY_MS) || 0) + 'ms')}`);
    console.log(`  Timeout: ${chalk.white.bold(CONFIG.connectionTimeout + 'ms')}`);
    console.log(`  Logging: ${chalk.white.bold(CONFIG.enableLogging)}`);
    console.log(chalk.cyan('═'.repeat(60)) + '\n');
}

function printResults() {
    const totalDuration = performance.now() - stats.startTime;
    const percentiles = calculatePercentiles(stats.connectionTimes);
    const successRate = ((stats.successful / CONFIG.numClients) * 100).toFixed(1);
    const connectionsPerSec = (stats.successful / (totalDuration / 1000)).toFixed(0);
    
    console.log('\n' + chalk.green('═'.repeat(60)));
    console.log(chalk.green.bold('CONNECTION TEST RESULTS'));
    console.log(chalk.green('═'.repeat(60)));
    
    // Summary with colors
    console.log(chalk.yellow('\nSummary:'));
    console.log(`  ${chalk.green('✓')} Successful: ${chalk.white.bold(stats.successful)}/${chalk.white.bold(CONFIG.numClients)}`);
    console.log(`  ${chalk.red('✗')} Failed: ${chalk.white.bold(stats.failed)}`);
    console.log(`  Success Rate: ${successRate === '100.0' ? chalk.green.bold(successRate + '%') : chalk.yellow.bold(successRate + '%')}`);
    
    // Performance Metrics
    if (stats.successful > 0) {
        console.log(chalk.yellow('\nPerformance:'));
        console.log(`  Total Time: ${chalk.white.bold((totalDuration / 1000).toFixed(2) + 's')}`);
        console.log(`  Avg Connection Time: ${chalk.white.bold(percentiles.avg.toFixed(2) + 'ms')}`);
        console.log(`  Connections/sec: ${chalk.white.bold(connectionsPerSec)}`);
        
        console.log(chalk.yellow('\nConnection Time Distribution:'));
        console.log(`  Min: ${chalk.white.bold(percentiles.min.toFixed(2) + 'ms')}`);
        console.log(`  P50: ${chalk.white.bold(percentiles.p50.toFixed(2) + 'ms')}`);
        console.log(`  P95: ${chalk.white.bold(percentiles.p95.toFixed(2) + 'ms')}`);
        console.log(`  P99: ${chalk.white.bold(percentiles.p99.toFixed(2) + 'ms')}`);
        console.log(`  Max: ${chalk.white.bold(percentiles.max.toFixed(2) + 'ms')}`);
        
        // Connection timing
        if (stats.firstConnectionTime && stats.lastConnectionTime) {
            const connectionSpan = (stats.lastConnectionTime - stats.firstConnectionTime) / 1000;
            console.log(chalk.yellow('\nConnection Timing:'));
            console.log(`  First connection: ${chalk.white.bold(((stats.firstConnectionTime - stats.startTime) / 1000).toFixed(2) + 's')}`);
            console.log(`  Last connection: ${chalk.white.bold(((stats.lastConnectionTime - stats.startTime) / 1000).toFixed(2) + 's')}`);
            console.log(`  Connection span: ${chalk.white.bold(connectionSpan.toFixed(2) + 's')}`);
        }
    }
    
    // Error breakdown
    if (Object.keys(stats.errors).length > 0) {
        console.log(chalk.red('\nError Breakdown:'));
        Object.entries(stats.errors).forEach(([errorType, count]) => {
            console.log(`  ${chalk.red(errorType)}: ${chalk.white.bold(count)}`);
        });
    }
    
    console.log(chalk.green('═'.repeat(60)));
    
    // Copy/paste friendly metrics section
    console.log(chalk.cyan('\n📋 Copy/Paste Metrics:'));
    console.log(chalk.gray('─'.repeat(40)));
    if (stats.successful > 0) {
        console.log(`Clients: ${CONFIG.numClients}`);
        console.log(`Connected: ${stats.successful}/${CONFIG.numClients}`);
        console.log(`Failed: ${stats.failed}`);
        console.log(`Total time: ${(totalDuration / 1000).toFixed(2)}s`);
        console.log(`Avg connection time: ${percentiles.avg.toFixed(2)}ms`);
        console.log(`Connections per second: ${connectionsPerSec}`);
        console.log(`P50: ${percentiles.p50.toFixed(2)}ms`);
        console.log(`P95: ${percentiles.p95.toFixed(2)}ms`);
        console.log(`P99: ${percentiles.p99.toFixed(2)}ms`);
    } else {
        console.log(`Clients: ${CONFIG.numClients}`);
        console.log(`Connected: 0/${CONFIG.numClients}`);
        console.log(`Failed: ${stats.failed}`);
        console.log('Status: All connections failed');
    }
    console.log(chalk.gray('─'.repeat(40)));
    
    // Broadcast test results
    if (CONFIG.broadcastTest && stats.broadcastLatencies.length > 0) {
        const broadcastPercentiles = calculatePercentiles(stats.broadcastLatencies);
        console.log(chalk.cyan('\n📡 Broadcast Performance:'));
        console.log(chalk.gray('─'.repeat(40)));
        console.log(`Messages received: ${stats.messagesReceived}/${stats.expectedMessages}`);
        console.log(`Avg latency: ${broadcastPercentiles.avg.toFixed(2)}ms`);
        console.log(`P50 latency: ${broadcastPercentiles.p50.toFixed(2)}ms`);
        console.log(`P95 latency: ${broadcastPercentiles.p95.toFixed(2)}ms`);
        console.log(`P99 latency: ${broadcastPercentiles.p99.toFixed(2)}ms`);
        console.log(`Max latency: ${broadcastPercentiles.max.toFixed(2)}ms`);
        console.log(chalk.gray('─'.repeat(40)));
    }
}

async function runBroadcastTest() {
    console.log(chalk.cyan('\n📡 Starting Broadcast Test...'));
    console.log(chalk.gray('─'.repeat(40)));
    
    // Calculate expected messages
    const subscribedChannels = stats.channelSubscriptions.size;
    let totalSubscribers = 0;
    for (const subscribers of stats.channelSubscriptions.values()) {
        totalSubscribers += subscribers.size;
    }
    
    stats.expectedMessages = totalSubscribers;
    console.log(`Channels: ${subscribedChannels}`);
    console.log(`Total subscribers: ${totalSubscribers}`);
    console.log(`Expected messages: ${stats.expectedMessages}`);
    
    // Wait a moment for all subscriptions to be ready
    await new Promise(resolve => setTimeout(resolve, 1000));
    
    // Record broadcast start time
    stats.broadcastStartTime = performance.now();
    
    // Trigger broadcasts to all channels via HTTP API
    const http = require('http');
    const crypto = require('crypto');
    const broadcastPromises = [];
    
    for (const channelName of stats.channelSubscriptions.keys()) {
        const promise = new Promise((resolve, reject) => {
            const postData = JSON.stringify({
                name: 'test-event',
                data: CONFIG.broadcastMessage,
                channels: [channelName]
            });
            
            // Generate authentication parameters
            const timestamp = Math.floor(Date.now() / 1000).toString();
            const bodyMd5 = crypto.createHash('md5').update(postData).digest('hex');
            const path = `/apps/${CONFIG.appId}/events`; // Use app ID in path, not app key
            
            // Create auth params (excluding auth_signature)
            const authParams = new URLSearchParams({
                auth_key: CONFIG.appKey,
                auth_timestamp: timestamp,
                auth_version: '1.0',
                body_md5: bodyMd5
            });
            
            // Create string to sign: METHOD\nPATH\nQUERY_STRING
            const queryString = authParams.toString();
            const stringToSign = `POST\n${path}\n${queryString}`;
            
            // Generate HMAC-SHA256 signature
            const signature = crypto
                .createHmac('sha256', CONFIG.appSecret)
                .update(stringToSign)
                .digest('hex');
            
            // Add signature to params
            authParams.append('auth_signature', signature);
            
            const options = {
                hostname: CONFIG.wsHost,
                port: CONFIG.wsPort, // HTTP API is on same port as WebSocket in Sockudo
                path: `${path}?${authParams.toString()}`,
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Content-Length': Buffer.byteLength(postData)
                }
            };
            
            const req = http.request(options, (res) => {
                let body = '';
                res.on('data', chunk => body += chunk);
                res.on('end', () => {
                    if (res.statusCode === 200) {
                        resolve();
                    } else {
                        reject(new Error(`Broadcast failed with status ${res.statusCode}: ${body}`));
                    }
                });
            });
            
            req.on('error', reject);
            req.write(postData);
            req.end();
        });
        
        broadcastPromises.push(promise);
    }
    
    // Send all broadcasts
    try {
        await Promise.all(broadcastPromises);
        console.log(chalk.green('✓ All broadcasts sent'));
    } catch (error) {
        console.log(chalk.red(`✗ Broadcast error: ${error.message}`));
    }
    
    // Wait for messages to be received
    console.log('Waiting for message delivery...');
    await new Promise(resolve => setTimeout(resolve, 2000));
    
    const deliveryRate = ((stats.messagesReceived / stats.expectedMessages) * 100).toFixed(1);
    console.log(chalk.gray('─'.repeat(40)));
    console.log(`Delivery rate: ${deliveryRate}%`);
}

async function runTest() {
    printHeader();
    
    console.log(chalk.blue('Creating Pusher clients...'));
    
    // Create all clients
    for (let i = 0; i < CONFIG.numClients; i++) {
        clients.push(new PusherTestClient(i));
    }
    
    // Setup progress bar
    progressBar = new cliProgress.SingleBar({
        format: chalk.cyan('Connecting') + ' |{bar}| {percentage}% | {value}/{total} | ETA: {eta}s',
        barCompleteChar: '\u2588',
        barIncompleteChar: '\u2591',
        hideCursor: true
    });
    
    progressBar.start(CONFIG.numClients, 0);
    
    console.log(chalk.blue(`\nConnecting ${CONFIG.numClients} clients sequentially...\n`));
    
    // Start timing
    stats.startTime = performance.now();
    
    // Connect clients one by one, waiting for each to fully connect and subscribe
    for (let i = 0; i < clients.length; i++) {
        const client = clients[i];
        try {
            if (CONFIG.enableLogging) {
                console.log(`Connecting client ${i + 1}/${clients.length}...`);
            }
            await client.connect();
            if (CONFIG.enableLogging) {
                console.log(`Client ${i + 1} fully connected and subscribed (${client.connectionTime.toFixed(2)}ms)`);
            }
        } catch (error) {
            console.error(`Client ${i + 1} failed to connect: ${error.message}`);
            // Error already tracked in client stats
        }
        
        // Small delay between connections if requested
        const delayMs = parseInt(process.env.CONNECTION_DELAY_MS) || 0;
        if (delayMs > 0) {
            await new Promise(resolve => setTimeout(resolve, delayMs));
        }
    }
    
    // Stop progress bar
    progressBar.stop();
    
    // Run broadcast test if enabled
    if (CONFIG.broadcastTest && CONFIG.numChannels > 0 && stats.successful > 0) {
        await runBroadcastTest();
    }
    
    // Print results
    printResults();

    // Keep connections alive for manual testing or exit
    // if (stats.successful > 0 && CONFIG.keepAlive) {
        console.log(chalk.green(`\n${stats.successful} clients connected and active.`));
        console.log(chalk.yellow('Press Ctrl+C to disconnect all clients and exit.'));
        console.log(chalk.yellow('Use this time to test server behavior with active connections.\n'));

        // Keep process alive indefinitely
        await new Promise(() => {}); // Never resolves
    // } else if (stats.successful > 0) {
    //     Keep connections alive briefly then exit
        // console.log(chalk.blue(`\nKeeping ${stats.successful} clients connected for ${CONFIG.keepAliveTime}ms...`));
        // await new Promise(resolve => setTimeout(resolve, CONFIG.keepAliveTime));
        // console.log(chalk.blue('Test complete. Disconnecting clients...'));
        // await cleanup();
    // }
}

async function cleanup() {
    console.log('\nDisconnecting all clients...');
    
    if (progressBar) {
        progressBar.stop();
    }
    
    // Cleanup all clients
    const cleanupPromises = clients.map(client => {
        return new Promise(resolve => {
            client.cleanup();
            setTimeout(resolve, 10); // Small delay for clean disconnect
        });
    });
    
    await Promise.all(cleanupPromises);
    console.log('Cleanup complete.');
}
//
// // Handle graceful shutdown
// process.on('SIGINT', async () => {
//     console.log('\nShutting down...');
//     await cleanup();
//     process.exit(0);
// });
//
// process.on('SIGTERM', async () => {
//     await cleanup();
//     process.exit(0);
// });

// Check dependencies
try {
    require('pusher-js');
    require('cli-progress');
} catch (e) {
    console.error('Missing dependencies. Please run:');
    console.error('npm install pusher-js cli-progress');
    process.exit(1);
}

// Run the test
if (require.main === module) {
    runTest().catch(error => {
        console.error('Test failed:', error.message);
        cleanup().then(() => process.exit(1));
    });
}

module.exports = { runTest, cleanup, CONFIG, stats };