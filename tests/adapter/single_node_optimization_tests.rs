use sockudo::adapter::connection_manager::{ConnectionManager, HorizontalAdapterInterface};
use sockudo::adapter::horizontal_adapter::RequestType;
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::error::Result;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use std::time::Duration;
use sonic_rs::json;
use super::horizontal_adapter_helpers::{MockConfig, MockNodeState, MockTransport};

/// Test that single-node deployments skip horizontal communications
#[tokio::test]
async fn test_single_node_skips_broadcast() -> Result<()> {
    // Create config with only 1 node
    let config = MockConfig {
        node_states: vec![MockNodeState::new("single-node")],
        ..Default::default()
    };

    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    // Give adapter time to initialize
    tokio::time::sleep(Duration::from_millis(50)).await;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("test message".to_string())),
        name: None,
        user_id: None,
    };

    // Send a broadcast message
    adapter
        .send("test-channel", message, None, "test-app", None)
        .await?;

    // Verify no broadcasts were published to transport
    let published_broadcasts = adapter.transport.get_published_broadcasts().await;
    assert_eq!(
        published_broadcasts.len(),
        0,
        "Single node should not publish broadcasts"
    );

    Ok(())
}

/// Test that multi-node deployments DO send broadcasts
#[tokio::test]
async fn test_multi_node_sends_broadcast() -> Result<()> {
    // Create config with 2 nodes
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1"), MockNodeState::new("node-2")],
        ..Default::default()
    };

    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.init().await;
    adapter.start_listeners().await?;

    // Simulate other nodes by adding them to heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Give adapter time to initialize
    tokio::time::sleep(Duration::from_millis(50)).await;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("test message".to_string())),
        name: None,
        user_id: None,
    };

    // Send a broadcast message
    adapter
        .send("test-channel", message, None, "test-app", None)
        .await?;

    // Verify broadcasts were published to transport
    let published_broadcasts = adapter.transport.get_published_broadcasts().await;
    assert_eq!(
        published_broadcasts.len(),
        1,
        "Multi-node should publish broadcasts"
    );

    let broadcast = &published_broadcasts[0];
    assert_eq!(broadcast.channel, "test-channel");
    assert_eq!(broadcast.app_id, "test-app");

    Ok(())
}

/// Test that single-node deployments skip horizontal requests
#[tokio::test]
async fn test_single_node_skips_requests() -> Result<()> {
    // Create config with only 1 node
    let config = MockConfig {
        node_states: vec![MockNodeState::new("single-node")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Send a horizontal request
    let response = adapter
        .send_request("test-app", RequestType::Sockets, None, None, None)
        .await?;

    // Verify no requests were published to transport
    let published_requests = adapter.transport.get_published_requests().await;
    assert_eq!(
        published_requests.len(),
        0,
        "Single node should not publish requests"
    );

    // Response should still be valid but empty
    assert_eq!(response.app_id, "test-app");
    assert_eq!(response.sockets_count, 0);
    assert!(response.socket_ids.is_empty());

    Ok(())
}

/// Test that multi-node deployments DO send horizontal requests
#[tokio::test]
async fn test_multi_node_sends_requests() -> Result<()> {
    // Create config with 2 nodes
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1"), MockNodeState::new("node-2")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Simulate other nodes by adding them to heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Send a horizontal request
    let _response = adapter
        .send_request("test-app", RequestType::Sockets, None, None, None)
        .await?;

    // Verify requests were published to transport
    let published_requests = adapter.transport.get_published_requests().await;
    assert!(
        !published_requests.is_empty(),
        "Multi-node should publish at least 1 request, got {}",
        published_requests.len()
    );

    let request = &published_requests[0];
    assert_eq!(request.app_id, "test-app");
    assert!(matches!(request.request_type, RequestType::Sockets));

    Ok(())
}

/// Test that presence broadcasts are skipped in single-node mode
#[tokio::test]
async fn test_single_node_skips_presence_broadcasts() -> Result<()> {
    // Create config with only 1 node
    let config = MockConfig {
        node_states: vec![MockNodeState::new("single-node")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Broadcast presence join
    adapter
        .broadcast_presence_join(
            "test-app",
            "presence-channel",
            "user123",
            "socket123",
            Some(json!({"name": "Alice"})),
        )
        .await?;

    // Broadcast presence leave
    adapter
        .broadcast_presence_leave("test-app", "presence-channel", "user123", "socket123")
        .await?;

    // Verify no presence requests were published to transport
    let published_requests = adapter.transport.get_published_requests().await;
    assert_eq!(
        published_requests.len(),
        0,
        "Single node should not publish presence broadcasts"
    );

    Ok(())
}

/// Test that presence broadcasts are sent in multi-node mode
#[tokio::test]
async fn test_multi_node_sends_presence_broadcasts() -> Result<()> {
    // Create config with 2 nodes
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1"), MockNodeState::new("node-2")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Simulate other nodes by adding them to heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Broadcast presence join
    adapter
        .broadcast_presence_join(
            "test-app",
            "presence-channel",
            "user123",
            "socket123",
            Some(json!({"name": "Alice"})),
        )
        .await?;

    // Broadcast presence leave
    adapter
        .broadcast_presence_leave("test-app", "presence-channel", "user123", "socket123")
        .await?;

    // Verify presence requests were published to transport
    let published_requests = adapter.transport.get_published_requests().await;
    assert_eq!(
        published_requests.len(),
        2,
        "Multi-node should publish presence broadcasts"
    );

    // Check request types
    let join_request = &published_requests[0];
    let leave_request = &published_requests[1];

    assert!(matches!(
        join_request.request_type,
        RequestType::PresenceMemberJoined
    ));
    assert!(matches!(
        leave_request.request_type,
        RequestType::PresenceMemberLeft
    ));

    Ok(())
}

/// Test node count detection accuracy
#[tokio::test]
async fn test_effective_node_count_detection() -> Result<()> {
    // Create config with 1 node
    let config = MockConfig {
        node_states: vec![MockNodeState::new("single-node")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Check initial effective node count (should be 1 - just ourselves)
    let node_count = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.get_effective_node_count().await
    };
    assert_eq!(node_count, 1, "Initial node count should be 1");

    // Simulate another node joining by adding to heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Check updated effective node count (should be 2)
    let node_count = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.get_effective_node_count().await
    };
    assert_eq!(
        node_count, 2,
        "Node count should be 2 after adding heartbeat"
    );

    Ok(())
}

/// Test that the adapter transitions from single-node to multi-node behavior
#[tokio::test]
async fn test_transition_single_to_multi_node() -> Result<()> {
    // Start with single node config
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1")],
        ..Default::default()
    };

    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    let message = PusherMessage {
        channel: Some("test-channel".to_string()),
        event: Some("test-event".to_string()),
        data: Some(MessageData::String("test message".to_string())),
        name: None,
        user_id: None,
    };

    // Send broadcast in single-node mode - should be skipped
    adapter
        .send("test-channel", message.clone(), None, "test-app", None)
        .await?;

    let published_broadcasts = adapter.transport.get_published_broadcasts().await;
    assert_eq!(
        published_broadcasts.len(),
        0,
        "Single node should skip broadcast"
    );

    // Simulate second node joining
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Send broadcast in multi-node mode - should be published
    adapter
        .send("test-channel", message, None, "test-app", None)
        .await?;

    let published_broadcasts = adapter.transport.get_published_broadcasts().await;
    assert_eq!(
        published_broadcasts.len(),
        1,
        "Multi-node should send broadcast"
    );

    Ok(())
}

/// Test the should_skip_horizontal_communication helper method
#[tokio::test]
async fn test_should_skip_horizontal_communication() -> Result<()> {
    // Single node config
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;

    // Should skip with single node
    let should_skip = adapter.should_skip_horizontal_communication().await;
    assert!(should_skip, "Should skip with single node");

    // Add another node
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert("node-2".to_string(), std::time::Instant::now());
    }

    // Should not skip with multiple nodes
    let should_skip = adapter.should_skip_horizontal_communication().await;
    assert!(!should_skip, "Should not skip with multiple nodes");

    Ok(())
}

/// Test that dead node processing is properly handled
#[tokio::test]
async fn test_dead_node_optimization() -> Result<()> {
    // Create config with 1 node initially
    let config = MockConfig {
        node_states: vec![MockNodeState::new("node-1")],
        ..Default::default()
    };

    let adapter = HorizontalAdapterBase::<MockTransport>::new(config.clone()).await?;
    adapter.start_listeners().await?;

    // Add a "dead" node to the heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        // Add an old heartbeat (simulating dead node)
        heartbeats.insert(
            "dead-node".to_string(),
            std::time::Instant::now() - std::time::Duration::from_secs(300),
        );
    }

    // Test get_dead_nodes method
    let dead_nodes = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.get_dead_nodes(5000).await // 5 second timeout
    };

    assert_eq!(dead_nodes.len(), 1, "Should detect 1 dead node");
    assert_eq!(dead_nodes[0], "dead-node");

    Ok(())
}
