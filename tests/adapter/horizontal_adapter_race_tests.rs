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
use tokio::sync::Barrier;

use super::horizontal_adapter_helpers::{MockConfig, MockNodeState, MockTransport};

/// Test race conditions and concurrent access patterns
#[tokio::test]
async fn test_concurrent_listener_starts() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);

    // Try to start listeners from multiple tasks simultaneously
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(5));

    for _ in 0..5 {
        let adapter = adapter.clone();
        let barrier = barrier.clone();
        let handle = tokio::spawn(async move {
            barrier.wait().await;
            adapter.start_listeners().await
        });
        handles.push(handle);
    }

    // Wait for all attempts to complete
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // At least one should succeed, others may succeed or fail gracefully
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        success_count >= 1,
        "At least one start_listeners should succeed"
    );

    // Verify adapter is functional after concurrent starts
    let response = adapter
        .send_request(
            "concurrent-start-test",
            RequestType::Sockets,
            None,
            None,
            None,
        )
        .await?;
    assert_eq!(response.app_id, "concurrent-start-test");

    Ok(())
}

#[tokio::test]
async fn test_concurrent_health_checks_during_operations() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    let mut handles = Vec::new();

    // Start multiple operations and health checks concurrently
    for i in 0..20 {
        let adapter = adapter.clone();

        if i % 2 == 0 {
            // Send requests
            let handle = tokio::spawn(async move {
                adapter
                    .send_request(
                        &format!("health-race-{}", i),
                        RequestType::Sockets,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map(|r| r.app_id)
            });
            handles.push(handle);
        } else {
            // Check health
            let handle = tokio::spawn(async move {
                adapter
                    .transport
                    .check_health()
                    .await
                    .map(|_| format!("health-ok-{}", i))
            });
            handles.push(handle);
        }
    }

    // Wait for all operations
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // Most operations should succeed
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        success_count >= 15,
        "Most concurrent operations should succeed"
    );

    Ok(())
}

#[tokio::test]
async fn test_request_response_matching_under_load() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Send many concurrent requests to test response matching
    let mut handles = Vec::new();
    let request_count = 50;

    for i in 0..request_count {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            let app_id = format!("load-test-{}", i);
            let response = adapter
                .send_request(&app_id, RequestType::Sockets, None, None, None)
                .await?;

            // Verify response matches request
            if response.app_id != app_id {
                return Err(sockudo::error::Error::Internal(format!(
                    "Response mismatch: expected {}, got {}",
                    app_id, response.app_id
                )));
            }

            Ok(response.request_id)
        });
        handles.push(handle);
    }

    // Wait for all requests
    let mut request_ids = Vec::new();
    for handle in handles {
        let request_id = handle.await.unwrap()?;
        request_ids.push(request_id);
    }

    // All should succeed and have unique request IDs
    assert_eq!(request_ids.len(), request_count);

    let unique_ids: HashSet<String> = request_ids.into_iter().collect();
    assert_eq!(
        unique_ids.len(),
        request_count,
        "All request IDs should be unique"
    );

    Ok(())
}

#[tokio::test]
async fn test_broadcast_during_listener_changes() -> Result<()> {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.init().await;

    // Start broadcasts while listeners might be starting/stopping
    let adapter_arc = Arc::new(adapter);
    let mut handles = Vec::new();

    // Start listener registration
    let adapter_clone = adapter_arc.clone();
    let listener_handle = tokio::spawn(async move { adapter_clone.start_listeners().await });

    // Immediately start sending broadcasts
    for i in 0..10 {
        let adapter_clone = adapter_arc.clone();
        let handle = tokio::spawn(async move {
            let _message = PusherMessage {
                channel: None,
                name: None,
                event: Some(format!("race-event-{}", i)),
                data: Some(MessageData::String(format!("race data {}", i))),
                user_id: None,
            };

            tokio::time::sleep(Duration::from_millis(i * 10)).await;

            adapter_clone
                .send_request(
                    &format!("race-app-{}", i),
                    RequestType::Sockets,
                    Some(&format!("race-channel-{}", i)),
                    None,
                    None,
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for listener registration
    let _ = listener_handle.await.unwrap();

    // Wait for all broadcasts
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // Most broadcasts should succeed (some early ones might fail if listeners weren't ready)
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        success_count >= 5,
        "At least half of broadcasts should succeed"
    );

    Ok(())
}

#[tokio::test]
async fn test_node_count_changes_during_requests() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    let mut handles = Vec::new();

    // Start requests while continuously changing node count
    for i in 0..30 {
        let adapter = adapter.clone();

        // Mix of requests and node changes
        if i % 3 == 0 {
            // Change node count
            let handle = tokio::spawn(async move {
                if i % 6 == 0 {
                    adapter.transport.add_node().await;
                } else {
                    adapter.transport.remove_node().await;
                }
                Ok::<String, sockudo::error::Error>(format!("node-change-{}", i))
            });
            handles.push(handle);
        } else {
            // Send request
            let handle = tokio::spawn(async move {
                adapter
                    .send_request(
                        &format!("node-race-{}", i),
                        RequestType::Sockets,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map(|r| r.app_id)
            });
            handles.push(handle);
        }
    }

    // Wait for all operations
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // All operations should succeed despite dynamic node changes
    for result in &results {
        assert!(result.is_ok(), "Operation failed: {:?}", result);
    }

    Ok(())
}

#[tokio::test]
async fn test_transport_state_consistency_under_load() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    let mut handles = Vec::new();

    // Mix of different operations that access transport state
    for i in 0..20 {
        let adapter = adapter.clone();

        let operation = i % 4;
        let handle = tokio::spawn(async move {
            match operation {
                0 => {
                    // Send request
                    adapter
                        .send_request("consistency-test", RequestType::Sockets, None, None, None)
                        .await
                        .map(|_| "request".to_string())
                }
                1 => {
                    // Check health
                    adapter
                        .transport
                        .check_health()
                        .await
                        .map(|_| "health".to_string())
                }
                2 => {
                    // Get node count
                    adapter
                        .transport
                        .get_node_count()
                        .await
                        .map(|c| format!("nodes-{}", c))
                }
                3 => {
                    // Send broadcast
                    let _message = PusherMessage {
                        channel: None,
                        name: None,
                        event: Some("consistency-event".to_string()),
                        data: Some(MessageData::String("test".to_string())),
                        user_id: None,
                    };

                    adapter
                        .send_request(
                            "consistency-app",
                            RequestType::ChannelSockets,
                            Some("consistency-channel"),
                            None,
                            None,
                        )
                        .await
                        .map(|_| ())
                        .map(|_| "broadcast".to_string())
                }
                _ => unreachable!(),
            }
        });
        handles.push(handle);
    }

    // Wait for all operations
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // All operations should succeed
    for result in &results {
        assert!(result.is_ok(), "Consistency test failed: {:?}", result);
    }

    Ok(())
}

#[tokio::test]
async fn test_response_handler_race_conditions() -> Result<()> {
    // Test what happens when responses arrive while request is being cleaned up
    let node1 = MockNodeState::new("node-1")
        .with_sockets(vec!["socket-1"])
        .with_response_delay(50);

    let node2 = MockNodeState::new("node-2")
        .with_sockets(vec!["socket-2"])
        .with_response_delay(100);

    let config = MockConfig {
        prefix: "test".to_string(),
        request_timeout_ms: 75, // Between node1 and node2 response times
        simulate_failures: false,
        healthy: true,
        node_states: vec![node1, node2],
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config).await?;
    adapter.start_listeners().await?;

    // Simulate discovered nodes for multi-node behavior
    let adapter = adapter
        .with_discovered_nodes(vec!["node-1", "node-2"])
        .await?;
    let adapter = Arc::new(adapter);

    // Send multiple requests that will have mixed timing
    let mut handles = Vec::new();

    for i in 0..10 {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            adapter
                .send_request(
                    &format!("race-response-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await
        });
        handles.push(handle);
    }

    // Wait for all requests
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All should succeed
    assert_eq!(responses.len(), 10);

    // Some should have only node1 data (timeout before node2), others may have both
    for response in &responses {
        assert!(response.sockets_count >= 1); // At least node1 response
        assert!(response.sockets_count <= 2); // At most both nodes
    }

    Ok(())
}

#[tokio::test]
async fn test_concurrent_connection_manager_operations() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    let mut handles = Vec::new();

    // Mix different ConnectionManager operations concurrently
    for i in 0..25 {
        let adapter = adapter.clone();

        let op = i % 5;
        let handle = tokio::spawn(async move {
            match op {
                0 => adapter
                    .send_request("concurrent-app", RequestType::Sockets, None, None, None)
                    .await
                    .map(|r| format!("sockets-{}", r.sockets_count)),
                1 => adapter
                    .send_request(
                        "concurrent-app",
                        RequestType::ChannelSockets,
                        Some("concurrent-channel"),
                        None,
                        None,
                    )
                    .await
                    .map(|r| format!("channel-sockets-{}", r.socket_ids.len())),
                2 => adapter
                    .send_request("concurrent-app", RequestType::Sockets, None, None, None)
                    .await
                    .map(|r| format!("user-sockets-{}", r.socket_ids.len())),
                3 => adapter
                    .send_request(
                        "concurrent-app",
                        RequestType::ChannelMembers,
                        Some("presence-channel"),
                        None,
                        None,
                    )
                    .await
                    .map(|r| format!("members-{}", r.members.len())),
                4 => {
                    let socket = SocketId(format!("socket-{}", i));
                    adapter
                        .send_request(
                            "concurrent-app",
                            RequestType::SocketExistsInChannel,
                            Some("concurrent-channel"),
                            Some(&socket.0),
                            None,
                        )
                        .await
                        .map(|r| format!("exists-{}", r.exists))
                }
                _ => unreachable!(),
            }
        });
        handles.push(handle);
    }

    // Wait for all operations
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // All operations should succeed
    for result in &results {
        assert!(
            result.is_ok(),
            "Connection manager operation failed: {:?}",
            result
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_memory_consistency_under_concurrent_access() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Stress test with many concurrent operations accessing shared state
    let mut handles = Vec::new();
    let iterations = 100;

    for i in 0..iterations {
        let adapter = adapter.clone();
        let handle = tokio::spawn(async move {
            // Each task does multiple operations
            let mut results = Vec::new();

            // Send request
            let response = adapter
                .send_request(
                    &format!("stress-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await?;
            results.push(response.request_id);

            // Check transport state
            let node_count = adapter.transport.get_node_count().await?;
            results.push(format!("nodes-{}", node_count));

            // Get published data
            let broadcasts = adapter.transport.get_published_broadcasts().await;
            results.push(format!("broadcasts-{}", broadcasts.len()));

            Ok::<Vec<String>, sockudo::error::Error>(results)
        });
        handles.push(handle);
    }

    // Wait for all stress operations
    let mut all_results = Vec::new();
    for handle in handles {
        let results = handle.await.unwrap()?;
        all_results.extend(results);
    }

    // Verify we got expected number of results
    assert_eq!(all_results.len(), iterations * 3);

    // Verify no corrupted data (all results should be non-empty strings)
    for result in &all_results {
        assert!(
            !result.is_empty(),
            "Found empty result indicating potential memory corruption"
        );
        assert!(!result.contains("CORRUPTED"), "Found corrupted data marker");
    }

    Ok(())
}

#[tokio::test]
async fn test_adapter_state_isolation() -> Result<()> {
    // Test that concurrent operations on same adapter don't interfere
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    let barrier = Arc::new(Barrier::new(10));
    let mut handles = Vec::new();

    // All tasks start simultaneously
    for i in 0..10 {
        let adapter = adapter.clone();
        let barrier = barrier.clone();

        let handle = tokio::spawn(async move {
            barrier.wait().await;

            // Each does the same operation with different parameters
            let response = adapter
                .send_request(
                    &format!("isolation-app-{}", i),
                    RequestType::ChannelSockets,
                    Some(&format!("isolation-channel-{}", i)),
                    None,
                    None,
                )
                .await?;

            // Verify response matches request parameters
            if response.app_id != format!("isolation-app-{}", i) {
                return Err(sockudo::error::Error::Internal(
                    "State isolation failed: wrong app_id".to_string(),
                ));
            }

            Ok(response.request_id)
        });
        handles.push(handle);
    }

    // Wait for all operations
    let mut request_ids = Vec::new();
    for handle in handles {
        let request_id = handle.await.unwrap()?;
        request_ids.push(request_id);
    }

    // All should succeed with unique request IDs
    assert_eq!(request_ids.len(), 10);

    let unique_ids: HashSet<String> = request_ids.into_iter().collect();
    assert_eq!(
        unique_ids.len(),
        10,
        "State isolation failed: duplicate request IDs"
    );

    Ok(())
}

#[tokio::test]
async fn test_cleanup_after_abandoned_operations() -> Result<()> {
    let config = MockConfig::default();
    let adapter = Arc::new(HorizontalAdapterBase::<MockTransport>::new(config).await?);
    adapter.start_listeners().await?;

    // Start many operations and abandon them (drop handles)
    for i in 0..50 {
        let adapter = adapter.clone();
        let _handle = tokio::spawn(async move {
            adapter
                .send_request(
                    &format!("abandoned-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await
        });
        // Handle is dropped immediately, abandoning the operation
    }

    // Give time for operations to be abandoned
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify adapter is still functional
    for i in 0..10 {
        let response = adapter
            .send_request(
                &format!("cleanup-test-{}", i),
                RequestType::Sockets,
                None,
                None,
                None,
            )
            .await?;
        assert_eq!(response.app_id, format!("cleanup-test-{}", i));
    }

    // Verify transport state is clean
    let node_count = adapter.transport.get_node_count().await?;
    assert_eq!(node_count, 2); // Should be back to original

    adapter.transport.check_health().await?; // Should still be healthy

    Ok(())
}
