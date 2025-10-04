use super::horizontal_adapter_helpers::{MockConfig, MockTransport};
use sockudo::adapter::ConnectionManager;
use sockudo::adapter::horizontal_adapter::RequestType;
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::adapter::horizontal_transport::{HorizontalTransport, TransportConfig};
use sockudo::error::Result;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use sockudo::websocket::SocketId;
use sonic_rs::JsonValueTrait;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_horizontal_adapter_base_new() -> Result<()> {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;

    // Should be able to create successfully
    assert_eq!(adapter.config.prefix(), "test");
    assert_eq!(adapter.config.request_timeout_ms(), 1000);

    Ok(())
}

#[tokio::test]
async fn test_horizontal_adapter_base_new_failure() -> Result<()> {
    let config = MockConfig {
        prefix: "fail_new".to_string(),
        simulate_failures: true,
        ..Default::default()
    };

    let result = HorizontalAdapterBase::<MockTransport>::new(config).await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_realistic_socket_aggregation() -> Result<()> {
    // Use default config with realistic overlapping data:
    // node-1: ["socket-1", "socket-2", "socket-shared"]
    // node-2: ["socket-3", "socket-4", "socket-shared"]
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter
        .with_discovered_nodes(vec!["node-1", "node-2"])
        .await?;

    let response = adapter
        .send_request("test-app", RequestType::Sockets, None, None, None)
        .await?;

    // Should aggregate all unique sockets from both nodes
    assert_eq!(response.app_id, "test-app");
    assert!(!response.request_id.is_empty());

    // Expect summed count from both nodes: 3 + 3 = 6 total sockets
    assert_eq!(response.sockets_count, 6);

    // Verify specific socket aggregation (deduplicated)
    let expected_sockets: HashSet<String> = [
        "socket-1",
        "socket-2",
        "socket-3",
        "socket-4",
        "socket-shared",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let actual_sockets: HashSet<String> = response.socket_ids.into_iter().collect();
    assert_eq!(actual_sockets, expected_sockets);

    Ok(())
}

#[tokio::test]
async fn test_conflicting_data_handling() -> Result<()> {
    // Test how adapter handles conflicting presence data across nodes
    let config = MockTransport::conflicting_data();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter
        .with_discovered_nodes(vec!["node-1", "node-2"])
        .await?;

    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelMembers,
            Some("presence-channel"),
            None,
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");

    // Should have merged member data from both nodes
    // The adapter should handle conflicting user info (last writer wins or merge)
    assert!(response.members.contains_key("user-1"));

    // Verify that some user data is present (either from node-1 or node-2)
    let user_info = &response.members["user-1"];
    assert!(user_info.user_info.as_ref().unwrap().get("name").is_some());
    assert!(
        user_info
            .user_info
            .as_ref()
            .unwrap()
            .get("status")
            .is_some()
    );

    Ok(())
}

#[tokio::test]
async fn test_channel_socket_overlap_aggregation() -> Result<()> {
    // Test channel membership with overlapping sockets
    let adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelSockets,
            Some("channel-1"),
            None,
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");

    // channel-1 on node-1: ["socket-1", "socket-shared"]
    // channel-1 on node-2: ["socket-3", "socket-shared"]
    // Expected: ["socket-1", "socket-3", "socket-shared"] = 3 unique sockets
    assert_eq!(response.sockets_count, 3);

    let expected_sockets: HashSet<String> = ["socket-1", "socket-3", "socket-shared"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let actual_sockets: HashSet<String> = response.socket_ids.into_iter().collect();
    assert_eq!(actual_sockets, expected_sockets);

    Ok(())
}

#[tokio::test]
async fn test_send_request_timeout_with_partial_responses() -> Result<()> {
    let config = MockTransport::partial_failures(); // 500ms timeout, node-3 responds in 2000ms
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter
        .with_discovered_nodes(vec!["node-1", "node-2", "node-3"])
        .await?;

    let start = std::time::Instant::now();

    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelSockets,
            Some("test-channel"),
            None,
            None,
        )
        .await?;

    let duration = start.elapsed();

    // Should timeout around 500ms (allowing 200ms tolerance for processing)
    assert!(duration >= Duration::from_millis(400));
    assert!(duration <= Duration::from_millis(700));

    // Should aggregate responses from nodes that responded in time
    assert_eq!(response.app_id, "test-app");

    // Only node-1 should respond in time: ["socket-1"] from test-channel
    // node-2 won't respond, node-3 responds too late
    assert_eq!(response.sockets_count, 1);
    assert_eq!(response.socket_ids, vec!["socket-1"]);

    Ok(())
}

#[tokio::test]
async fn test_corrupt_response_handling() -> Result<()> {
    let config = MockTransport::corrupt_responses();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    let response = adapter
        .send_request("test-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "test-app");

    // Should handle corrupt data gracefully
    // node-1 returns corrupted data, node-2 returns clean data
    // The adapter should either:
    // 1. Filter out corrupted responses, or
    // 2. Include them but handle safely

    // At minimum, should not crash and should return valid response structure
    assert!(response.sockets_count > 0 || response.socket_ids.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_multi_node_socket_aggregation() -> Result<()> {
    // Test socket aggregation across multiple nodes with overlapping sockets
    let adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    let response = adapter
        .send_request("test-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "test-app");

    // Default config: Node1[socket-1,socket-2,socket-shared] + Node2[socket-3,socket-4,socket-shared]
    // RequestType::Sockets returns unique socket IDs but raw total count across nodes
    assert_eq!(
        response.sockets_count, 6,
        "Should count all sockets including duplicates across nodes"
    );
    assert_eq!(
        response.socket_ids.len(),
        5,
        "Should return deduplicated socket IDs"
    );

    // Verify all expected unique sockets are present
    let unique_sockets: HashSet<String> = response.socket_ids.into_iter().collect();
    assert_eq!(
        unique_sockets.len(),
        5,
        "Should have exactly 5 unique sockets"
    );

    let expected_sockets: HashSet<String> = [
        "socket-1",
        "socket-2",
        "socket-3",
        "socket-4",
        "socket-shared",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        unique_sockets, expected_sockets,
        "Should contain all expected sockets"
    );

    Ok(())
}

#[tokio::test]
async fn test_concurrent_request_isolation() -> Result<()> {
    let adapter = Arc::new(MockConfig::create_multi_node_adapter().await?);
    adapter.start_listeners().await?;

    // Send multiple concurrent requests of different types
    let mut handles = Vec::new();

    for i in 0..5 {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            adapter
                .send_request(
                    &format!("test-app-{}", i),
                    if i % 2 == 0 {
                        RequestType::Sockets
                    } else {
                        RequestType::ChannelSockets
                    },
                    if i % 2 == 1 { Some("channel-1") } else { None },
                    None,
                    None,
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All should succeed with correct responses
    assert_eq!(responses.len(), 5);

    // Verify each response matches its request type
    for (i, response) in responses.iter().enumerate() {
        assert_eq!(response.app_id, format!("test-app-{}", i));

        if i % 2 == 0 {
            // Sockets request: should return summed count (6 total: 3+3)
            assert_eq!(response.sockets_count, 6);
        } else {
            // ChannelSockets request for channel-1: should return deduplicated count (3 unique: socket-1, socket-3, socket-shared)
            assert_eq!(response.sockets_count, 3);
        }
    }

    // Each should have unique request IDs
    let mut request_ids = HashSet::new();
    for response in &responses {
        assert!(request_ids.insert(response.request_id.clone()));
    }

    Ok(())
}

#[tokio::test]
async fn test_transport_health_failure_propagation() -> Result<()> {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;

    // Should be healthy initially
    adapter.transport.check_health().await?;

    // Make transport unhealthy
    adapter.transport.set_health_status(false).await;

    let result = adapter.transport.check_health().await;
    assert!(result.is_err());

    let error_msg = format!("{}", result.unwrap_err());
    assert!(error_msg.contains("unhealthy"));

    Ok(())
}

#[tokio::test]
async fn test_dynamic_node_count_handling() -> Result<()> {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;

    // Initial node count should match config
    let initial_count = adapter.transport.get_node_count().await?;
    assert_eq!(initial_count, 2); // Default config has 2 nodes

    // Simulate node joining
    adapter.transport.add_node().await;
    let new_count = adapter.transport.get_node_count().await?;
    assert_eq!(new_count, 3);

    // Simulate node leaving
    adapter.transport.remove_node().await;
    let final_count = adapter.transport.get_node_count().await?;
    assert_eq!(final_count, 2);

    Ok(())
}

#[tokio::test]
async fn test_user_socket_aggregation() -> Result<()> {
    // Test user-specific socket aggregation across nodes
    let adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    // Query for shared-user who has sockets on both nodes
    let response = adapter
        .send_request(
            "test-app",
            RequestType::CountUserConnectionsInChannel,
            Some("channel-1"),
            None,
            Some("shared-user"),
        )
        .await?;

    assert_eq!(response.app_id, "test-app");

    // shared-user has socket-shared on both nodes, which is in channel-1 on both
    // Count is summed: 1 from node-1 + 1 from node-2 = 2
    assert_eq!(response.sockets_count, 2);

    Ok(())
}

#[tokio::test]
async fn test_broadcast_message_verification() -> Result<()> {
    let mut adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        name: None,
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("test message content".to_string())),
        user_id: None,
    };

    // Send broadcast
    adapter
        .send(
            "test-channel",
            message.clone(),
            None,
            "test-app",
            Some(123.45),
        )
        .await?;

    // Verify broadcast was published correctly
    let published_broadcasts = adapter.transport.get_published_broadcasts().await;
    assert_eq!(published_broadcasts.len(), 1);

    let broadcast = &published_broadcasts[0];
    assert_eq!(broadcast.app_id, "test-app");
    assert_eq!(broadcast.channel, "test-channel");
    assert!(broadcast.message.contains("test message content"));
    assert_eq!(broadcast.timestamp_ms, Some(123.45));

    Ok(())
}

#[tokio::test]
async fn test_request_id_uniqueness_under_load() -> Result<()> {
    let adapter = Arc::new(MockConfig::create_multi_node_adapter().await?);
    adapter.start_listeners().await?;

    // Generate many concurrent requests to test ID uniqueness
    let mut handles = Vec::new();

    for _i in 0..50 {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            adapter
                .send_request("load-test-app", RequestType::Sockets, None, None, None)
                .await
        });
        handles.push(handle);
    }

    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All requests should succeed
    assert_eq!(responses.len(), 50);

    // All request IDs should be unique
    let mut request_ids = HashSet::new();
    for response in &responses {
        assert!(
            request_ids.insert(response.request_id.clone()),
            "Duplicate request ID found: {}",
            response.request_id
        );
    }

    // All should have consistent socket count (6 total from both nodes: 3+3)
    for response in &responses {
        assert_eq!(response.sockets_count, 6);
    }

    Ok(())
}

#[tokio::test]
async fn test_socket_existence_validation() -> Result<()> {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter
        .with_discovered_nodes(vec!["node-1", "node-2"])
        .await?;

    // Test existing socket
    let response = adapter
        .send_request(
            "test-app",
            RequestType::SocketExistsInChannel,
            Some("channel-1"),
            Some("socket-shared"),
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");
    assert!(response.exists); // socket-shared is in channel-1 on both nodes

    // Test non-existing socket
    let response = adapter
        .send_request(
            "test-app",
            RequestType::SocketExistsInChannel,
            Some("channel-1"),
            Some("non-existent-socket"),
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");
    assert!(!response.exists); // non-existent-socket is not in any channel

    Ok(())
}

#[tokio::test]
async fn test_presence_member_data_integrity() -> Result<()> {
    let adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelMembers,
            Some("presence-channel"),
            None,
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");
    assert_eq!(response.members_count, 2); // user-1 and user-2

    // Verify specific member data
    assert!(response.members.contains_key("user-1"));
    assert!(response.members.contains_key("user-2"));

    let user1_info = &response.members["user-1"];
    assert_eq!(
        user1_info.user_info.as_ref().unwrap().get("name").unwrap(),
        "Alice"
    );

    let user2_info = &response.members["user-2"];
    assert_eq!(
        user2_info.user_info.as_ref().unwrap().get("name").unwrap(),
        "Bob"
    );

    Ok(())
}

// Connection Manager tests with realistic validation
#[tokio::test]
async fn test_connection_manager_distributed_socket_count() -> Result<()> {
    let adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    let count = adapter.get_sockets_count("test-app").await?;

    // Should sum socket counts from all nodes: 3 + 3 = 6
    assert_eq!(count, 6); // Total socket count across both nodes

    Ok(())
}

#[tokio::test]
async fn test_connection_manager_channel_specific_operations() -> Result<()> {
    let mut adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    // Test channel socket count
    let count = adapter
        .get_channel_socket_count("test-app", "channel-1")
        .await;
    assert_eq!(count, 4); // channel-1: 2 from node-1 + 2 from node-2 = 4

    // Test channel members for presence channel
    let members = adapter
        .get_channel_members("test-app", "presence-channel")
        .await?;
    assert_eq!(members.len(), 2); // user-1 and user-2
    assert!(members.contains_key("user-1"));
    assert!(members.contains_key("user-2"));

    // Test channel sockets
    let sockets = adapter.get_channel_sockets("test-app", "channel-1").await?;
    let expected_sockets: HashSet<String> = ["socket-1", "socket-3", "socket-shared"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let actual_sockets: HashSet<String> = sockets.into_iter().map(|s| s.to_string()).collect();
    assert_eq!(actual_sockets, expected_sockets);

    Ok(())
}

#[tokio::test]
async fn test_connection_manager_user_operations() -> Result<()> {
    let mut adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    // Test user sockets aggregation via horizontal communication
    let response = adapter
        .send_request(
            "test-app",
            RequestType::Sockets,
            None,
            None,
            Some("shared-user"),
        )
        .await?;
    assert_eq!(response.sockets_count, 6); // Total sockets from both nodes (user_id is ignored for Sockets request)
    assert!(!response.socket_ids.is_empty(), "Should have socket data");

    // Test user connection count in channel
    let count = adapter
        .count_user_connections_in_channel("shared-user", "test-app", "channel-1", None)
        .await?;
    assert_eq!(count, 2); // shared-user has socket-shared in channel-1 on both nodes (summed)

    Ok(())
}

#[tokio::test]
async fn test_connection_manager_socket_existence() -> Result<()> {
    let mut adapter = MockConfig::create_multi_node_adapter().await?;
    adapter.start_listeners().await?;

    let socket_id = SocketId("socket-shared".to_string());

    // Test socket exists in channel
    let exists = adapter
        .is_in_channel("test-app", "channel-1", &socket_id)
        .await?;
    assert!(exists);

    // Test socket doesn't exist in non-existent channel
    let exists = adapter
        .is_in_channel("test-app", "non-existent-channel", &socket_id)
        .await?;
    assert!(!exists);

    Ok(())
}

#[tokio::test]
async fn test_connection_manager_error_propagation() -> Result<()> {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.init().await;

    let socket_id = SocketId("test-socket".to_string());
    let message = PusherMessage {
        channel: None,
        name: None,
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("test message".to_string())),
        user_id: None,
    };

    // These operations should complete without error (may succeed or fail gracefully)
    let _result1 = adapter
        .add_to_channel("test-app", "test-channel", &socket_id)
        .await;
    let _result2 = adapter
        .remove_from_channel("test-app", "test-channel", &socket_id)
        .await;
    let _result3 = adapter.send_message("test-app", &socket_id, message).await;
    let _result4 = adapter
        .terminate_user_connections("test-app", "test-user")
        .await;

    // Should not panic or cause undefined behavior
    adapter.remove_channel("test-app", "test-channel").await;

    Ok(())
}
