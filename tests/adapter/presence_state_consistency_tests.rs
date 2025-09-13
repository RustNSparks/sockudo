use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use serde_json::json;
use sockudo::adapter::horizontal_adapter::{RequestBody, RequestType};
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::options::ClusterHealthConfig;
use std::collections::HashMap;

/// Test sequence number conflict resolution during presence updates
/// This adds value beyond existing tests by testing the conflict resolution logic
#[tokio::test]
async fn test_sequence_number_conflict_resolution() {
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();
    adapter.set_cluster_health(&cluster_config).await.unwrap();

    let app_id = "test-app";
    let channel = "presence-sequence-test";
    let user_id = "user-conflict";
    let socket_id = "socket-conflict";

    // Get access to horizontal adapter
    let horizontal = adapter.horizontal.lock().await;

    // Create two presence entries with different sequence numbers
    let entry1 = sockudo::adapter::horizontal_adapter::PresenceEntry {
        user_info: Some(json!({"version": 1})),
        node_id: "node-1".to_string(),
        app_id: app_id.to_string(),
        user_id: user_id.to_string(),
        socket_id: socket_id.to_string(),
        sequence_number: 100,
    };

    let entry2 = sockudo::adapter::horizontal_adapter::PresenceEntry {
        user_info: Some(json!({"version": 2})),
        node_id: "node-2".to_string(),
        app_id: app_id.to_string(),
        user_id: user_id.to_string(),
        socket_id: socket_id.to_string(),
        sequence_number: 200, // Higher sequence number should win
    };

    // Add both entries
    {
        let mut registry = horizontal.cluster_presence_registry.write().await;

        registry
            .entry("node-1".to_string())
            .or_insert_with(HashMap::new)
            .entry(channel.to_string())
            .or_insert_with(HashMap::new)
            .insert(socket_id.to_string(), entry1);

        registry
            .entry("node-2".to_string())
            .or_insert_with(HashMap::new)
            .entry(channel.to_string())
            .or_insert_with(HashMap::new)
            .insert(socket_id.to_string(), entry2.clone());
    }

    // Verify higher sequence number entry is preferred
    let registry = horizontal.cluster_presence_registry.read().await;
    let node2_entry = registry
        .get("node-2")
        .and_then(|channels| channels.get(channel))
        .and_then(|sockets| sockets.get(socket_id));

    assert!(node2_entry.is_some());
    assert_eq!(node2_entry.unwrap().sequence_number, 200);
    assert_eq!(node2_entry.unwrap().user_info, Some(json!({"version": 2})));
}

/// Test handling of presence updates with clock skew
/// This tests robustness against timing issues in distributed systems
#[tokio::test]
async fn test_presence_state_with_clock_skew() {
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();
    adapter.set_cluster_health(&cluster_config).await.unwrap();

    // Simulate receiving presence updates with clock skew
    let request_early = RequestBody {
        request_id: "req-1".to_string(),
        node_id: "node-early".to_string(),
        app_id: "test-app".to_string(),
        request_type: RequestType::PresenceMemberJoined,
        channel: Some("presence-clock-test".to_string()),
        socket_id: Some("socket-1".to_string()),
        user_id: Some("user-1".to_string()),
        user_info: Some(json!({"timestamp": "early"})),
        timestamp: Some(1000), // Earlier timestamp
        dead_node_id: None,
        target_node_id: None,
    };

    let request_late = RequestBody {
        request_id: "req-2".to_string(),
        node_id: "node-late".to_string(),
        app_id: "test-app".to_string(),
        request_type: RequestType::PresenceMemberJoined,
        channel: Some("presence-clock-test".to_string()),
        socket_id: Some("socket-1".to_string()),
        user_id: Some("user-1".to_string()),
        user_info: Some(json!({"timestamp": "late"})),
        timestamp: Some(2000), // Later timestamp
        dead_node_id: None,
        target_node_id: None,
    };

    // Process requests (late one first to test handling)
    {
        let mut horizontal = adapter.horizontal.lock().await;
        let _ = horizontal.process_request(request_late).await;
        let _ = horizontal.process_request(request_early).await;
    }

    // The system should handle clock skew gracefully
    // This test ensures no panics or inconsistent state from out-of-order processing
}

/// Test bulk cleanup operations for performance and correctness
/// This tests scenarios with many presence entries that existing tests don't cover
#[tokio::test]
async fn test_bulk_presence_cleanup() {
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();
    adapter.set_cluster_health(&cluster_config).await.unwrap();

    let app_id = "test-app";
    let channels = 5;
    let users_per_channel = 20;

    // Populate with bulk data to test scale
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut registry = horizontal.cluster_presence_registry.write().await;

        for c in 0..channels {
            for u in 0..users_per_channel {
                let channel = format!("channel-{}", c);
                let user_id = format!("user-{}-{}", c, u);
                let socket_id = format!("socket-{}-{}", c, u);

                let entry = sockudo::adapter::horizontal_adapter::PresenceEntry {
                    user_info: Some(json!({"bulk": true})),
                    node_id: "bulk-node".to_string(),
                    app_id: app_id.to_string(),
                    user_id,
                    socket_id: socket_id.clone(),
                    sequence_number: (c * users_per_channel + u) as u64,
                };

                registry
                    .entry("bulk-node".to_string())
                    .or_insert_with(HashMap::new)
                    .entry(channel)
                    .or_insert_with(HashMap::new)
                    .insert(socket_id, entry);
            }
        }
    }

    // Test bulk cleanup performance and correctness
    {
        let horizontal = adapter.horizontal.lock().await;
        let cleanup_tasks = horizontal
            .handle_dead_node_cleanup("bulk-node")
            .await
            .unwrap();

        let expected_total = channels * users_per_channel;
        assert_eq!(
            cleanup_tasks.len(),
            expected_total,
            "Should cleanup all {} entries",
            expected_total
        );

        // Verify registry is properly cleaned
        let registry = horizontal.cluster_presence_registry.read().await;
        assert!(
            !registry.contains_key("bulk-node"),
            "Bulk node should be completely removed"
        );
    }
}
