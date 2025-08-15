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
    appKey: process.env.APP_KEY || 'app-key',
    wsHost: process.env.WS_HOST || 'localhost',
    wsPort: parseInt(process.env.WS_PORT) || 6003,
    numClients: parseInt(process.env.NUM_CLIENTS) || 20,
    enableLogging: process.env.ENABLE_LOGGING === 'true',
    connectionTimeout: parseInt(process.env.CONNECTION_TIMEOUT) || 10000,
    useTLS: process.env.USE_TLS === 'true',
    keepAlive: process.env.KEEP_ALIVE === 'true', // Keep connections alive after test
    keepAliveTime: parseInt(process.env.KEEP_ALIVE_TIME) || 5000, // How long to keep connections alive (ms)
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
};

const clients = [];
let progressBar = null;

class PusherTestClient {
    constructor(id) {
        this.id = id;
        this.pusher = null;
        this.connected = false;
        this.connectionStartTime = null;
        this.connectionTime = null;
    }

    async connect() {
        return new Promise((resolve, reject) => {
            const connectionTimeout = setTimeout(() => {
                if (!this.connected) {
                    this.cleanup();
                    reject(new Error(`Connection timeout after ${CONFIG.connectionTimeout}ms`));
                }
            }, CONFIG.connectionTimeout);

            try {
                this.connectionStartTime = performance.now();
                
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
                    clearTimeout(connectionTimeout);
                    
                    // Record statistics
                    stats.connectionTimes.push(this.connectionTime);
                    stats.successful++;
                    
                    if (!stats.firstConnectionTime) {
                        stats.firstConnectionTime = performance.now();
                    }
                    stats.lastConnectionTime = performance.now();
                    
                    // Update progress
                    if (progressBar) {
                        progressBar.increment();
                    }
                    
                    resolve(this.connectionTime);
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

    cleanup() {
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
    console.log(chalk.cyan.bold('     Pusher.js Realistic Load Test'));
    console.log(chalk.cyan('═'.repeat(60)));
    console.log(chalk.yellow('Configuration:'));
    console.log(`  App Key: ${chalk.white.bold(CONFIG.appKey)}`);
    console.log(`  Server: ${chalk.white.bold(`${CONFIG.useTLS ? 'wss' : 'ws'}://${CONFIG.wsHost}:${CONFIG.wsPort}`)}`);
    console.log(`  Clients: ${chalk.white.bold(CONFIG.numClients)}`);
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
    
    console.log(chalk.blue(`\nConnecting ${CONFIG.numClients} clients simultaneously...\n`));
    
    // Start timing
    stats.startTime = performance.now();
    
    // Connect all clients simultaneously (no artificial batching)
    const promises = clients.map(client => 
        client.connect().catch(error => {
            // Error already tracked in client, just return null
            return null;
        })
    );
    
    // Wait for all connections to complete
    await Promise.allSettled(promises);
    
    // Stop progress bar
    progressBar.stop();
    
    // Print results
    printResults();
    
    // Keep connections alive for manual testing or exit
    if (stats.successful > 0 && CONFIG.keepAlive) {
        console.log(chalk.green(`\n${stats.successful} clients connected and active.`));
        console.log(chalk.yellow('Press Ctrl+C to disconnect all clients and exit.'));
        console.log(chalk.yellow('Use this time to test server behavior with active connections.\n'));
        
        // Keep process alive indefinitely
        await new Promise(() => {}); // Never resolves
    } else if (stats.successful > 0) {
        // Keep connections alive briefly then exit
        console.log(chalk.blue(`\nKeeping ${stats.successful} clients connected for ${CONFIG.keepAliveTime}ms...`));
        await new Promise(resolve => setTimeout(resolve, CONFIG.keepAliveTime));
        console.log(chalk.blue('Test complete. Disconnecting clients...'));
        await cleanup();
    }
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

// Handle graceful shutdown
process.on('SIGINT', async () => {
    console.log('\nShutting down...');
    await cleanup();
    process.exit(0);
});

process.on('SIGTERM', async () => {
    await cleanup();
    process.exit(0);
});

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