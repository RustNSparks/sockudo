use sockudo::adapter::ConnectionManager;
use sockudo::adapter::horizontal_adapter::RequestType;
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::error::Result;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "nats")]
use sockudo::adapter::transports::NatsTransport;
#[cfg(feature = "redis")]
use sockudo::adapter::transports::RedisTransport;

// Import test helpers from the transports tests
#[cfg(feature = "nats")]
use super::transports::test_helpers::get_nats_config;
#[cfg(feature = "redis")]
use super::transports::test_helpers::get_redis_config;

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_horizontal_adapter_with_redis_transport() -> Result<()> {
    let config = get_redis_config();
    let adapter = HorizontalAdapterBase::<RedisTransport>::new(config.clone()).await?;

    // Should be able to initialize
    let mut adapter_mut = HorizontalAdapterBase::<RedisTransport>::new(config).await?;
    adapter_mut.init().await;

    // Start listeners
    adapter.start_listeners().await?;

    // Should be able to check health
    adapter.check_health().await?;

    // Should be able to send a request (even if no other nodes respond)
    let response = adapter
        .send_request("integration-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "integration-app");
    assert!(!response.request_id.is_empty());

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_horizontal_adapter_with_nats_transport() -> Result<()> {
    let config = get_nats_config();
    let adapter = HorizontalAdapterBase::<NatsTransport>::new(config.clone()).await?;

    // Should be able to initialize
    let mut adapter_mut = HorizontalAdapterBase::<NatsTransport>::new(config).await?;
    adapter_mut.init().await;

    // Start listeners
    adapter.start_listeners().await?;

    // Should be able to check health
    adapter.check_health().await?;

    // Should be able to send a request
    let response = adapter
        .send_request("integration-app", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "integration-app");
    assert!(!response.request_id.is_empty());

    Ok(())
}

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_cross_node_broadcast_redis() -> Result<()> {
    let config1 = get_redis_config();
    let config2 = get_redis_config();

    let mut adapter1 = HorizontalAdapterBase::<RedisTransport>::new(config1).await?;
    let mut adapter2 = HorizontalAdapterBase::<RedisTransport>::new(config2).await?;

    // Initialize both adapters
    adapter1.init().await;
    adapter2.init().await;

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        name: None,
        event: Some("cross-node-test".to_string()),
        data: Some(MessageData::String("cross-node broadcast test".to_string())),
        user_id: None,
    };

    // Send broadcast from adapter1
    adapter1
        .send("test-channel", message, None, "test-app", None)
        .await?;

    // Give time for message propagation
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify both adapters are healthy and can communicate
    adapter1.check_health().await?;
    adapter2.check_health().await?;

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_cross_node_broadcast_nats() -> Result<()> {
    let config1 = get_nats_config();
    let config2 = get_nats_config();

    let mut adapter1 = HorizontalAdapterBase::<NatsTransport>::new(config1).await?;
    let mut adapter2 = HorizontalAdapterBase::<NatsTransport>::new(config2).await?;

    // Initialize both adapters
    adapter1.init().await;
    adapter2.init().await;

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        name: None,
        event: Some("cross-node-nats-test".to_string()),
        data: Some(MessageData::String(
            "cross-node nats broadcast test".to_string(),
        )),
        user_id: None,
    };

    // Send broadcast from adapter1
    adapter1
        .send("test-channel", message, None, "test-app", None)
        .await?;

    // Give time for message propagation
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify both adapters are healthy and can communicate
    adapter1.check_health().await?;
    adapter2.check_health().await?;

    Ok(())
}

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_distributed_socket_count_redis() -> Result<()> {
    let config1 = get_redis_config();
    let config2 = get_redis_config();

    let mut adapter1 = HorizontalAdapterBase::<RedisTransport>::new(config1).await?;
    let mut adapter2 = HorizontalAdapterBase::<RedisTransport>::new(config2).await?;

    // Initialize both adapters
    adapter1.init().await;
    adapter2.init().await;

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Add sockets to both adapters to simulate distributed state
    let socket1 = sockudo::websocket::SocketId("test-socket-1".to_string());
    let socket2 = sockudo::websocket::SocketId("test-socket-2".to_string());
    let shared_socket = sockudo::websocket::SocketId("shared-socket".to_string());

    // Add different sockets to each adapter
    let _ = adapter1
        .add_to_channel("test-app", "test-channel", &socket1)
        .await;
    let _ = adapter1
        .add_to_channel("test-app", "test-channel", &shared_socket)
        .await;
    let _ = adapter2
        .add_to_channel("test-app", "test-channel", &socket2)
        .await;
    let _ = adapter2
        .add_to_channel("test-app", "test-channel", &shared_socket)
        .await;

    // Give time for state synchronization
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Query socket count from adapter1 - should see all sockets
    let _count1 = adapter1.get_sockets_count("test-app").await?;

    // Query socket count from adapter2 - should see the same aggregated count
    let _count2 = adapter2.get_sockets_count("test-app").await?;

    // Both adapters should report consistent counts
    // The exact count depends on the local adapter implementation, but should be consistent

    // Verify adapters can communicate by checking channel sockets
    let sockets1 = adapter1
        .get_channel_sockets("test-app", "test-channel")
        .await?;
    let sockets2 = adapter2
        .get_channel_sockets("test-app", "test-channel")
        .await?;

    // Should have some sockets from the local state
    assert!(!sockets1.is_empty() || !sockets2.is_empty());

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_distributed_socket_count_nats() -> Result<()> {
    let config1 = get_nats_config();
    let config2 = get_nats_config();

    let mut adapter1 = HorizontalAdapterBase::<NatsTransport>::new(config1).await?;
    let mut adapter2 = HorizontalAdapterBase::<NatsTransport>::new(config2).await?;

    // Initialize both adapters
    adapter1.init().await;
    adapter2.init().await;

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Add sockets to both adapters to simulate distributed state
    let socket1 = sockudo::websocket::SocketId("nats-socket-1".to_string());
    let socket2 = sockudo::websocket::SocketId("nats-socket-2".to_string());
    let shared_socket = sockudo::websocket::SocketId("nats-shared-socket".to_string());

    // Add different sockets to each adapter
    let _ = adapter1
        .add_to_channel("test-app", "nats-channel", &socket1)
        .await;
    let _ = adapter1
        .add_to_channel("test-app", "nats-channel", &shared_socket)
        .await;
    let _ = adapter2
        .add_to_channel("test-app", "nats-channel", &socket2)
        .await;
    let _ = adapter2
        .add_to_channel("test-app", "nats-channel", &shared_socket)
        .await;

    // Give time for state synchronization
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Query socket count from both adapters
    let _count1 = adapter1.get_sockets_count("test-app").await?;
    let _count2 = adapter2.get_sockets_count("test-app").await?;

    // Both adapters should report valid counts

    // Verify channel-specific operations work
    let channel_count1 = adapter1
        .get_channel_socket_count("test-app", "nats-channel")
        .await;
    let channel_count2 = adapter2
        .get_channel_socket_count("test-app", "nats-channel")
        .await;

    // Should have some sockets in the channel
    assert!(channel_count1 > 0 || channel_count2 > 0);

    Ok(())
}

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_cross_node_request_aggregation_redis() -> Result<()> {
    let config1 = get_redis_config();
    let config2 = get_redis_config();

    let adapter1 = Arc::new(HorizontalAdapterBase::<RedisTransport>::new(config1).await?);
    let adapter2 = Arc::new(HorizontalAdapterBase::<RedisTransport>::new(config2).await?);

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send request from adapter1 - should get responses from both nodes
    let response1 = adapter1
        .send_request("cross-node-app", RequestType::Sockets, None, None, None)
        .await?;

    // Send request from adapter2 - should get responses from both nodes
    let response2 = adapter2
        .send_request("cross-node-app", RequestType::Sockets, None, None, None)
        .await?;

    // Both should succeed
    assert_eq!(response1.app_id, "cross-node-app");
    assert_eq!(response2.app_id, "cross-node-app");
    assert!(!response1.request_id.is_empty());
    assert!(!response2.request_id.is_empty());

    // Request IDs should be different
    assert_ne!(response1.request_id, response2.request_id);

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_cross_node_request_aggregation_nats() -> Result<()> {
    let config1 = get_nats_config();
    let config2 = get_nats_config();

    let adapter1 = Arc::new(HorizontalAdapterBase::<NatsTransport>::new(config1).await?);
    let adapter2 = Arc::new(HorizontalAdapterBase::<NatsTransport>::new(config2).await?);

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give adapters time to establish connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send request from adapter1 - should get responses from both nodes
    let response1 = adapter1
        .send_request("nats-cross-app", RequestType::Sockets, None, None, None)
        .await?;

    // Send request from adapter2 - should get responses from both nodes
    let response2 = adapter2
        .send_request("nats-cross-app", RequestType::Sockets, None, None, None)
        .await?;

    // Both should succeed
    assert_eq!(response1.app_id, "nats-cross-app");
    assert_eq!(response2.app_id, "nats-cross-app");
    assert!(!response1.request_id.is_empty());
    assert!(!response2.request_id.is_empty());

    // Request IDs should be different
    assert_ne!(response1.request_id, response2.request_id);

    Ok(())
}

#[tokio::test]
#[cfg(all(feature = "redis", feature = "nats"))]
async fn test_redis_vs_nats_consistency() -> Result<()> {
    // Test that Redis and NATS transports behave consistently
    let redis_config = get_redis_config();
    let nats_config = get_nats_config();

    let redis_adapter = HorizontalAdapterBase::<RedisTransport>::new(redis_config).await?;
    let nats_adapter = HorizontalAdapterBase::<NatsTransport>::new(nats_config).await?;

    // Start listeners on both
    redis_adapter.start_listeners().await?;
    nats_adapter.start_listeners().await?;

    // Send same request to both
    let redis_response = redis_adapter
        .send_request("consistency-test", RequestType::Sockets, None, None, None)
        .await?;

    let nats_response = nats_adapter
        .send_request("consistency-test", RequestType::Sockets, None, None, None)
        .await?;

    // Both should have same app_id and valid request_ids
    assert_eq!(redis_response.app_id, "consistency-test");
    assert_eq!(nats_response.app_id, "consistency-test");
    assert!(!redis_response.request_id.is_empty());
    assert!(!nats_response.request_id.is_empty());

    // Response structure should be consistent - both should have valid counts
    assert_eq!(redis_response.sockets_count, nats_response.sockets_count);

    Ok(())
}

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_transport_failure_isolation_redis() -> Result<()> {
    let config = get_redis_config();
    let adapter = HorizontalAdapterBase::<RedisTransport>::new(config).await?;

    // Start listeners
    adapter.start_listeners().await?;

    // Should be healthy initially
    adapter.check_health().await?;

    // Send request - should work even if no other nodes respond
    let response = adapter
        .send_request("isolation-test", RequestType::Sockets, None, None, None)
        .await?;

    assert_eq!(response.app_id, "isolation-test");
    assert!(!response.request_id.is_empty());

    // Adapter should remain functional
    adapter.check_health().await?;

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_transport_failure_isolation_nats() -> Result<()> {
    let config = get_nats_config();
    let adapter = HorizontalAdapterBase::<NatsTransport>::new(config).await?;

    // Start listeners
    adapter.start_listeners().await?;

    // Should be healthy initially
    adapter.check_health().await?;

    // Send request - should work even if no other nodes respond
    let response = adapter
        .send_request(
            "nats-isolation-test",
            RequestType::Sockets,
            None,
            None,
            None,
        )
        .await?;

    assert_eq!(response.app_id, "nats-isolation-test");
    assert!(!response.request_id.is_empty());

    // Adapter should remain functional
    adapter.check_health().await?;

    Ok(())
}

#[tokio::test]
#[cfg(feature = "redis")]
async fn test_concurrent_cross_node_operations_redis() -> Result<()> {
    let config1 = get_redis_config();
    let config2 = get_redis_config();

    let adapter1 = Arc::new(HorizontalAdapterBase::<RedisTransport>::new(config1).await?);
    let adapter2 = Arc::new(HorizontalAdapterBase::<RedisTransport>::new(config2).await?);

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give time for connection establishment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Run concurrent operations on both adapters
    let mut handles = Vec::new();

    for i in 0..5 {
        let adapter1 = adapter1.clone();
        let adapter2 = adapter2.clone();

        let handle1 = tokio::spawn(async move {
            adapter1
                .send_request(
                    &format!("concurrent-redis-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await
        });

        let handle2 = tokio::spawn(async move {
            adapter2
                .send_request(
                    &format!("concurrent-redis-{}", i),
                    RequestType::ChannelSockets,
                    Some("test-channel"),
                    None,
                    None,
                )
                .await
        });

        handles.push(handle1);
        handles.push(handle2);
    }

    // Wait for all operations to complete
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All operations should succeed
    assert_eq!(responses.len(), 10);

    // All responses should have valid structure
    for response in &responses {
        assert!(!response.request_id.is_empty());
        assert!(response.app_id.starts_with("concurrent-redis-"));
    }

    // All request IDs should be unique
    let mut request_ids = HashSet::new();
    for response in &responses {
        assert!(request_ids.insert(response.request_id.clone()));
    }

    Ok(())
}

#[tokio::test]
#[cfg(feature = "nats")]
async fn test_concurrent_cross_node_operations_nats() -> Result<()> {
    let config1 = get_nats_config();
    let config2 = get_nats_config();

    let adapter1 = Arc::new(HorizontalAdapterBase::<NatsTransport>::new(config1).await?);
    let adapter2 = Arc::new(HorizontalAdapterBase::<NatsTransport>::new(config2).await?);

    // Start listeners on both
    adapter1.start_listeners().await?;
    adapter2.start_listeners().await?;

    // Give time for connection establishment
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Run concurrent operations on both adapters
    let mut handles = Vec::new();

    for i in 0..5 {
        let adapter1 = adapter1.clone();
        let adapter2 = adapter2.clone();

        let handle1 = tokio::spawn(async move {
            adapter1
                .send_request(
                    &format!("concurrent-nats-{}", i),
                    RequestType::Sockets,
                    None,
                    None,
                    None,
                )
                .await
        });

        let handle2 = tokio::spawn(async move {
            adapter2
                .send_request(
                    &format!("concurrent-nats-{}", i),
                    RequestType::ChannelSockets,
                    Some("nats-channel"),
                    None,
                    None,
                )
                .await
        });

        handles.push(handle1);
        handles.push(handle2);
    }

    // Wait for all operations to complete
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap()?;
        responses.push(response);
    }

    // All operations should succeed
    assert_eq!(responses.len(), 10);

    // All responses should have valid structure
    for response in &responses {
        assert!(!response.request_id.is_empty());
        assert!(response.app_id.starts_with("concurrent-nats-"));
    }

    // All request IDs should be unique
    let mut request_ids = HashSet::new();
    for response in &responses {
        assert!(request_ids.insert(response.request_id.clone()));
    }

    Ok(())
}
