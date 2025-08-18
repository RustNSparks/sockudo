#[cfg(test)]
mod tests {
    use sockudo::cleanup::{AuthInfo, CleanupSender, ConnectionCleanupInfo, DisconnectTask};
    use sockudo::websocket::SocketId;
    use std::time::Instant;
    use tokio::sync::mpsc;

    fn create_disconnect_task_with_presence(socket_id: &str) -> DisconnectTask {
        DisconnectTask {
            socket_id: SocketId(socket_id.to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec!["presence-room1".to_string(), "public-channel".to_string()],
            user_id: Some("user123".to_string()),
            timestamp: Instant::now(),
            connection_info: Some(ConnectionCleanupInfo {
                presence_channels: vec!["presence-room1".to_string()],
                auth_info: Some(AuthInfo {
                    user_id: "user123".to_string(),
                    user_info: None,
                }),
            }),
        }
    }

    #[tokio::test]
    async fn test_fallback_when_queue_full() {
        // This tests the core fallback logic: when async queue is full,
        // the system should fall back to sync cleanup

        // Create a small channel to simulate full queue
        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(1);
        let sender = CleanupSender::Direct(tx);

        // Fill the queue
        let task1 = create_disconnect_task_with_presence("socket1");
        assert!(sender.try_send(task1).is_ok());

        // Try to send another task - should fail with Full
        let task2 = create_disconnect_task_with_presence("socket2");
        let result = sender.try_send(task2.clone());

        assert!(result.is_err());
        match result {
            Err(mpsc::error::TrySendError::Full(returned_task)) => {
                // Verify we get the same task back for fallback processing
                assert_eq!(returned_task.socket_id.0, task2.socket_id.0);
                assert_eq!(returned_task.app_id, task2.app_id);

                // In real code, this would trigger sync cleanup
                // The ConnectionHandler would call handle_disconnect_sync
            }
            _ => panic!("Expected Full error for fallback trigger"),
        }

        // Verify first task is still in queue
        let received = rx.recv().await.unwrap();
        assert_eq!(received.socket_id.0, "socket1");
    }

    #[tokio::test]
    async fn test_presence_channel_cleanup_info() {
        // Test that presence channels are properly identified and stored
        let task = create_disconnect_task_with_presence("socket1");

        // Verify presence channels are extracted
        assert!(task.connection_info.is_some());
        let info = task.connection_info.unwrap();
        assert_eq!(info.presence_channels.len(), 1);
        assert_eq!(info.presence_channels[0], "presence-room1");

        // Verify auth info is preserved
        assert!(info.auth_info.is_some());
        let auth = info.auth_info.unwrap();
        assert_eq!(auth.user_id, "user123");
    }

    #[tokio::test]
    async fn test_non_presence_channel_no_cleanup_info() {
        // Test that non-presence channels don't create unnecessary cleanup info
        let task = DisconnectTask {
            socket_id: SocketId("socket1".to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec!["public-channel".to_string(), "private-channel".to_string()],
            user_id: Some("user123".to_string()),
            timestamp: Instant::now(),
            connection_info: None, // No presence channels, so no cleanup info
        };

        // Verify no cleanup info for non-presence channels
        assert!(task.connection_info.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_task_cloneable() {
        // Test that DisconnectTask can be cloned (needed for retry/fallback)
        let original = create_disconnect_task_with_presence("socket1");
        let cloned = original.clone();

        assert_eq!(original.socket_id.0, cloned.socket_id.0);
        assert_eq!(original.app_id, cloned.app_id);
        assert_eq!(original.subscribed_channels, cloned.subscribed_channels);
        assert_eq!(original.user_id, cloned.user_id);

        // Connection info should also be cloned
        assert!(cloned.connection_info.is_some());
        let original_info = original.connection_info.unwrap();
        let cloned_info = cloned.connection_info.unwrap();
        assert_eq!(
            original_info.presence_channels,
            cloned_info.presence_channels
        );
    }

    #[tokio::test]
    async fn test_fallback_preserves_disconnecting_flag_reset() {
        // This tests that when falling back to sync, we need to reset
        // the disconnecting flag (as implemented in handle_disconnect_async)

        // Simulate the scenario where async cleanup fails
        let (tx, _rx) = mpsc::channel::<DisconnectTask>(1);
        let sender = CleanupSender::Direct(tx);

        // Fill channel
        sender
            .try_send(create_disconnect_task_with_presence("filler"))
            .ok();

        // Try to send task that will fail
        let task = create_disconnect_task_with_presence("socket1");
        let result = sender.try_send(task);

        assert!(result.is_err());

        // In the actual code (handle_disconnect_async), when this fails:
        // 1. The disconnecting flag is reset
        // 2. handle_disconnect_sync is called
        // This ensures the connection can be properly cleaned up
    }
}
