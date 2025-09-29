use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use serde_json::json;
use sockudo::adapter::connection_manager::HorizontalAdapterInterface;
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::options::ClusterHealthConfig;

#[tokio::test]
async fn test_presence_member_join_broadcast() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Configure cluster health enabled for presence broadcasting
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    // Simulate discovered nodes for multi-node behavior
    let adapter_mut = adapter_mut.with_discovered_nodes(vec!["node-1", "node-2"]).await.unwrap();

    let user_info = json!({"name": "Alice", "status": "online"});

    // Broadcast presence join
    let result = adapter_mut
        .broadcast_presence_join(
            "test-app",
            "presence-channel",
            "user-123",
            "socket-456",
            Some(user_info.clone()),
        )
        .await;

    assert!(result.is_ok(), "Presence join broadcast should succeed");

    // Verify the request was published
    let requests = adapter_mut.transport.get_published_requests().await;
    assert_eq!(
        requests.len(),
        1,
        "Should have published one presence join request"
    );

    let request = &requests[0];
    assert_eq!(request.app_id, "test-app");
    assert_eq!(request.channel, Some("presence-channel".to_string()));
    assert_eq!(request.user_id, Some("user-123".to_string()));
    assert_eq!(request.socket_id, Some("socket-456".to_string()));
    assert_eq!(request.user_info, Some(user_info));
    assert_eq!(
        request.request_type,
        sockudo::adapter::horizontal_adapter::RequestType::PresenceMemberJoined
    );
}

#[tokio::test]
async fn test_presence_member_leave_broadcast() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Configure cluster health enabled
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    // Simulate discovered nodes for multi-node behavior
    let adapter_mut = adapter_mut.with_discovered_nodes(vec!["node-1", "node-2"]).await.unwrap();

    // Broadcast presence leave
    let result = adapter_mut
        .broadcast_presence_leave("test-app", "presence-channel", "user-123", "socket-456")
        .await;

    assert!(result.is_ok(), "Presence leave broadcast should succeed");

    // Verify the request was published
    let requests = adapter_mut.transport.get_published_requests().await;
    assert_eq!(
        requests.len(),
        1,
        "Should have published one presence leave request"
    );

    let request = &requests[0];
    assert_eq!(request.app_id, "test-app");
    assert_eq!(request.channel, Some("presence-channel".to_string()));
    assert_eq!(request.user_id, Some("user-123".to_string()));
    assert_eq!(request.socket_id, Some("socket-456".to_string()));
    assert_eq!(request.user_info, None); // Leave requests don't include user info
    assert_eq!(
        request.request_type,
        sockudo::adapter::horizontal_adapter::RequestType::PresenceMemberLeft
    );
}

#[tokio::test]
async fn test_presence_broadcast_skipped_when_cluster_health_disabled() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Configure cluster health DISABLED
    let cluster_config = ClusterHealthConfig {
        enabled: false,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    let user_info = json!({"name": "Alice", "status": "online"});

    // Try to broadcast presence join
    let result = adapter_mut
        .broadcast_presence_join(
            "test-app",
            "presence-channel",
            "user-123",
            "socket-456",
            Some(user_info),
        )
        .await;

    assert!(result.is_ok(), "Function should succeed but skip broadcast");

    // Verify NO request was published when cluster health is disabled
    let requests = adapter_mut.transport.get_published_requests().await;
    assert_eq!(
        requests.len(),
        0,
        "Should not publish when cluster health disabled"
    );
}

#[tokio::test]
async fn test_presence_local_registry_update_always_happens() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Test with cluster health disabled to verify local registry still works
    let cluster_config = ClusterHealthConfig {
        enabled: false,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    let user_info = json!({"name": "Bob", "status": "away"});

    // Join presence
    let result = adapter_mut
        .broadcast_presence_join(
            "test-app",
            "presence-channel",
            "user-456",
            "socket-789",
            Some(user_info.clone()),
        )
        .await;

    assert!(result.is_ok(), "Presence join should succeed");

    // Verify local registry was updated even with cluster health disabled
    {
        let horizontal = adapter_mut.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;

        // Should have entry for our node
        assert!(
            registry.contains_key(&adapter_mut.node_id),
            "Local registry should be updated"
        );

        let node_data = &registry[&adapter_mut.node_id];
        assert!(
            node_data.contains_key("presence-channel"),
            "Channel should exist in registry"
        );

        let channel_data = &node_data["presence-channel"];
        assert!(
            channel_data.contains_key("socket-789"),
            "Socket should exist in channel"
        );

        let socket_entry = &channel_data["socket-789"];
        assert_eq!(socket_entry.user_id, "user-456");
        assert_eq!(socket_entry.app_id, "test-app");
        assert_eq!(socket_entry.user_info, Some(user_info));
    }

    // Now test leave
    let result = adapter_mut
        .broadcast_presence_leave("test-app", "presence-channel", "user-456", "socket-789")
        .await;

    assert!(result.is_ok(), "Presence leave should succeed");

    // Verify local registry was cleaned up
    {
        let horizontal = adapter_mut.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;

        if let Some(node_data) = registry.get(&adapter_mut.node_id)
            && let Some(channel_data) = node_data.get("presence-channel")
        {
            assert!(
                !channel_data.contains_key("socket-789"),
                "Socket should be removed from registry"
            );
        }
    }
}

#[tokio::test]
async fn test_multiple_presence_members_same_channel() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    // Simulate discovered nodes for multi-node behavior
    let adapter_mut = adapter_mut.with_discovered_nodes(vec!["node-1", "node-2"]).await.unwrap();

    // Add multiple presence members to same channel
    let users = vec![
        (
            "user-1",
            "socket-1",
            json!({"name": "Alice", "role": "admin"}),
        ),
        ("user-2", "socket-2", json!({"name": "Bob", "role": "user"})),
        (
            "user-3",
            "socket-3",
            json!({"name": "Carol", "role": "moderator"}),
        ),
    ];

    for (user_id, socket_id, user_info) in &users {
        let result = adapter_mut
            .broadcast_presence_join(
                "test-app",
                "presence-channel",
                user_id,
                socket_id,
                Some(user_info.clone()),
            )
            .await;

        assert!(
            result.is_ok(),
            "Presence join should succeed for {}",
            user_id
        );
    }

    // Verify all members are in local registry
    {
        let horizontal = adapter_mut.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;

        // Use safe access with proper error handling
        let node_data = registry
            .get(&adapter_mut.node_id)
            .expect("Node should exist in registry");
        let channel_data = node_data
            .get("presence-channel")
            .expect("Channel should exist in node data");

        assert_eq!(channel_data.len(), 3, "Should have 3 presence members");

        for (user_id, socket_id, user_info) in &users {
            assert!(
                channel_data.contains_key(*socket_id),
                "Socket {} should exist",
                socket_id
            );
            let entry = channel_data
                .get(*socket_id)
                .unwrap_or_else(|| panic!("Socket {} should exist", socket_id));
            assert_eq!(entry.user_id, *user_id);
            assert_eq!(entry.user_info, Some(user_info.clone()));
        }
    }

    // Verify correct number of broadcast requests were sent
    let requests = adapter_mut.transport.get_published_requests().await;
    assert_eq!(
        requests.len(),
        3,
        "Should have published 3 presence join requests"
    );
}

#[tokio::test]
async fn test_presence_state_consistency_across_join_leave() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 1000,
        node_timeout_ms: 5000,
        cleanup_interval_ms: 2000,
    };

    let mut adapter_mut = adapter;
    adapter_mut
        .set_cluster_health(&cluster_config)
        .await
        .unwrap();
    adapter_mut.start_listeners().await.unwrap();

    // Simulate discovered nodes for multi-node behavior
    let adapter_mut = adapter_mut.with_discovered_nodes(vec!["node-1", "node-2"]).await.unwrap();

    let user_info = json!({"name": "Test User", "status": "online"});

    // Pattern: Join -> Leave -> Join again
    let result = adapter_mut
        .broadcast_presence_join(
            "test-app",
            "test-channel",
            "user-1",
            "socket-1",
            Some(user_info.clone()),
        )
        .await;
    assert!(result.is_ok(), "First join should succeed");

    let result = adapter_mut
        .broadcast_presence_leave("test-app", "test-channel", "user-1", "socket-1")
        .await;
    assert!(result.is_ok(), "Leave should succeed");

    let result = adapter_mut
        .broadcast_presence_join(
            "test-app",
            "test-channel",
            "user-1",
            "socket-2", // Different socket for same user
            Some(user_info.clone()),
        )
        .await;
    assert!(result.is_ok(), "Second join with new socket should succeed");

    // Verify final state
    {
        let horizontal = adapter_mut.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;

        // Use safe access
        let node_data = registry
            .get(&adapter_mut.node_id)
            .expect("Node should exist in registry");
        let channel_data = node_data
            .get("test-channel")
            .expect("Channel should exist in node data");

        // Should only have socket-2 now (socket-1 was removed)
        assert!(
            !channel_data.contains_key("socket-1"),
            "Old socket should be removed"
        );
        assert!(
            channel_data.contains_key("socket-2"),
            "New socket should be present"
        );
        assert_eq!(channel_data.len(), 1, "Should have exactly 1 socket");
    }

    // Verify correct sequence of requests
    let requests = adapter_mut.transport.get_published_requests().await;
    assert_eq!(requests.len(), 3, "Should have 3 requests total");

    use sockudo::adapter::horizontal_adapter::RequestType;
    assert_eq!(requests[0].request_type, RequestType::PresenceMemberJoined);
    assert_eq!(requests[1].request_type, RequestType::PresenceMemberLeft);
    assert_eq!(requests[2].request_type, RequestType::PresenceMemberJoined);
}
