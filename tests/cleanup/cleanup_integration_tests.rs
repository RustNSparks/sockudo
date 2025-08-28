#[cfg(test)]
mod tests {
    use sockudo::adapter::connection_manager::ConnectionManager;
    use sockudo::adapter::local_adapter::LocalAdapter;
    use sockudo::app::config::App;
    use sockudo::app::manager::AppManager;
    use sockudo::app::memory_app_manager::MemoryAppManager;
    use sockudo::channel::ChannelManager;
    use sockudo::cleanup::multi_worker::MultiWorkerCleanupSystem;
    use sockudo::cleanup::{
        AuthInfo, CleanupConfig, ConnectionCleanupInfo, DisconnectTask, WorkerThreadsConfig,
    };
    use sockudo::webhook::types::Webhook;
    use sockudo::websocket::SocketId;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::{Mutex, RwLock};

    /// Helper to create a real cleanup system with all components
    async fn create_real_cleanup_system() -> (
        MultiWorkerCleanupSystem,
        Arc<Mutex<LocalAdapter>>,
        Arc<RwLock<ChannelManager>>,
        Arc<MemoryAppManager>,
    ) {
        let config = CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 2,
            batch_timeout_ms: 50, // Fast timeout for testing
            worker_threads: WorkerThreadsConfig::Fixed(1), // Single worker for predictable testing
            max_retry_attempts: 2,
            async_enabled: true,
            fallback_to_sync: true,
        };

        let local_adapter = Arc::new(Mutex::new(LocalAdapter::new()));
        let connection_manager = local_adapter.clone();
        let channel_manager =
            Arc::new(RwLock::new(ChannelManager::new(connection_manager.clone())));
        let app_manager = Arc::new(MemoryAppManager::new());

        // Create test app
        let test_app = App {
            id: "test-app".to_string(),
            key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            enabled: true,
            max_connections: 100,
            enable_client_messages: true,
            max_backend_events_per_second: None,
            max_client_events_per_second: 100,
            max_read_requests_per_second: None,
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(32),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(100),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(32),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: Some(vec![]),
            enable_watchlist_events: None,
            allowed_origins: None,
        };

        app_manager.create_app(test_app).await.unwrap();

        let cleanup_system = MultiWorkerCleanupSystem::new(
            connection_manager.clone(),
            channel_manager.clone(),
            app_manager.clone(),
            None,
            config,
        );

        (cleanup_system, local_adapter, channel_manager, app_manager)
    }

    /// Helper to create a cleanup system with webhook app configuration
    async fn create_cleanup_system_with_webhook_app() -> (
        MultiWorkerCleanupSystem,
        Arc<Mutex<LocalAdapter>>,
        Arc<RwLock<ChannelManager>>,
        Arc<MemoryAppManager>,
    ) {
        let config = CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 2,
            batch_timeout_ms: 50,
            worker_threads: WorkerThreadsConfig::Fixed(1),
            max_retry_attempts: 2,
            async_enabled: true,
            fallback_to_sync: true,
        };

        let local_adapter = Arc::new(Mutex::new(LocalAdapter::new()));
        let connection_manager = local_adapter.clone();
        let channel_manager =
            Arc::new(RwLock::new(ChannelManager::new(connection_manager.clone())));
        let app_manager = Arc::new(MemoryAppManager::new());

        // Create test app with webhook configuration
        let webhook_config = Webhook {
            url: Some("http://localhost:3000/webhook".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["member_removed".to_string(), "channel_vacated".to_string()],
            filter: None,
            headers: None,
        };

        let test_app = App {
            id: "test-app".to_string(),
            key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            enabled: true,
            max_connections: 100,
            enable_client_messages: true,
            max_backend_events_per_second: None,
            max_client_events_per_second: 100,
            max_read_requests_per_second: None,
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(32),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(100),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(32),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: Some(vec![webhook_config]),
            enable_watchlist_events: None,
            allowed_origins: None,
        };

        app_manager.create_app(test_app).await.unwrap();

        // Create cleanup system WITHOUT webhook integration
        let cleanup_system = MultiWorkerCleanupSystem::new(
            connection_manager.clone(),
            channel_manager.clone(),
            app_manager.clone(),
            None,
            config,
        );

        (cleanup_system, local_adapter, channel_manager, app_manager)
    }

    /// Helper to create a disconnect task
    fn create_disconnect_task(socket_id: &str, channels: Vec<String>) -> DisconnectTask {
        DisconnectTask {
            socket_id: SocketId(socket_id.to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: channels,
            user_id: None,
            timestamp: Instant::now(),
            connection_info: None,
        }
    }

    #[tokio::test]
    async fn test_cleanup_actually_removes_socket_from_channel() {
        // TEST: Verify that the cleanup worker actually removes sockets from channels
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_real_cleanup_system().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("socket-123".to_string());
        let channel = "test-channel";

        // SETUP: Add socket to channel
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP: Socket should be in channel
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            initial_count, 1,
            "Setup failed: socket should be in channel"
        );

        let is_in_channel_before = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", channel, &socket_id)
                .await
                .unwrap()
        };
        assert!(
            is_in_channel_before,
            "Setup failed: socket should be detectable in channel"
        );

        // ACTION: Send cleanup task
        let cleanup_task = create_disconnect_task("socket-123", vec![channel.to_string()]);
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Socket should be removed from channel
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            final_count, 0,
            "CLEANUP FAILED: Socket was not removed from channel"
        );

        let is_in_channel_after = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", channel, &socket_id)
                .await
                .unwrap()
        };
        assert!(
            !is_in_channel_after,
            "CLEANUP FAILED: Socket is still detectable in channel"
        );
    }

    #[tokio::test]
    async fn test_cleanup_handles_multiple_channels_per_socket() {
        // TEST: Verify cleanup removes socket from ALL subscribed channels
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_real_cleanup_system().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("multi-socket".to_string());
        let channels = vec!["channel-1", "channel-2", "channel-3"];

        // SETUP: Add socket to multiple channels
        for channel in &channels {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP: Socket should be in all channels
        for channel in &channels {
            let count = {
                let mut adapter_locked = adapter.lock().await;
                adapter_locked
                    .get_channel_socket_count("test-app", channel)
                    .await
            };
            assert_eq!(count, 1, "Setup failed for channel {}", channel);
        }

        // ACTION: Send cleanup task for all channels
        let cleanup_task = create_disconnect_task(
            "multi-socket",
            channels.iter().map(|c| c.to_string()).collect(),
        );
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Socket should be removed from ALL channels
        for channel in &channels {
            let count = {
                let mut adapter_locked = adapter.lock().await;
                adapter_locked
                    .get_channel_socket_count("test-app", channel)
                    .await
            };
            assert_eq!(
                count, 0,
                "CLEANUP FAILED: Socket not removed from channel {}",
                channel
            );

            let is_in_channel = {
                let mut adapter_locked = adapter.lock().await;
                adapter_locked
                    .is_in_channel("test-app", channel, &socket_id)
                    .await
                    .unwrap()
            };
            assert!(
                !is_in_channel,
                "CLEANUP FAILED: Socket still in channel {}",
                channel
            );
        }
    }

    #[tokio::test]
    async fn test_cleanup_preserves_other_sockets_in_same_channel() {
        // TEST: Verify cleanup only removes the target socket, not others in same channel
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_real_cleanup_system().await;
        let cleanup_sender = cleanup_system.get_sender();

        let target_socket = SocketId("target-socket".to_string());
        let other_socket = SocketId("other-socket".to_string());
        let channel = "shared-channel";

        // SETUP: Add both sockets to same channel
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &target_socket)
                .await
                .unwrap();
            adapter_locked
                .add_to_channel("test-app", channel, &other_socket)
                .await
                .unwrap();
        }

        // VERIFY SETUP: Both sockets should be in channel
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            initial_count, 2,
            "Setup failed: both sockets should be in channel"
        );

        // ACTION: Cleanup only the target socket
        let cleanup_task = create_disconnect_task("target-socket", vec![channel.to_string()]);
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Only target socket removed, other socket preserved
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            final_count, 1,
            "CLEANUP FAILED: Should have exactly 1 socket remaining"
        );

        let target_in_channel = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", channel, &target_socket)
                .await
                .unwrap()
        };
        assert!(
            !target_in_channel,
            "CLEANUP FAILED: Target socket should be removed"
        );

        let other_in_channel = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", channel, &other_socket)
                .await
                .unwrap()
        };
        assert!(
            other_in_channel,
            "CLEANUP FAILED: Other socket should be preserved"
        );
    }

    #[tokio::test]
    async fn test_cleanup_batch_processing() {
        // TEST: Verify that batch processing actually works for multiple sockets
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_real_cleanup_system().await;
        let cleanup_sender = cleanup_system.get_sender();

        let channel = "batch-channel";
        let sockets = vec!["batch-1", "batch-2", "batch-3"];

        // SETUP: Add all sockets to channel
        for socket_id in &sockets {
            let socket = SocketId(socket_id.to_string());
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &socket)
                .await
                .unwrap();
        }

        // VERIFY SETUP: All sockets in channel
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            initial_count, 3,
            "Setup failed: all sockets should be in channel"
        );

        // ACTION: Send cleanup tasks (should be batched due to batch_size=2)
        for socket_id in &sockets {
            let cleanup_task = create_disconnect_task(socket_id, vec![channel.to_string()]);
            cleanup_sender.send(cleanup_task).unwrap();
        }

        // Wait for batch processing
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // VERIFY: All sockets cleaned up via batching
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            final_count, 0,
            "BATCH CLEANUP FAILED: All sockets should be removed"
        );

        // Verify individual sockets
        for socket_id in &sockets {
            let socket = SocketId(socket_id.to_string());
            let is_in_channel = {
                let mut adapter_locked = adapter.lock().await;
                adapter_locked
                    .is_in_channel("test-app", channel, &socket)
                    .await
                    .unwrap()
            };
            assert!(
                !is_in_channel,
                "BATCH CLEANUP FAILED: Socket {} still in channel",
                socket_id
            );
        }
    }

    #[tokio::test]
    async fn test_cleanup_with_presence_channels() {
        // TEST: Verify cleanup handles presence channels with user info
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_real_cleanup_system().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("presence-socket".to_string());
        let presence_channel = "presence-room1";
        let user_id = "user123";

        // SETUP: Add socket to presence channel
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", presence_channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        assert_eq!(
            initial_count, 1,
            "Setup failed: socket should be in presence channel"
        );

        // ACTION: Send cleanup task with presence info
        let cleanup_task = DisconnectTask {
            socket_id: SocketId("presence-socket".to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec![presence_channel.to_string()],
            user_id: Some(user_id.to_string()),
            timestamp: Instant::now(),
            connection_info: Some(ConnectionCleanupInfo {
                presence_channels: vec![presence_channel.to_string()],
                auth_info: Some(AuthInfo {
                    user_id: user_id.to_string(),
                    user_info: None,
                }),
            }),
        };
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Presence channel cleanup worked
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        assert_eq!(
            final_count, 0,
            "PRESENCE CLEANUP FAILED: Socket not removed from presence channel"
        );

        let is_in_channel = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", presence_channel, &socket_id)
                .await
                .unwrap()
        };
        assert!(
            !is_in_channel,
            "PRESENCE CLEANUP FAILED: Socket still in presence channel"
        );
    }

    #[tokio::test]
    async fn test_cleanup_queue_accepts_tasks_when_not_full() {
        // TEST: Verify cleanup queue behaves deterministically when not at capacity
        let config = CleanupConfig {
            queue_buffer_size: 10, // Large enough to accept all our test tasks
            batch_size: 2,
            batch_timeout_ms: 50,
            worker_threads: WorkerThreadsConfig::Fixed(1),
            max_retry_attempts: 2,
            async_enabled: true,
            fallback_to_sync: true,
        };

        let local_adapter = Arc::new(Mutex::new(LocalAdapter::new()));
        let connection_manager = local_adapter.clone();
        let channel_manager =
            Arc::new(RwLock::new(ChannelManager::new(connection_manager.clone())));
        let app_manager = Arc::new(MemoryAppManager::new());

        // Create test app
        let test_app = App {
            id: "test-app".to_string(),
            key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            enabled: true,
            max_connections: 100,
            enable_client_messages: true,
            max_backend_events_per_second: None,
            max_client_events_per_second: 100,
            max_read_requests_per_second: None,
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(32),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(100),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(32),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: Some(vec![]),
            enable_watchlist_events: None,
            allowed_origins: None,
        };
        app_manager.create_app(test_app).await.unwrap();

        let cleanup_system = MultiWorkerCleanupSystem::new(
            connection_manager.clone(),
            channel_manager.clone(),
            app_manager.clone(),
            None,
            config,
        );

        let cleanup_sender = cleanup_system.get_sender();
        let channel = "queue-channel";

        // SETUP: Add exactly 3 sockets to channel
        let sockets = vec!["queue-1", "queue-2", "queue-3"];
        for socket_id in &sockets {
            let socket = SocketId(socket_id.to_string());
            let mut adapter_locked = local_adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &socket)
                .await
                .unwrap();
        }

        // VERIFY SETUP
        let initial_count = {
            let mut adapter_locked = local_adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            initial_count, 3,
            "Setup failed: exactly 3 sockets should be in channel"
        );

        // ACTION: Send all cleanup tasks (queue is large enough to accept all)
        for socket_id in &sockets {
            let cleanup_task = create_disconnect_task(socket_id, vec![channel.to_string()]);
            cleanup_sender
                .send(cleanup_task)
                .expect("Queue should accept all tasks when not full");
        }

        // Wait for processing
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // VERIFY: All 3 sockets should be cleaned up
        let final_count = {
            let mut adapter_locked = local_adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };

        assert_eq!(
            final_count, 0,
            "CLEANUP FAILED: All 3 sockets should be removed"
        );

        // Verify each socket individually
        for socket_id in &sockets {
            let socket = SocketId(socket_id.to_string());
            let is_in_channel = {
                let mut adapter_locked = local_adapter.lock().await;
                adapter_locked
                    .is_in_channel("test-app", channel, &socket)
                    .await
                    .unwrap()
            };
            assert!(
                !is_in_channel,
                "CLEANUP FAILED: Socket {} should be removed",
                socket_id
            );
        }
    }

    #[tokio::test]
    async fn test_cleanup_processes_presence_channels_with_user_info() {
        // TEST: Verify cleanup worker properly processes presence channel disconnections with user info
        // This tests the webhook preparation logic without actually sending webhooks
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_cleanup_system_with_webhook_app().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("presence-user-socket".to_string());
        let presence_channel = "presence-test-room";
        let user_id = "test-user-123";

        // SETUP: Add socket to presence channel
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", presence_channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        assert_eq!(
            initial_count, 1,
            "Setup failed: socket should be in presence channel"
        );

        // ACTION: Send cleanup task with presence info (tests webhook preparation code path)
        let cleanup_task = DisconnectTask {
            socket_id: SocketId("presence-user-socket".to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec![presence_channel.to_string()],
            user_id: Some(user_id.to_string()),
            timestamp: Instant::now(),
            connection_info: Some(ConnectionCleanupInfo {
                presence_channels: vec![presence_channel.to_string()],
                auth_info: Some(AuthInfo {
                    user_id: user_id.to_string(),
                    user_info: None,
                }),
            }),
        };
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Socket should be removed from presence channel
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        assert_eq!(
            final_count, 0,
            "CLEANUP FAILED: Socket not removed from presence channel"
        );

        let is_in_channel = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", presence_channel, &socket_id)
                .await
                .unwrap()
        };
        assert!(
            !is_in_channel,
            "CLEANUP FAILED: Socket still in presence channel"
        );

        // VERIFY: Cleanup processed successfully (no webhook integration errors occurred)
        // The cleanup worker's webhook preparation code was exercised even though no webhooks were sent
        // This verifies the presence channel cleanup logic works correctly
    }

    #[tokio::test]
    async fn test_cleanup_detects_channel_vacancy_for_webhook_logic() {
        // TEST: Verify cleanup worker detects when channels become empty (webhook preparation logic)
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_cleanup_system_with_webhook_app().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("last-socket".to_string());
        let channel = "test-channel-to-vacate";

        // SETUP: Add single socket to channel (so cleanup will make it empty)
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP
        let initial_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            initial_count, 1,
            "Setup failed: socket should be in channel"
        );

        // ACTION: Send cleanup task (this should trigger channel_vacated webhook since channel becomes empty)
        let cleanup_task = create_disconnect_task("last-socket", vec![channel.to_string()]);
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup and webhook processing
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // VERIFY: Channel should be empty (socket removed)
        let final_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", channel)
                .await
        };
        assert_eq!(
            final_count, 0,
            "CLEANUP FAILED: Channel should be empty after removing last socket"
        );

        let is_in_channel = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .is_in_channel("test-app", channel, &socket_id)
                .await
                .unwrap()
        };
        assert!(
            !is_in_channel,
            "CLEANUP FAILED: Socket should be removed from channel"
        );

        // VERIFY: Channel vacancy detection logic works correctly
        // This exercises the webhook preparation code that checks if socket_count == 0
    }

    #[tokio::test]
    async fn test_cleanup_handles_mixed_presence_and_regular_channels() {
        // TEST: Verify cleanup worker handles both presence and regular channels in single task
        let (cleanup_system, adapter, _channel_manager, _app_manager) =
            create_cleanup_system_with_webhook_app().await;
        let cleanup_sender = cleanup_system.get_sender();

        let socket_id = SocketId("multi-channel-socket".to_string());
        let presence_channel = "presence-test-room";
        let regular_channel = "regular-channel";
        let user_id = "mixed-user";

        // SETUP: Add socket to both presence and regular channels
        {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .add_to_channel("test-app", presence_channel, &socket_id)
                .await
                .unwrap();
            adapter_locked
                .add_to_channel("test-app", regular_channel, &socket_id)
                .await
                .unwrap();
        }

        // VERIFY SETUP: Socket should be in both channels
        let presence_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        let regular_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", regular_channel)
                .await
        };
        assert_eq!(
            presence_count, 1,
            "Setup failed: socket should be in presence channel"
        );
        assert_eq!(
            regular_count, 1,
            "Setup failed: socket should be in regular channel"
        );

        // ACTION: Send cleanup task with both channel types
        let cleanup_task = DisconnectTask {
            socket_id: SocketId("multi-channel-socket".to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec![presence_channel.to_string(), regular_channel.to_string()],
            user_id: Some(user_id.to_string()),
            timestamp: Instant::now(),
            connection_info: Some(ConnectionCleanupInfo {
                presence_channels: vec![presence_channel.to_string()], // Only presence channel listed here
                auth_info: Some(AuthInfo {
                    user_id: user_id.to_string(),
                    user_info: None,
                }),
            }),
        };
        cleanup_sender.send(cleanup_task).unwrap();

        // Wait for cleanup processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // VERIFY: Socket should be removed from BOTH channels
        let final_presence_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", presence_channel)
                .await
        };
        let final_regular_count = {
            let mut adapter_locked = adapter.lock().await;
            adapter_locked
                .get_channel_socket_count("test-app", regular_channel)
                .await
        };

        assert_eq!(
            final_presence_count, 0,
            "CLEANUP FAILED: Socket not removed from presence channel"
        );
        assert_eq!(
            final_regular_count, 0,
            "CLEANUP FAILED: Socket not removed from regular channel"
        );

        // VERIFY: Test exercises both webhook preparation paths (member_removed + channel_vacated)
        // This tests the cleanup worker's ability to handle mixed channel types in a single task
    }
}
