use sockudo::adapter::ConnectionManager;
use sockudo::adapter::horizontal_adapter::RequestType;
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::adapter::horizontal_transport::HorizontalTransport;
use sockudo::error::Result;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use sockudo::websocket::SocketId;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use super::horizontal_adapter_helpers::{MockConfig, MockNodeState, MockTransport};

/// Test comprehensive failure scenarios and edge cases
#[tokio::test]
async fn test_transport_publish_failure_recovery() -> Result<()> {
    let config = MockConfig {
        simulate_failures: true,
        ..Default::default()
    };

    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let mut adapter = adapter.with_discovered_nodes(vec!["node-1", "node-2"]).await?;

    let message = PusherMessage {
        channel: None,
        name: None,
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("simulate_error".to_string())), // Triggers failure
        user_id: None,
    };

    // Send should fail due to simulated transport error
    let result = adapter
        .send("test-channel", message, None, "test-app", None)
        .await;
    assert!(result.is_err());

    // But adapter should remain functional for other operations
    let valid_message = PusherMessage {
        channel: None,
        name: None,
        event: Some("valid-event".to_string()),
        data: Some(MessageData::String("valid message".to_string())),
        user_id: None,
    };

    // This should succeed
    adapter
        .send("test-channel", valid_message, None, "test-app", None)
        .await?;

    Ok(())
}

#[tokio::test]
async fn test_request_publish_failure_recovery() -> Result<()> {
    let config = MockConfig {
        simulate_failures: true,
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter.with_discovered_nodes(vec!["node-1", "node-2"]).await?;

    // Request with failing app_id should fail
    let result = adapter
        .send_request("fail_request", RequestType::Sockets, None, None, None)
        .await;
    assert!(result.is_err());

    // But valid requests should still work
    let response = adapter
        .send_request("valid-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "valid-app");
    assert!(!response.request_id.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_partial_node_response_aggregation() -> Result<()> {
    let config = MockTransport::partial_failures();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter.with_discovered_nodes(vec!["node-1", "node-2", "node-3"]).await?;

    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelSockets,
            Some("test-channel"),
            None,
            None,
        )
        .await?;

    assert_eq!(response.app_id, "test-app");

    // Only node-1 responds in time with ["socket-1"]
    // node-2 won't respond, node-3 times out
    assert_eq!(response.sockets_count, 1);
    assert_eq!(response.socket_ids, vec!["socket-1"]);

    Ok(())
}

#[tokio::test]
async fn test_all_nodes_timeout_scenario() -> Result<()> {
    // Configure all nodes to respond too slowly
    let node1 = MockNodeState::new("node-1")
        .with_sockets(vec!["socket-1"])
        .with_response_delay(2000);
    let node2 = MockNodeState::new("node-2")
        .with_sockets(vec!["socket-2"])
        .with_response_delay(2000);

    let config = MockConfig {
        prefix: "test".to_string(),
        request_timeout_ms: 500,
        simulate_failures: false,
        healthy: true,
        node_states: vec![node1, node2],
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    let start = std::time::Instant::now();

    let response = adapter
        .send_request("timeout-app", RequestType::Sockets, None, None, None)
        .await?;

    let duration = start.elapsed();

    // Should timeout around 500ms
    assert!(duration >= Duration::from_millis(400));
    assert!(duration <= Duration::from_millis(700));

    // Should still return valid response structure (empty or partial)
    assert_eq!(response.app_id, "timeout-app");
    assert!(!response.request_id.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_corrupted_response_filtering() -> Result<()> {
    let config = MockTransport::corrupt_responses();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter.with_discovered_nodes(vec!["node-1", "node-2"]).await?;

    let response = adapter
        .send_request("corrupt-test", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "corrupt-test");

    // The adapter should handle corrupted data gracefully
    // Either by filtering it out or handling extreme values safely

    // If corrupted data is included, verify it doesn't cause overflow
    if response
        .socket_ids
        .contains(&"CORRUPTED_DATA_💀".to_string())
    {
        // Corrupted data was included - verify system handled it
        assert!(response.sockets_count == 999_999_999 || response.sockets_count > 0);
    } else {
        // Corrupted data was filtered out - should have clean data from node-2
        assert_eq!(response.sockets_count, 1);
        assert_eq!(response.socket_ids, vec!["socket-2"]);
    }

    Ok(())
}

#[tokio::test]
async fn test_memory_exhaustion_protection() -> Result<()> {
    let config = MockTransport::large_scale();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior (10 nodes: node-0 to node-9)
    let node_ids: Vec<&str> = (0..10).map(|i| Box::leak(format!("node-{}", i).into_boxed_str()) as &str).collect();
    let adapter = adapter.with_discovered_nodes(node_ids).await?;

    // This tests whether the adapter can handle large responses without crashing
    let response = adapter
        .send_request("large-scale-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "large-scale-app");
    assert_eq!(response.sockets_count, 1000);
    assert_eq!(response.socket_ids.len(), 1000);

    // Verify memory usage is reasonable (no duplicates, proper cleanup)
    let unique_sockets: HashSet<String> = response.socket_ids.into_iter().collect();
    assert_eq!(unique_sockets.len(), 1000);

    Ok(())
}

#[tokio::test]
async fn test_concurrent_request_cleanup() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Send many concurrent requests and then immediately drop handles
    // Tests that request cleanup works properly even with abandoned requests
    for _ in 0..20 {
        let adapter = adapter.clone();
        tokio::spawn(async move {
            let _ = adapter
                .send_request("cleanup-test", RequestType::Sockets, None, None, None)
                .await;
        });
    }

    // Give some time for concurrent operations
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send a normal request to verify adapter is still functional
    let response = adapter
        .send_request("final-test", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "final-test");
    assert!(!response.request_id.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_handler_registration_race_condition() -> Result<()> {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;

    // Try to send requests before handlers are registered
    let _result1 = adapter
        .send_request("race-test-1", RequestType::Sockets, None, None, None)
        .await;

    // Start listeners (register handlers)
    adapter.start_listeners().await?;

    // Now requests should work
    let response2 = adapter
        .send_request("race-test-2", RequestType::Sockets, None, None, None)
        .await?;

    // First request may succeed or fail depending on implementation
    // Second request should definitely succeed
    assert_eq!(response2.app_id, "race-test-2");
    assert!(!response2.request_id.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_transport_health_during_active_requests() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Start long-running request
    let adapter_clone = adapter.clone();
    let request_handle = tokio::spawn(async move {
        adapter_clone
            .send_request("health-test", RequestType::Sockets, None, None, None)
            .await
    });

    // Change health status while request is active
    tokio::time::sleep(Duration::from_millis(10)).await;
    adapter.transport.set_health_status(false).await;

    // Check health - should fail
    let health_result = adapter.transport.check_health().await;
    assert!(health_result.is_err());

    // Original request should still complete
    let request_result = request_handle.await.unwrap();
    assert!(request_result.is_ok());

    let response = request_result?;
    assert_eq!(response.app_id, "health-test");

    Ok(())
}

#[tokio::test]
async fn test_node_join_leave_during_requests() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Initial node count
    let initial_count = adapter.transport.get_node_count().await?;
    assert_eq!(initial_count, 2);

    // Start concurrent requests while changing node count
    let mut handles = Vec::new();

    for i in 0..10 {
        let adapter_clone = adapter.clone();
        let handle = tokio::spawn(async move {
            // Some requests while nodes are joining/leaving
            if i % 3 == 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            adapter_clone
                .send_request(
                    &format!("dynamic-node-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await
        });
        handles.push(handle);

        // Simulate nodes joining and leaving during requests
        if i % 3 == 0 {
            adapter.transport.add_node().await;
        } else if i % 3 == 1 {
            adapter.transport.remove_node().await;
        }
    }

    // Wait for all requests
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All requests should succeed despite dynamic node changes
    assert_eq!(responses.len(), 10);

    for (i, response) in responses.iter().enumerate() {
        assert_eq!(response.app_id, format!("dynamic-node-{}", i));
        assert!(!response.request_id.is_empty());
    }

    Ok(())
}

#[tokio::test]
async fn test_duplicate_request_id_handling() -> Result<()> {
    // This tests edge case where request IDs might collide
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Send many requests rapidly to increase chance of ID collision
    let mut handles = Vec::new();

    for _i in 0..100 {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            adapter
                .send_request("collision-test", RequestType::Sockets, None, None, None)
                .await
        });
        handles.push(handle);
    }

    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All should succeed
    assert_eq!(responses.len(), 100);

    // All request IDs should be unique
    let mut request_ids = HashSet::new();
    for response in &responses {
        assert!(
            request_ids.insert(response.request_id.clone()),
            "Duplicate request ID found: {}",
            response.request_id
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_channel_operation_edge_cases() -> Result<()> {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    let socket_id = SocketId("edge-case-socket".to_string());

    // Test operations on empty/non-existent channels
    let empty_members = adapter
        .get_channel_members("test-app", "empty-channel")
        .await?;
    assert_eq!(empty_members.len(), 0);

    let empty_sockets = adapter
        .get_channel_sockets("test-app", "empty-channel")
        .await?;
    assert_eq!(empty_sockets.len(), 0);

    let empty_count = adapter
        .get_channel_socket_count("test-app", "empty-channel")
        .await;
    assert_eq!(empty_count, 0);

    // Test socket existence in non-existent channel
    let exists = adapter
        .is_in_channel("test-app", "empty-channel", &socket_id)
        .await?;
    assert!(!exists);

    // Test user operations with non-existent users
    let empty_user_sockets = adapter
        .get_user_sockets("non-existent-user", "test-app")
        .await?;
    assert_eq!(empty_user_sockets.len(), 0);

    let zero_user_count = adapter
        .count_user_connections_in_channel("non-existent-user", "test-app", "any-channel", None)
        .await?;
    assert_eq!(zero_user_count, 0);

    Ok(())
}

#[tokio::test]
async fn test_extreme_value_handling() -> Result<()> {
    // Test with extreme string lengths and values
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Very long app_id
    let long_app_id = "a".repeat(1000);
    let response = adapter
        .send_request(&long_app_id, RequestType::Sockets, None, None, None)
        .await?;
    assert_eq!(response.app_id, long_app_id);

    // Very long channel name
    let long_channel = "channel-".to_string() + &"x".repeat(500);
    let response = adapter
        .send_request(
            "test-app",
            RequestType::ChannelSockets,
            Some(&long_channel),
            None,
            None,
        )
        .await?;
    assert_eq!(response.app_id, "test-app");

    // Empty strings
    let response = adapter
        .send_request("", RequestType::Sockets, Some(""), None, Some(""))
        .await?;
    assert_eq!(response.app_id, "");

    Ok(())
}

#[tokio::test]
async fn test_unicode_and_special_characters() -> Result<()> {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let mut adapter = adapter.with_discovered_nodes(vec!["node-1", "node-2"]).await?;

    // Test with Unicode characters
    let unicode_app = "测试-app-🚀";
    let unicode_channel = "频道-💻";

    let response = adapter
        .send_request(
            unicode_app,
            RequestType::ChannelSockets,
            Some(unicode_channel),
            None,
            None,
        )
        .await?;
    assert_eq!(response.app_id, unicode_app);

    // Test broadcast with Unicode content
    let unicode_message = PusherMessage {
        channel: Some(unicode_channel.to_string()),
        name: None,
        event: Some("测试事件".to_string()),
        data: Some(MessageData::String("Unicode data: 你好世界 🌍".to_string())),
        user_id: None,
    };

    adapter
        .send(unicode_channel, unicode_message, None, unicode_app, None)
        .await?;

    // Verify broadcast was published
    let broadcasts = adapter.transport.get_published_broadcasts().await;
    assert!(!broadcasts.is_empty());
    assert_eq!(broadcasts.last().unwrap().app_id, unicode_app);
    assert_eq!(broadcasts.last().unwrap().channel, unicode_channel);

    Ok(())
}

#[tokio::test]
async fn test_request_timeout_boundary_conditions() -> Result<()> {
    // Test with very short timeout (1ms)
    let node1 = MockNodeState::new("node-1")
        .with_sockets(vec!["socket-1"])
        .with_response_delay(100); // Much longer than timeout

    let short_timeout_config = MockConfig {
        prefix: "test".to_string(),
        request_timeout_ms: 1,
        simulate_failures: false,
        healthy: true,
        node_states: vec![node1],
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(short_timeout_config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter.with_discovered_nodes(vec!["node-1"]).await?;

    let start = std::time::Instant::now();
    let response = adapter
        .send_request("boundary-test", RequestType::Sockets, None, None, None)
        .await?;
    let duration = start.elapsed();

    // Should timeout very quickly
    assert!(duration <= Duration::from_millis(100));
    assert_eq!(response.app_id, "boundary-test");

    // Test with very long timeout (10 seconds) using 2-node config for proper mock operation
    let node1 = MockNodeState::new("node-1")
        .with_sockets(vec!["socket-1"])
        .with_response_delay(50); // Much shorter than timeout

    let node2 = MockNodeState::new("node-2")
        .with_sockets(vec!["socket-2"])
        .with_response_delay(50); // Much shorter than timeout

    let long_timeout_config = MockConfig {
        prefix: "test".to_string(),
        request_timeout_ms: 10000,
        simulate_failures: false,
        healthy: true,
        node_states: vec![node1, node2],
    };

    let adapter2 = HorizontalAdapterBase::<MockTransport>::new(long_timeout_config).await?;
    adapter2.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter2 = adapter2.with_discovered_nodes(vec!["node-1", "node-2"]).await?;

    let start = std::time::Instant::now();
    let response = adapter2
        .send_request("long-timeout-test", RequestType::Sockets, None, None, None)
        .await?;
    let duration = start.elapsed();

    // Should complete quickly (not wait for full timeout)
    assert!(duration <= Duration::from_millis(200));
    assert_eq!(response.app_id, "long-timeout-test");
    assert_eq!(response.sockets_count, 2); // 1 socket from each node: 1+1=2

    Ok(())
}
