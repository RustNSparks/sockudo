require("dotenv").config();
const WebSocket = require("ws");
const assert = require("assert");

// Test configuration
const config = {
  host: process.env.PUSHER_HOST || "localhost",
  port: Number(process.env.PUSHER_PORT) || 6001,
  useTLS: process.env.PUSHER_USE_TLS === "true",
};

// Helper to create WebSocket URL
function getWsUrl(appKey) {
  const protocol = config.useTLS ? "wss" : "ws";
  return `${protocol}://${config.host}:${config.port}/app/${appKey}?protocol=7&client=js&version=7.0.0`;
}

// Helper to connect with specific origin
function connectWithOrigin(appKey, origin) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(getWsUrl(appKey), {
      headers: origin ? { Origin: origin } : {},
    });

    let connectionEstablished = false;
    let errorReceived = null;

    ws.on("open", () => {
      // Connection opened, but we need to wait for pusher:connection_established or pusher:error
    });

    ws.on("message", (data) => {
      try {
        const message = JSON.parse(data);
        
        if (message.event === "pusher:connection_established") {
          connectionEstablished = true;
          ws.close();
          resolve({ success: true, message });
        } else if (message.event === "pusher:error") {
          errorReceived = message;
          ws.close();
          resolve({ success: false, error: message });
        }
      } catch (e) {
        reject(new Error(`Failed to parse message: ${e.message}`));
      }
    });

    ws.on("error", (err) => {
      reject(err);
    });

    ws.on("close", () => {
      if (!connectionEstablished && !errorReceived) {
        reject(new Error("Connection closed without establishing or error"));
      }
    });

    // Timeout after 5 seconds
    setTimeout(() => {
      ws.close();
      reject(new Error("Connection timeout"));
    }, 5000);
  });
}

// Test cases
async function runTests() {
  console.log("Starting origin validation tests...\n");

  // Test 1: App without origin restrictions should accept any origin
  console.log("Test 1: App without origin restrictions");
  try {
    const result = await connectWithOrigin("app_no_restrictions", "https://example.com");
    assert(result.success, "Should connect successfully");
    console.log("✓ Passed: Connection accepted without origin restrictions\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  // Test 2: App with origin restrictions should reject non-allowed origin
  console.log("Test 2: App with origin restrictions - reject non-allowed origin");
  try {
    const result = await connectWithOrigin("app_with_origins", "https://evil.com");
    assert(!result.success, "Should not connect successfully");
    assert(result.error, "Should receive error");
    assert.equal(result.error.data.code, 4009, "Should receive error code 4009");
    assert(result.error.data.message.includes("Origin not allowed"), "Should have correct error message");
    console.log("✓ Passed: Connection rejected for non-allowed origin\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  // Test 3: App with origin restrictions should accept allowed origin
  console.log("Test 3: App with origin restrictions - accept allowed origin");
  try {
    const result = await connectWithOrigin("app_with_origins", "https://allowed.example.com");
    assert(result.success, "Should connect successfully");
    console.log("✓ Passed: Connection accepted for allowed origin\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  // Test 4: Test wildcard subdomain matching
  console.log("Test 4: Wildcard subdomain matching");
  try {
    const result = await connectWithOrigin("app_with_wildcard", "https://test.staging.example.com");
    assert(result.success, "Should connect successfully with wildcard match");
    console.log("✓ Passed: Wildcard subdomain matching works\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  // Test 5: Test missing Origin header behavior
  console.log("Test 5: Missing Origin header");
  try {
    const result = await connectWithOrigin("app_with_origins", null);
    // Behavior depends on configuration - either reject or accept
    console.log(`Result: ${result.success ? "Accepted" : "Rejected"} connection without Origin header`);
    if (!result.success) {
      assert.equal(result.error.data.code, 4009, "Should receive error code 4009");
    }
    console.log("✓ Passed: Handled missing Origin header correctly\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  // Test 6: Test localhost origin
  console.log("Test 6: Localhost origin");
  try {
    const result = await connectWithOrigin("app_with_localhost", "http://localhost:3000");
    assert(result.success, "Should connect successfully from localhost");
    console.log("✓ Passed: Localhost origin accepted\n");
  } catch (error) {
    console.error("✗ Failed:", error.message, "\n");
  }

  console.log("Origin validation tests completed!");
}

// Note: These tests require specific app configurations to be set up in the server
// The following apps should be configured:
// - app_no_restrictions: An app with no allowed_origins configured
// - app_with_origins: An app with specific allowed_origins like ["https://allowed.example.com"]
// - app_with_wildcard: An app with wildcard origins like ["*.staging.example.com"]
// - app_with_localhost: An app allowing localhost origins like ["http://localhost:3000"]

// Run tests if this file is executed directly
if (require.main === module) {
  runTests().catch(console.error);
}

module.exports = { connectWithOrigin, runTests };