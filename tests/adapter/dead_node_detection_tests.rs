use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use sockudo::adapter::ConnectionManager;
use sockudo::adapter::horizontal_adapter::{DeadNodeEvent, RequestType};
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::options::ClusterHealthConfig;
use std::time::Duration;

#[tokio::test]
async fn test_heartbeat_tracking_updates_node_registry() {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Configure cluster health
    let cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 1000,
        cleanup_interval_ms: 500,
    };
    adapter.set_cluster_health(&cluster_config).await.unwrap();
    adapter.start_listeners().await.unwrap();

    // Simulate receiving a heartbeat from another node
    let heartbeat_request = sockudo::adapter::horizontal_adapter::RequestBody {
        request_id: "test-heartbeat-1".to_string(),
        node_id: "remote-node-1".to_string(),
        app_id: "cluster".to_string(),
        request_type: RequestType::Heartbeat,
        channel: None,
        socket_id: None,
        user_id: None,
        user_info: None,
        timestamp: Some(1234567890),
        dead_node_id: None,
        target_node_id: None,
    };

    // Process the heartbeat through the adapter's request handler
    {
        let mut horizontal = adapter.horizontal.lock().await;
        let result = horizontal.process_request(heartbeat_request.clone()).await;
        assert!(result.is_ok(), "Heartbeat processing should succeed");
    }

    // Verify the node is now tracked in heartbeat registry
    {
        let horizontal = adapter.horizontal.lock().await;
        let heartbeats = horizontal.node_heartbeats.read().await;
        assert!(
            heartbeats.contains_key("remote-node-1"),
            "Remote node should be tracked after heartbeat"
        );
        assert!(
            heartbeats.len() == 1,
            "Should have exactly 1 remote node tracked"
        );
    }
}

#[tokio::test]
async fn test_dead_node_detection_identifies_timeout_nodes() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let node_timeout_ms = 500; // 500ms timeout for testing

    // Manually add a node to heartbeat tracking with an old timestamp
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;

        // Add node with timestamp that will be considered "dead"
        let old_time = std::time::Instant::now() - Duration::from_millis(node_timeout_ms + 100);
        heartbeats.insert("old-node".to_string(), old_time);

        // Add a "recent" node that should still be alive
        let recent_time = std::time::Instant::now() - Duration::from_millis(100);
        heartbeats.insert("recent-node".to_string(), recent_time);
    }

    // Use the adapter's dead node detection logic
    let dead_nodes = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.get_dead_nodes(node_timeout_ms).await
    };

    // Verify that only the old node is identified as dead
    assert_eq!(dead_nodes.len(), 1, "Should identify exactly 1 dead node");
    assert_eq!(
        dead_nodes[0], "old-node",
        "Should identify the old node as dead"
    );
}

#[tokio::test]
async fn test_cleanup_leader_election_logic() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Get the actual node_id that was generated
    let our_node_id = adapter.node_id.clone();

    // Set up a scenario where our node should be leader (alphabetically first)
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;

        // Add nodes that are alphabetically after our node
        let after_node_1 = format!("zzz-after-{}", our_node_id);
        let after_node_2 = format!("zzz-also-after-{}", our_node_id);

        heartbeats.insert(after_node_1, std::time::Instant::now());
        heartbeats.insert(after_node_2, std::time::Instant::now());
    }

    let dead_nodes = vec!["some-dead-node".to_string()];

    // Test leader election logic
    let is_leader = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.is_cleanup_leader(&dead_nodes).await
    };

    // Since our node ID is alphabetically first among alive nodes, we should be elected leader
    assert!(is_leader, "This node should be elected as cleanup leader");
}

#[tokio::test]
async fn test_cleanup_leader_election_excludes_dead_nodes() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Get the actual node_id that was generated
    let our_node_id = adapter.node_id.clone();

    // Set up scenario where we control the alphabetical ordering
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;

        // Add nodes that are alphabetically before and after our node
        // We'll mark the "before" node as dead, so we should become leader
        let before_node = format!("aaa-before-{}", our_node_id);
        let after_node = format!("zzz-after-{}", our_node_id);

        heartbeats.insert(before_node.clone(), std::time::Instant::now());
        heartbeats.insert(after_node, std::time::Instant::now());
    }

    // Mark the alphabetically first node as dead
    let before_node = format!("aaa-before-{}", our_node_id);
    let dead_nodes = vec![before_node];

    // Test leader election with dead node excluded
    let is_leader = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.is_cleanup_leader(&dead_nodes).await
    };

    // Since the node that would normally be leader (alphabetically first) is dead,
    // our node should become leader (it's now the alphabetically first among alive nodes)
    assert!(
        is_leader,
        "This node should become leader when the natural leader is dead"
    );
}

#[tokio::test]
async fn test_dead_node_cleanup_removes_presence_data() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Set up presence data for a node we'll mark as dead
    let dead_node_id = "doomed-node";
    {
        let horizontal = adapter.horizontal.lock().await;

        // Add presence data for the doomed node
        horizontal
            .add_presence_entry(
                dead_node_id,
                "test-channel",
                "socket-123",
                "user-456",
                "test-app",
                Some(sonic_rs::json!({"name": "Doomed User"})),
            )
            .await;

        horizontal
            .add_presence_entry(
                dead_node_id,
                "another-channel",
                "socket-789",
                "user-999",
                "test-app",
                Some(sonic_rs::json!({"name": "Another Doomed User"})),
            )
            .await;
    }

    // Verify presence data exists before cleanup
    {
        let horizontal = adapter.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;
        assert!(
            registry.contains_key(dead_node_id),
            "Dead node should have presence data before cleanup"
        );
        assert_eq!(
            registry[dead_node_id].len(),
            2,
            "Dead node should have 2 channels of presence data"
        );
    }

    // Perform dead node cleanup
    let cleanup_tasks = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal
            .handle_dead_node_cleanup(dead_node_id)
            .await
            .unwrap()
    };

    // Verify cleanup tasks were generated
    assert_eq!(
        cleanup_tasks.len(),
        2,
        "Should generate 2 cleanup tasks (one per unique user)"
    );

    // Verify presence data was removed
    {
        let horizontal = adapter.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;
        assert!(
            !registry.contains_key(dead_node_id),
            "Dead node should have no presence data after cleanup"
        );
    }

    // Verify cleanup task contents
    for task in &cleanup_tasks {
        let (app_id, channel, user_id, user_info) = task;
        assert_eq!(app_id, "test-app");
        assert!(channel == "test-channel" || channel == "another-channel");
        assert!(user_id == "user-456" || user_id == "user-999");
        assert!(user_info.is_some());
    }
}

#[tokio::test]
async fn test_dead_node_event_structure_contains_required_data() {
    let config = MockConfig::default();
    let mut adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Set up event receiver
    let mut event_receiver = adapter
        .configure_dead_node_events()
        .expect("Should be able to configure dead node events");

    // Add some presence data that will become orphaned
    let dead_node_id = "failing-node";
    {
        let horizontal = adapter.horizontal.lock().await;
        horizontal
            .add_presence_entry(
                dead_node_id,
                "presence-channel",
                "socket-abc",
                "user-def",
                "my-app",
                Some(sonic_rs::json!({"display_name": "Test User", "status": "online"})),
            )
            .await;
    }

    // Simulate the dead node cleanup process that emits events
    let cleanup_tasks = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal
            .handle_dead_node_cleanup(dead_node_id)
            .await
            .unwrap()
    };

    // Manually emit the event (simulating what the cleanup system does)
    if !cleanup_tasks.is_empty() {
        let orphaned_members = cleanup_tasks
            .into_iter()
            .map(|(app_id, channel, user_id, user_info)| {
                sockudo::adapter::horizontal_adapter::OrphanedMember {
                    app_id,
                    channel,
                    user_id,
                    user_info,
                }
            })
            .collect();

        let event = DeadNodeEvent {
            dead_node_id: dead_node_id.to_string(),
            orphaned_members,
        };

        // Send through the adapter's event bus
        if let Some(event_bus) = &adapter.event_bus {
            event_bus.send(event).unwrap();
        }
    }

    // Verify event was received with correct structure
    let received_event = tokio::time::timeout(Duration::from_millis(100), event_receiver.recv())
        .await
        .expect("Should receive event within timeout")
        .expect("Event should be received successfully");

    assert_eq!(received_event.dead_node_id, dead_node_id);
    assert_eq!(received_event.orphaned_members.len(), 1);

    let orphaned_member = &received_event.orphaned_members[0];
    assert_eq!(orphaned_member.app_id, "my-app");
    assert_eq!(orphaned_member.channel, "presence-channel");
    assert_eq!(orphaned_member.user_id, "user-def");
    assert!(orphaned_member.user_info.is_some());

    let user_info = orphaned_member.user_info.as_ref().unwrap();
    assert_eq!(user_info["display_name"], "Test User");
    assert_eq!(user_info["status"], "online");
}

#[tokio::test]
async fn test_follower_cleanup_only_removes_registry_data() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let dead_node_id = "failing-node";

    // Set up presence data for a dead node
    {
        let horizontal = adapter.horizontal.lock().await;
        horizontal
            .add_presence_entry(
                dead_node_id,
                "test-channel",
                "socket-1",
                "user-1",
                "test-app",
                Some(sonic_rs::json!({"name": "User 1"})),
            )
            .await;

        // Also add heartbeat tracking for this node
        let mut heartbeats = horizontal.node_heartbeats.write().await;
        heartbeats.insert(dead_node_id.to_string(), std::time::Instant::now());
    }

    // Simulate receiving a "NodeDead" notification as a follower
    let dead_node_request = sockudo::adapter::horizontal_adapter::RequestBody {
        request_id: "cleanup-notification".to_string(),
        node_id: "leader-node".to_string(),
        app_id: "cluster".to_string(),
        request_type: RequestType::NodeDead,
        channel: None,
        socket_id: None,
        user_id: None,
        user_info: None,
        timestamp: Some(1234567890),
        dead_node_id: Some(dead_node_id.to_string()),
        target_node_id: None,
    };

    // Process the dead node notification
    {
        let mut horizontal = adapter.horizontal.lock().await;
        let result = horizontal.process_request(dead_node_request).await;
        assert!(
            result.is_ok(),
            "Dead node notification processing should succeed"
        );
    }

    // Verify both heartbeat and presence registry were cleaned up
    {
        let horizontal = adapter.horizontal.lock().await;

        // Check heartbeat tracking
        let heartbeats = horizontal.node_heartbeats.read().await;
        assert!(
            !heartbeats.contains_key(dead_node_id),
            "Dead node should be removed from heartbeat tracking"
        );

        // Check presence registry
        let registry = horizontal.cluster_presence_registry.read().await;
        assert!(
            !registry.contains_key(dead_node_id),
            "Dead node should be removed from presence registry"
        );
    }
}
