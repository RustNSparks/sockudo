/**
 * Broadcast Only Script
 * 
 * Simple script that ONLY sends broadcast messages to channels.
 * Uses the same channel naming scheme as pusher-load-test.js for consistency.
 */

const http = require('http');
const crypto = require('crypto');
const chalk = require('chalk');

// Configuration - matches pusher-load-test.js
const CONFIG = {
    appId: process.env.APP_ID || 'app-id',
    appKey: process.env.APP_KEY || 'app-key', 
    appSecret: process.env.APP_SECRET || 'app-secret',
    wsHost: process.env.WS_HOST || 'localhost',
    wsPort: parseInt(process.env.WS_PORT) || 6003,
    numChannels: parseInt(process.env.NUM_CHANNELS) || 1, // How many channels to broadcast to
    channelPrefix: process.env.CHANNEL_PREFIX || 'test-channel-',
    broadcastMessage: process.env.BROADCAST_MESSAGE || JSON.stringify({ 
        test: 'message', 
        timestamp: Date.now(),
        source: 'broadcast-only-script'
    }),
    eventName: process.env.EVENT_NAME || 'test-event',
};

function printHeader() {
    console.log('\n' + chalk.cyan('═'.repeat(50)));
    console.log(chalk.cyan.bold('     Broadcast Only Script'));
    console.log(chalk.cyan('═'.repeat(50)));
    console.log(chalk.yellow('Configuration:'));
    console.log(`  App ID: ${chalk.white.bold(CONFIG.appId)}`);
    console.log(`  App Key: ${chalk.white.bold(CONFIG.appKey)}`);
    console.log(`  Server: ${chalk.white.bold(`http://${CONFIG.wsHost}:${CONFIG.wsPort}`)}`);
    console.log(`  Channels: ${chalk.white.bold(CONFIG.numChannels)}`);
    console.log(`  Channel Prefix: ${chalk.white.bold(CONFIG.channelPrefix)}`);
    console.log(`  Event Name: ${chalk.white.bold(CONFIG.eventName)}`);
    console.log(chalk.cyan('═'.repeat(50)) + '\n');
}

async function broadcastToChannel(channelName) {
    return new Promise((resolve, reject) => {
        const postData = JSON.stringify({
            name: CONFIG.eventName,
            data: CONFIG.broadcastMessage,
            channels: [channelName]
        });
        
        // Generate authentication parameters
        const timestamp = Math.floor(Date.now() / 1000).toString();
        const bodyMd5 = crypto.createHash('md5').update(postData).digest('hex');
        const path = `/apps/${CONFIG.appId}/events`;
        
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
            port: CONFIG.wsPort,
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
                    resolve({ channel: channelName, status: 'success' });
                } else {
                    reject(new Error(`Broadcast to ${channelName} failed with status ${res.statusCode}: ${body}`));
                }
            });
        });
        
        req.on('error', (error) => {
            reject(new Error(`Request error for ${channelName}: ${error.message}`));
        });
        
        req.write(postData);
        req.end();
    });
}

async function runBroadcastTest() {
    printHeader();
    
    console.log(chalk.blue('Generating channel list...'));
    
    // Generate channel names using same logic as pusher-load-test.js
    // In the test script: channelIndex = this.id % CONFIG.numChannels
    // So channels are: test-channel-0, test-channel-1, ..., test-channel-(NUM_CHANNELS-1)
    const channels = [];
    for (let i = 0; i < CONFIG.numChannels; i++) {
        channels.push(`${CONFIG.channelPrefix}${i}`);
    }
    
    console.log(chalk.yellow(`Broadcasting to ${channels.length} channels:`));
    channels.forEach((channel, index) => {
        console.log(`  ${index + 1}. ${chalk.white.bold(channel)}`);
    });
    
    console.log(chalk.blue('\nSending broadcasts...'));
    
    const startTime = Date.now();
    
    try {
        // Send broadcasts to all channels in parallel
        const results = await Promise.allSettled(
            channels.map(channel => broadcastToChannel(channel))
        );
        
        const endTime = Date.now();
        const totalTime = endTime - startTime;
        
        // Analyze results
        const successful = results.filter(r => r.status === 'fulfilled').length;
        const failed = results.filter(r => r.status === 'rejected').length;
        
        console.log('\n' + chalk.green('═'.repeat(50)));
        console.log(chalk.green.bold('BROADCAST RESULTS'));
        console.log(chalk.green('═'.repeat(50)));
        
        console.log(chalk.yellow('\nSummary:'));
        console.log(`  ${chalk.green('✓')} Successful: ${chalk.white.bold(successful)}/${chalk.white.bold(channels.length)}`);
        console.log(`  ${chalk.red('✗')} Failed: ${chalk.white.bold(failed)}`);
        console.log(`  Total Time: ${chalk.white.bold(totalTime + 'ms')}`);
        console.log(`  Avg Time per Channel: ${chalk.white.bold((totalTime / channels.length).toFixed(2) + 'ms')}`);
        
        // Show failures if any
        if (failed > 0) {
            console.log(chalk.red('\nFailures:'));
            results.forEach((result, index) => {
                if (result.status === 'rejected') {
                    console.log(`  ${chalk.red(channels[index])}: ${result.reason.message}`);
                }
            });
        }
        
        console.log(chalk.green('═'.repeat(50)));
        
        // Copy/paste friendly metrics
        console.log(chalk.cyan('\n📋 Copy/Paste Metrics:'));
        console.log(chalk.gray('─'.repeat(30)));
        console.log(`Channels: ${CONFIG.numChannels}`);
        console.log(`Successful broadcasts: ${successful}/${channels.length}`);
        console.log(`Failed broadcasts: ${failed}`);
        console.log(`Total time: ${totalTime}ms`);
        console.log(`Avg time per channel: ${(totalTime / channels.length).toFixed(2)}ms`);
        console.log(chalk.gray('─'.repeat(30)));
        
    } catch (error) {
        console.error(chalk.red(`\nBroadcast test failed: ${error.message}`));
        process.exit(1);
    }
}

// Run the broadcast test
if (require.main === module) {
    runBroadcastTest().catch(error => {
        console.error(chalk.red('Script failed:'), error.message);
        process.exit(1);
    });
}

module.exports = { runBroadcastTest, CONFIG };