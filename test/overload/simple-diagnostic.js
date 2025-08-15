/**
 * Simple Diagnostic Test
 * 
 * Minimal test to isolate timing issues and compare with browser performance
 */

const WebSocket = require('ws');
const { performance } = require('perf_hooks');

const CONFIG = {
    serverUrl: process.env.WS_URL || 'ws://localhost:6003/app/app-key',
    numClients: parseInt(process.env.NUM_CLIENTS) || 20,
};

async function testSingleConnection() {
    console.log('Testing single connection...');
    
    return new Promise((resolve, reject) => {
        const start = performance.now();
        console.log(`Connecting to: ${CONFIG.serverUrl}`);
        
        const ws = new WebSocket(CONFIG.serverUrl);
        let openTime = null;
        let establishedTime = null;
        
        ws.on('open', () => {
            openTime = performance.now() - start;
            console.log(`WebSocket 'open' event: ${openTime.toFixed(2)}ms`);
            
            // Send ping to trigger connection established
            ws.send(JSON.stringify({
                event: 'pusher:ping',
                data: {}
            }));
        });
        
        ws.on('message', (data) => {
            try {
                const message = JSON.parse(data);
                console.log(`Received message:`, message);
                
                if (message.event === 'pusher:connection_established') {
                    establishedTime = performance.now() - start;
                    console.log(`Connection established: ${establishedTime.toFixed(2)}ms`);
                    
                    ws.close();
                    resolve({
                        openTime,
                        establishedTime,
                        total: establishedTime
                    });
                }
            } catch (e) {
                console.log('Non-JSON message received:', data.toString());
            }
        });
        
        ws.on('error', (error) => {
            console.error('WebSocket error:', error.message);
            reject(error);
        });
        
        ws.on('close', (code, reason) => {
            console.log(`Connection closed: ${code} - ${reason}`);
        });
        
        // Timeout after 10 seconds
        setTimeout(() => {
            if (!establishedTime) {
                console.log('Connection timeout - no pusher:connection_established received');
                ws.terminate();
                resolve({
                    openTime,
                    establishedTime: null,
                    total: performance.now() - start,
                    timeout: true
                });
            }
        }, 10000);
    });
}

async function testMultipleConnections() {
    console.log(`\nTesting ${CONFIG.numClients} concurrent connections...`);
    
    const startTime = performance.now();
    const results = [];
    
    const promises = [];
    for (let i = 0; i < CONFIG.numClients; i++) {
        const promise = new Promise((resolve, reject) => {
            const connectionStart = performance.now();
            const ws = new WebSocket(CONFIG.serverUrl);
            
            let resolved = false;
            
            ws.on('open', () => {
                const openTime = performance.now() - connectionStart;
                
                ws.send(JSON.stringify({
                    event: 'pusher:ping',
                    data: {}
                }));
            });
            
            ws.on('message', (data) => {
                if (resolved) return;
                
                try {
                    const message = JSON.parse(data);
                    if (message.event === 'pusher:connection_established') {
                        const totalTime = performance.now() - connectionStart;
                        resolved = true;
                        
                        ws.close();
                        resolve({
                            id: i,
                            time: totalTime
                        });
                    }
                } catch (e) {
                    // Ignore malformed messages
                }
            });
            
            ws.on('error', (error) => {
                if (!resolved) {
                    resolved = true;
                    reject(new Error(`Client ${i}: ${error.message}`));
                }
            });
            
            // Timeout
            setTimeout(() => {
                if (!resolved) {
                    resolved = true;
                    ws.terminate();
                    reject(new Error(`Client ${i}: timeout`));
                }
            }, 10000);
        });
        
        promises.push(promise);
    }
    
    try {
        const results = await Promise.allSettled(promises);
        const totalTime = performance.now() - startTime;
        
        const successful = results.filter(r => r.status === 'fulfilled').map(r => r.value);
        const failed = results.filter(r => r.status === 'rejected');
        
        console.log('\nResults:');
        console.log(`Total time: ${totalTime.toFixed(2)}ms`);
        console.log(`Successful: ${successful.length}/${CONFIG.numClients}`);
        console.log(`Failed: ${failed.length}`);
        
        if (successful.length > 0) {
            const times = successful.map(r => r.time);
            const avgTime = times.reduce((a, b) => a + b, 0) / times.length;
            const minTime = Math.min(...times);
            const maxTime = Math.max(...times);
            
            console.log(`Avg connection time: ${avgTime.toFixed(2)}ms`);
            console.log(`Min: ${minTime.toFixed(2)}ms, Max: ${maxTime.toFixed(2)}ms`);
            console.log(`Rate: ${(successful.length / (totalTime / 1000)).toFixed(0)} connections/sec`);
        }
        
        if (failed.length > 0) {
            console.log('\nFailures:');
            failed.forEach((failure, idx) => {
                console.log(`  ${idx + 1}: ${failure.reason.message}`);
            });
        }
        
    } catch (error) {
        console.error('Test failed:', error);
    }
}

async function main() {
    console.log('='.repeat(50));
    console.log('Simple Diagnostic Test for Sockudo');
    console.log('='.repeat(50));
    console.log(`Server: ${CONFIG.serverUrl}`);
    console.log(`Clients: ${CONFIG.numClients}`);
    console.log('='.repeat(50));
    
    try {
        // Test single connection first
        const singleResult = await testSingleConnection();
        console.log('\nSingle connection result:', singleResult);
        
        // Test multiple connections
        await testMultipleConnections();
        
    } catch (error) {
        console.error('Diagnostic failed:', error);
    }
}

main().catch(console.error);