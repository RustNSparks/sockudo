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
    async fn test_fallback_queue_full_with_multiple_tasks() {
        // Test behavior when queue becomes full with multiple tasks in rapid succession
        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(2); // Small buffer
        let sender = CleanupSender::Direct(tx);

        // Fill the queue completely
        assert!(
            sender
                .try_send(create_disconnect_task_with_presence("socket1"))
                .is_ok()
        );
        assert!(
            sender
                .try_send(create_disconnect_task_with_presence("socket2"))
                .is_ok()
        );

        // Try to send multiple additional tasks that should all fail
        let failed_tasks = vec![
            create_disconnect_task_with_presence("socket3"),
            create_disconnect_task_with_presence("socket4"),
            create_disconnect_task_with_presence("socket5"),
        ];

        let mut fallback_tasks = Vec::new();

        for task in failed_tasks {
            match sender.try_send(task.clone()) {
                Err(mpsc::error::TrySendError::Full(returned_task)) => {
                    fallback_tasks.push(returned_task);
                }
                Ok(()) => panic!("Task should have failed due to full queue"),
                Err(other) => panic!("Unexpected error: {:?}", other),
            }
        }

        // All three tasks should have been returned for fallback
        assert_eq!(fallback_tasks.len(), 3);
        assert_eq!(fallback_tasks[0].socket_id.0, "socket3");
        assert_eq!(fallback_tasks[1].socket_id.0, "socket4");
        assert_eq!(fallback_tasks[2].socket_id.0, "socket5");

        // Verify original tasks are still in queue
        let task1 = rx.recv().await.unwrap();
        let task2 = rx.recv().await.unwrap();
        assert_eq!(task1.socket_id.0, "socket1");
        assert_eq!(task2.socket_id.0, "socket2");

        // Queue should now be empty
        assert!(rx.try_recv().is_err());
    }
}
