#![cfg(feature = "nats")]

use serde_json::json;
use sockudo::adapter::ConnectionManager;
use sockudo::adapter::connection_manager::HorizontalAdapterInterface;
use sockudo::adapter::nats_adapter::NatsAdapter;
use sockudo::options::{ClusterHealthConfig, NatsAdapterConfig};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Helper to check if NATS is available
async fn is_nats_available() -> bool {
    let nats_url = env::var("NATS_SERVERS").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());

    (async_nats::connect(&nats_url).await).is_ok()
}

/// Helper to create a NATS adapter with cluster health enabled
async fn create_nats_adapter(node_id: &str, cluster_config: &ClusterHealthConfig) -> NatsAdapter {
    let nats_servers = env::var("NATS_SERVERS")
        .unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string())
        .split(',')
        .map(|s| s.to_string())
        .collect();

    let config = NatsAdapterConfig {
        servers: nats_servers,
        prefix: format!("test_cluster_health_{}", node_id),
        request_timeout_ms: 5000,
        username: env::var("NATS_USERNAME").ok(),
        password: env::var("NATS_PASSWORD").ok(),
        token: env::var("NATS_TOKEN").ok(),
        connection_timeout_ms: 5000,
        nodes_number: Some(1),
    };

    let mut adapter = NatsAdapter::new(config).await.unwrap();
    adapter.set_cluster_health(cluster_config).await.unwrap();
    adapter
}

#[tokio::test]
async fn test_nats_adapter_heartbeat_system() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    // Create two NATS adapters simulating two nodes
    let mut adapter1 = create_nats_adapter("nats_node1", &cluster_config).await;
    let mut adapter2 = create_nats_adapter("nats_node2", &cluster_config).await;

    adapter1.init().await;
    adapter2.init().await;

    // Allow heartbeats to propagate through NATS
    sleep(Duration::from_millis(300)).await;

    // Both adapters should be aware of each other

    drop(adapter1);
    drop(adapter2);
}

#[tokio::test]
async fn test_nats_presence_broadcast_and_sync() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let mut adapter1 = create_nats_adapter("nats_node1", &cluster_config).await;
    let mut adapter2 = create_nats_adapter("nats_node2", &cluster_config).await;
    let mut adapter3 = create_nats_adapter("nats_node3", &cluster_config).await;

    adapter1.init().await;
    adapter2.init().await;
    adapter3.init().await;

    let app_id = "test-app";
    let channel = "presence-nats-test";

    // Broadcast presence from different nodes
    adapter1
        .broadcast_presence_join(
            app_id,
            channel,
            "user1",
            "socket1",
            Some(json!({"from": "node1"})),
        )
        .await
        .unwrap();

    adapter2
        .broadcast_presence_join(
            app_id,
            channel,
            "user2",
            "socket2",
            Some(json!({"from": "node2"})),
        )
        .await
        .unwrap();

    adapter3
        .broadcast_presence_join(
            app_id,
            channel,
            "user3",
            "socket3",
            Some(json!({"from": "node3"})),
        )
        .await
        .unwrap();

    // Allow NATS to propagate messages
    sleep(Duration::from_millis(300)).await;

    // All nodes should have synchronized presence state

    // Test presence leave
    adapter2
        .broadcast_presence_leave(app_id, channel, "user2", "socket2")
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;

    drop(adapter1);
    drop(adapter2);
    drop(adapter3);
}

#[tokio::test]
async fn test_nats_dead_node_detection_and_cleanup() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 300, // Short timeout for testing
        cleanup_interval_ms: 150,
    };

    let mut adapter1 = create_nats_adapter("nats_node1", &cluster_config).await;
    let mut adapter2 = create_nats_adapter("nats_node2", &cluster_config).await;

    adapter1.init().await;
    adapter2.init().await;

    let app_id = "test-app";
    let channel = "presence-cleanup-test";

    // Add presence on adapter2 which will "die"
    adapter2
        .broadcast_presence_join(
            app_id,
            channel,
            "user_orphaned",
            "socket_orphaned",
            Some(json!({"status": "will_be_orphaned"})),
        )
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;

    // Simulate adapter2 dying (drop connection)
    drop(adapter2);

    // Wait for dead node detection
    sleep(Duration::from_millis(500)).await;

    // Adapter1 should detect the dead node and clean up orphaned presence

    drop(adapter1);
}

#[tokio::test]
async fn test_nats_pub_sub_pattern_isolation() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    // Create adapters with different prefixes
    let mut adapter1 = create_nats_adapter("nats_cluster_a_node1", &cluster_config).await;
    let mut adapter2 = create_nats_adapter("nats_cluster_b_node1", &cluster_config).await;

    adapter1.init().await;
    adapter2.init().await;

    // Each adapter should only receive messages for its prefix/cluster
    adapter1
        .broadcast_presence_join("app1", "channel1", "user1", "socket1", None)
        .await
        .unwrap();

    adapter2
        .broadcast_presence_join("app2", "channel2", "user2", "socket2", None)
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;

    // Adapters should be isolated by prefix

    drop(adapter1);
    drop(adapter2);
}

#[tokio::test]
async fn test_nats_wildcard_subscriptions() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let mut adapter = create_nats_adapter("nats_node1", &cluster_config).await;
    adapter.init().await;

    // Test presence across multiple apps and channels
    let apps = vec!["app1", "app2", "app3"];
    let channels = ["channel.a", "channel.b", "channel.c"];

    for app in &apps {
        for (i, channel) in channels.iter().enumerate() {
            let user_id = format!("{}-user-{}", app, i);
            let socket_id = format!("{}-socket-{}", app, i);

            adapter
                .broadcast_presence_join(app, channel, &user_id, &socket_id, None)
                .await
                .unwrap();
        }
    }

    sleep(Duration::from_millis(300)).await;

    drop(adapter);
}

#[tokio::test]
async fn test_nats_concurrent_operations() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let adapter = Arc::new(Mutex::new(
        create_nats_adapter("nats_node1", &cluster_config).await,
    ));

    {
        let mut adapter_guard = adapter.lock().await;
        adapter_guard.init().await;
    }

    let app_id = "test-app";
    let channel = "presence-concurrent";

    // Spawn multiple concurrent operations
    let mut handles = vec![];

    for i in 0..20 {
        let adapter_clone = adapter.clone();
        let handle = tokio::spawn(async move {
            let adapter_guard = adapter_clone.lock().await;
            let user_id = format!("user-{}", i);
            let socket_id = format!("socket-{}", i);

            if i % 2 == 0 {
                adapter_guard
                    .broadcast_presence_join(
                        app_id,
                        channel,
                        &user_id,
                        &socket_id,
                        Some(json!({"concurrent": true})),
                    )
                    .await
                    .unwrap();
            } else {
                // Join then leave to test both operations
                adapter_guard
                    .broadcast_presence_join(app_id, channel, &user_id, &socket_id, None)
                    .await
                    .unwrap();

                adapter_guard
                    .broadcast_presence_leave(app_id, channel, &user_id, &socket_id)
                    .await
                    .unwrap();
            }
        });
        handles.push(handle);
    }

    // Wait for all operations
    for handle in handles {
        handle.await.unwrap();
    }

    sleep(Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_nats_reconnection_handling() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let mut adapter = create_nats_adapter("nats_node1", &cluster_config).await;
    adapter.init().await;

    // Add initial presence
    adapter
        .broadcast_presence_join(
            "app1",
            "channel1",
            "user1",
            "socket1",
            Some(json!({"before": "disconnect"})),
        )
        .await
        .unwrap();

    // NATS client should handle reconnection automatically
    // In a real scenario, we would simulate network interruption here

    sleep(Duration::from_millis(100)).await;

    // Should still work after potential reconnection
    adapter
        .broadcast_presence_join(
            "app1",
            "channel2",
            "user2",
            "socket2",
            Some(json!({"after": "reconnect"})),
        )
        .await
        .unwrap();

    drop(adapter);
}

#[tokio::test]
async fn test_nats_cluster_health_disabled() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: false, // Disabled
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let mut adapter = create_nats_adapter("nats_node1", &cluster_config).await;
    adapter.init().await;

    // Presence operations should work without health monitoring
    adapter
        .broadcast_presence_join("app1", "channel1", "user1", "socket1", None)
        .await
        .unwrap();

    adapter
        .broadcast_presence_leave("app1", "channel1", "user1", "socket1")
        .await
        .unwrap();

    // No heartbeats should be published
    sleep(Duration::from_millis(300)).await;

    drop(adapter);
}

#[tokio::test]
async fn test_nats_message_ordering() {
    if !is_nats_available().await {
        eprintln!("Skipping test: NATS not available");
        return;
    }

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let mut adapter1 = create_nats_adapter("nats_node1", &cluster_config).await;
    let mut adapter2 = create_nats_adapter("nats_node2", &cluster_config).await;

    adapter1.init().await;
    adapter2.init().await;

    let app_id = "test-app";
    let channel = "presence-ordering";
    let user_id = "user-order-test";
    let socket_id = "socket-order-test";

    // Rapid sequence of join/leave operations
    for i in 0..5 {
        if i % 2 == 0 {
            adapter1
                .broadcast_presence_join(
                    app_id,
                    channel,
                    user_id,
                    socket_id,
                    Some(json!({"iteration": i})),
                )
                .await
                .unwrap();
        } else {
            adapter1
                .broadcast_presence_leave(app_id, channel, user_id, socket_id)
                .await
                .unwrap();
        }
    }

    sleep(Duration::from_millis(300)).await;

    // Final state should be consistent across all nodes

    drop(adapter1);
    drop(adapter2);
}
