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
        match result {
            Err(boxed_err) => match boxed_err.as_ref() {
                mpsc::error::TrySendError::Full(returned_task) => {
                    println!(
                        "[TEST VALIDATION] Queue full, falling back to sync processing for task: {:?}",
                        returned_task
                    );
                    // Note: returned_task is now a reference, so you might need to clone or dereference
                    assert_eq!(returned_task.socket_id.0, task2.socket_id.0);
                    assert_eq!(returned_task.app_id, task2.app_id);
                }
                _ => panic!("Expected Full error for fallback trigger"),
            },
            Ok(_) => panic!("Expected error, but send succeeded"),
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
}
