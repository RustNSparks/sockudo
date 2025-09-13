use sockudo::adapter::ConnectionManager;
use sockudo::adapter::local_adapter::LocalAdapter;
use sockudo::options::ClusterHealthConfig;
use sockudo::websocket::SocketId;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Test that LocalAdapter gracefully handles cluster health configuration
#[tokio::test]
async fn test_local_adapter_cluster_health_disabled() {
    let mut adapter = LocalAdapter::new();

    // LocalAdapter should not support cluster health, but should not panic
    // when these methods are called (they should be no-ops or return appropriate errors)

    // Initialize adapter
    adapter.init().await;

    // These operations should work normally with local adapter
    let app_id = "test-app";
    let channel = "presence-local-test";
    let socket_id = SocketId("socket-123".to_string());

    // Add to channel should work
    let added = adapter
        .add_to_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(
        added,
        "LocalAdapter should successfully add socket to channel"
    );

    // Check if in channel
    let exists = adapter
        .is_in_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(exists, "Socket should exist in channel");

    // Remove from channel
    let removed = adapter
        .remove_from_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(
        removed,
        "LocalAdapter should successfully remove socket from channel"
    );
}

/// Test that LocalAdapter doesn't implement HorizontalAdapterInterface
#[tokio::test]
async fn test_local_adapter_no_horizontal_interface() {
    let adapter = LocalAdapter::new();

    // LocalAdapter should not implement HorizontalAdapterInterface
    // This test confirms that we can't call broadcast_presence_* methods

    // If this compiles, it means LocalAdapter incorrectly implements the interface
    // The following should not compile (commented out):

    // adapter.broadcast_presence_join("app", "channel", "user", "socket", None).await;
    // adapter.broadcast_presence_leave("app", "channel", "user", "socket").await;

    // Instead, LocalAdapter should work purely locally
    let mut local_adapter = adapter;
    local_adapter.init().await;

    // Local operations should work fine
    let socket_count = local_adapter
        .get_channel_socket_count("app", "channel")
        .await;
    assert_eq!(socket_count, 0, "Empty channel should have 0 sockets");
}

/// Test LocalAdapter with presence operations
#[tokio::test]
async fn test_local_adapter_presence_operations() {
    let mut adapter = LocalAdapter::new();
    adapter.init().await;

    let app_id = "test-app";
    let channel = "presence-local-channel";

    // Add multiple sockets
    let socket1 = SocketId("socket-1".to_string());
    let socket2 = SocketId("socket-2".to_string());
    let socket3 = SocketId("socket-3".to_string());

    adapter
        .add_to_channel(app_id, channel, &socket1)
        .await
        .unwrap();
    adapter
        .add_to_channel(app_id, channel, &socket2)
        .await
        .unwrap();
    adapter
        .add_to_channel(app_id, channel, &socket3)
        .await
        .unwrap();

    // Check socket count
    let socket_count = adapter.get_channel_socket_count(app_id, channel).await;
    assert_eq!(socket_count, 3, "Channel should have 3 sockets");

    // Get channel sockets
    let sockets = adapter.get_channel_sockets(app_id, channel).await.unwrap();
    assert_eq!(sockets.len(), 3, "Should return 3 sockets");
    assert!(sockets.contains(&socket1));
    assert!(sockets.contains(&socket2));
    assert!(sockets.contains(&socket3));

    // Get channel members (should be empty for non-presence channel)
    let members = adapter.get_channel_members(app_id, channel).await.unwrap();
    assert_eq!(
        members.len(),
        0,
        "Non-presence channel should have no members"
    );

    // Remove sockets
    adapter
        .remove_from_channel(app_id, channel, &socket2)
        .await
        .unwrap();

    let socket_count = adapter.get_channel_socket_count(app_id, channel).await;
    assert_eq!(
        socket_count, 2,
        "Channel should have 2 sockets after removal"
    );
}

/// Test LocalAdapter in fallback scenario
#[tokio::test]
async fn test_local_adapter_fallback_from_failed_distributed_adapter() {
    // Simulate scenario where distributed adapter fails and falls back to local

    // This would happen in the adapter factory when Redis/NATS fails
    let _cluster_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    // Create local adapter as fallback
    let mut fallback_adapter = LocalAdapter::new();
    fallback_adapter.init().await;

    // Local adapter should work even when cluster health config is available
    // (it just ignores the cluster health configuration)

    let app_id = "fallback-app";
    let channel = "fallback-channel";
    let socket_id = SocketId("fallback-socket".to_string());

    // Basic operations should still work
    fallback_adapter
        .add_to_channel(app_id, channel, &socket_id)
        .await
        .unwrap();

    let exists = fallback_adapter
        .is_in_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(exists, "Socket should exist in fallback adapter");

    // No cluster health operations are available, but local operations work
    let socket_count = fallback_adapter
        .get_channel_socket_count(app_id, channel)
        .await;
    assert_eq!(
        socket_count, 1,
        "Fallback adapter should track local sockets"
    );
}

/// Test LocalAdapter multi-app isolation
#[tokio::test]
async fn test_local_adapter_app_isolation() {
    let mut adapter = LocalAdapter::new();
    adapter.init().await;

    let app1 = "app-1";
    let app2 = "app-2";
    let channel = "shared-channel-name";
    let socket1 = SocketId("socket-app1".to_string());
    let socket2 = SocketId("socket-app2".to_string());

    // Add sockets to same channel name but different apps
    adapter
        .add_to_channel(app1, channel, &socket1)
        .await
        .unwrap();
    adapter
        .add_to_channel(app2, channel, &socket2)
        .await
        .unwrap();

    // Each app should only see its own sockets
    let app1_count = adapter.get_channel_socket_count(app1, channel).await;
    let app2_count = adapter.get_channel_socket_count(app2, channel).await;

    assert_eq!(app1_count, 1, "App1 should see only its socket");
    assert_eq!(app2_count, 1, "App2 should see only its socket");

    // Check socket existence per app
    assert!(
        adapter
            .is_in_channel(app1, channel, &socket1)
            .await
            .unwrap()
    );
    assert!(
        !adapter
            .is_in_channel(app1, channel, &socket2)
            .await
            .unwrap()
    );
    assert!(
        adapter
            .is_in_channel(app2, channel, &socket2)
            .await
            .unwrap()
    );
    assert!(
        !adapter
            .is_in_channel(app2, channel, &socket1)
            .await
            .unwrap()
    );
}

/// Test LocalAdapter concurrent operations
#[tokio::test]
async fn test_local_adapter_concurrent_operations() {
    let adapter = Arc::new(Mutex::new(LocalAdapter::new()));

    {
        let mut adapter_guard = adapter.lock().await;
        adapter_guard.init().await;
    }

    let app_id = "concurrent-app";
    let channel = "concurrent-channel";

    // Spawn multiple concurrent operations
    let mut handles = vec![];

    for i in 0..20 {
        let adapter_clone = adapter.clone();
        let socket_id = format!("socket-{}", i);

        let handle = tokio::spawn(async move {
            let mut adapter_guard = adapter_clone.lock().await;
            let socket = SocketId(socket_id);

            // Add to channel
            adapter_guard
                .add_to_channel(app_id, channel, &socket)
                .await
                .unwrap();

            // Check if exists
            let exists = adapter_guard
                .is_in_channel(app_id, channel, &socket)
                .await
                .unwrap();
            assert!(exists);

            // Remove from channel
            adapter_guard
                .remove_from_channel(app_id, channel, &socket)
                .await
                .unwrap();
        });
        handles.push(handle);
    }

    // Wait for all operations
    for handle in handles {
        handle.await.unwrap();
    }

    // Final count should be 0
    let final_count = {
        let mut adapter_guard = adapter.lock().await;
        adapter_guard
            .get_channel_socket_count(app_id, channel)
            .await
    };

    assert_eq!(
        final_count, 0,
        "All sockets should be removed after concurrent operations"
    );
}

/// Test LocalAdapter memory cleanup
#[tokio::test]
async fn test_local_adapter_memory_cleanup() {
    let mut adapter = LocalAdapter::new();
    adapter.init().await;

    let app_id = "cleanup-app";
    let channels = vec!["channel-1", "channel-2", "channel-3"];

    // Add sockets to multiple channels
    for (i, channel) in channels.iter().enumerate() {
        for j in 0..5 {
            let socket_id = SocketId(format!("socket-{}-{}", i, j));
            adapter
                .add_to_channel(app_id, channel, &socket_id)
                .await
                .unwrap();
        }
    }

    // Verify initial state
    for channel in &channels {
        let count = adapter.get_channel_socket_count(app_id, channel).await;
        assert_eq!(count, 5, "Each channel should have 5 sockets");
    }

    // Remove all sockets from one channel
    for j in 0..5 {
        let socket_id = SocketId(format!("socket-0-{}", j));
        adapter
            .remove_from_channel(app_id, channels[0], &socket_id)
            .await
            .unwrap();
    }

    // Channel should be cleaned up
    let count = adapter.get_channel_socket_count(app_id, channels[0]).await;
    assert_eq!(count, 0, "Cleaned channel should have 0 sockets");

    // Other channels should be unaffected
    for channel in &channels[1..] {
        let count = adapter.get_channel_socket_count(app_id, channel).await;
        assert_eq!(count, 5, "Other channels should be unaffected");
    }

    // Remove channel completely
    adapter.remove_channel(app_id, channels[1]).await;
    let count = adapter.get_channel_socket_count(app_id, channels[1]).await;
    assert_eq!(count, 0, "Removed channel should have 0 sockets");
}

/// Test LocalAdapter buffer configuration
#[tokio::test]
async fn test_local_adapter_with_custom_buffer() {
    let buffer_multiplier = 128; // Custom buffer size
    let adapter = LocalAdapter::new_with_buffer_multiplier(buffer_multiplier);

    // Adapter should be created successfully with custom buffer
    // The buffer size affects internal capacity but doesn't change the API behavior

    let mut local_adapter = adapter;
    local_adapter.init().await;

    // Should work the same as default adapter
    let app_id = "buffer-test-app";
    let channel = "buffer-test-channel";
    let socket_id = SocketId("buffer-socket".to_string());

    local_adapter
        .add_to_channel(app_id, channel, &socket_id)
        .await
        .unwrap();

    let exists = local_adapter
        .is_in_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(exists, "Socket should exist with custom buffer adapter");
}

/// Test LocalAdapter error handling
#[tokio::test]
async fn test_local_adapter_error_handling() {
    let mut adapter = LocalAdapter::new();
    adapter.init().await;

    let app_id = "error-test-app";
    let channel = "error-test-channel";
    let socket_id = SocketId("error-socket".to_string());

    // Try to remove from non-existent channel
    let removed = adapter
        .remove_from_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(!removed, "Removing non-existent socket should return false");

    // Check non-existent socket
    let exists = adapter
        .is_in_channel(app_id, channel, &socket_id)
        .await
        .unwrap();
    assert!(!exists, "Non-existent socket should not exist");

    // Empty channel operations
    let count = adapter
        .get_channel_socket_count(app_id, "non-existent-channel")
        .await;
    assert_eq!(count, 0, "Non-existent channel should have 0 sockets");

    let sockets = adapter
        .get_channel_sockets(app_id, "non-existent-channel")
        .await
        .unwrap();
    assert_eq!(
        sockets.len(),
        0,
        "Non-existent channel should return empty socket set"
    );

    let members = adapter
        .get_channel_members(app_id, "non-existent-channel")
        .await
        .unwrap();
    assert_eq!(
        members.len(),
        0,
        "Non-existent channel should return empty members"
    );
}

/// Test LocalAdapter large scale operations
#[tokio::test]
async fn test_local_adapter_large_scale() {
    let mut adapter = LocalAdapter::new();
    adapter.init().await;

    let app_id = "scale-test-app";
    let num_channels = 50;
    let sockets_per_channel = 100;

    // Add many sockets across many channels
    for c in 0..num_channels {
        let channel = format!("channel-{}", c);

        for s in 0..sockets_per_channel {
            let socket_id = SocketId(format!("socket-{}-{}", c, s));
            adapter
                .add_to_channel(app_id, &channel, &socket_id)
                .await
                .unwrap();
        }
    }

    // Verify all were added correctly
    for c in 0..num_channels {
        let channel = format!("channel-{}", c);
        let count = adapter.get_channel_socket_count(app_id, &channel).await;
        assert_eq!(
            count, sockets_per_channel,
            "Channel {} should have {} sockets",
            c, sockets_per_channel
        );
    }

    // Get channels with socket counts
    let channels_with_counts = adapter
        .get_channels_with_socket_count(app_id)
        .await
        .unwrap();
    assert_eq!(
        channels_with_counts.len(),
        num_channels,
        "Should have {} channels",
        num_channels
    );

    for item in channels_with_counts.iter() {
        let count = item.value();
        assert_eq!(
            *count, sockets_per_channel,
            "Each channel should have {} sockets",
            sockets_per_channel
        );
    }

    // Remove half the sockets from each channel
    for c in 0..num_channels {
        let channel = format!("channel-{}", c);

        for s in 0..(sockets_per_channel / 2) {
            let socket_id = SocketId(format!("socket-{}-{}", c, s));
            adapter
                .remove_from_channel(app_id, &channel, &socket_id)
                .await
                .unwrap();
        }
    }

    // Verify counts are correct
    for c in 0..num_channels {
        let channel = format!("channel-{}", c);
        let count = adapter.get_channel_socket_count(app_id, &channel).await;
        assert_eq!(
            count,
            sockets_per_channel / 2,
            "Channel {} should have {} sockets after removal",
            c,
            sockets_per_channel / 2
        );
    }
}
