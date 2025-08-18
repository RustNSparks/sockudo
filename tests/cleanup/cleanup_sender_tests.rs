#[cfg(test)]
mod tests {
    use sockudo::cleanup::{CleanupSender, DisconnectTask};
    use sockudo::websocket::SocketId;
    use std::time::Instant;
    use tokio::sync::mpsc;

    fn create_test_task(socket_id: &str) -> DisconnectTask {
        DisconnectTask {
            socket_id: SocketId(socket_id.to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec!["channel1".to_string(), "channel2".to_string()],
            user_id: Some("user123".to_string()),
            timestamp: Instant::now(),
            connection_info: None,
        }
    }

    #[tokio::test]
    async fn test_cleanup_sender_direct_variant() {
        // Test that Direct variant properly sends tasks
        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(10);
        let sender = CleanupSender::Direct(tx);

        let task = create_test_task("socket1");
        let task_clone = task.clone();

        // Should successfully send when channel has space
        assert!(sender.try_send(task).is_ok());

        // Verify task was received
        let received = rx.recv().await.unwrap();
        assert_eq!(received.socket_id.0, task_clone.socket_id.0);
        assert_eq!(received.app_id, task_clone.app_id);
    }

    #[tokio::test]
    async fn test_cleanup_sender_direct_backpressure() {
        // Test that Direct variant handles backpressure correctly
        let (tx, _rx) = mpsc::channel::<DisconnectTask>(2);
        let sender = CleanupSender::Direct(tx);

        // Fill the channel
        assert!(sender.try_send(create_test_task("socket1")).is_ok());
        assert!(sender.try_send(create_test_task("socket2")).is_ok());

        // Third send should fail with Full error
        let result = sender.try_send(create_test_task("socket3"));
        assert!(result.is_err());

        match result.unwrap_err() {
            mpsc::error::TrySendError::Full(_) => {
                // Expected: channel is full
            }
            _ => panic!("Expected Full error"),
        }
    }

    #[tokio::test]
    async fn test_cleanup_sender_try_send_interface() {
        // Test that try_send interface is consistent across variants
        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(10);
        let sender = CleanupSender::Direct(tx);

        let task = create_test_task("socket1");
        let task_clone = task.clone();

        // Test successful send
        match sender.try_send(task) {
            Ok(()) => {
                // Verify task was received
                let received = rx.recv().await.unwrap();
                assert_eq!(received.socket_id.0, task_clone.socket_id.0);
            }
            Err(_) => panic!("Expected successful send"),
        }
    }

    #[tokio::test]
    async fn test_cleanup_sender_is_closed() {
        // Test is_closed() for Direct variant
        let (tx, rx) = mpsc::channel::<DisconnectTask>(10);
        let sender = CleanupSender::Direct(tx);

        assert!(!sender.is_closed());

        drop(rx); // Close the receiver

        // Give time for closure to propagate
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        assert!(sender.is_closed());
    }

    #[tokio::test]
    async fn test_cleanup_sender_clone_semantics() {
        // Test that CleanupSender can be cloned safely
        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(10);
        let sender = CleanupSender::Direct(tx);
        let sender_clone = sender.clone();

        // Both should work
        assert!(sender.try_send(create_test_task("socket1")).is_ok());
        assert!(sender_clone.try_send(create_test_task("socket2")).is_ok());

        // Both should be received
        let _ = rx.recv().await.unwrap();
        let _ = rx.recv().await.unwrap();
    }

    #[tokio::test]
    async fn test_cleanup_sender_error_types() {
        // Test different error conditions with specific error type verification
        let (tx, rx) = mpsc::channel::<DisconnectTask>(1);
        let sender = CleanupSender::Direct(tx);

        // Fill channel
        assert!(sender.try_send(create_test_task("socket1")).is_ok());

        // Next send should fail with Full error
        let task2 = create_test_task("socket2");
        let result = sender.try_send(task2.clone());
        assert!(result.is_err());

        match result.unwrap_err() {
            mpsc::error::TrySendError::Full(returned_task) => {
                assert_eq!(returned_task.socket_id.0, task2.socket_id.0);
                assert_eq!(returned_task.app_id, task2.app_id);
            }
            other => panic!("Expected Full error, got: {:?}", other),
        }

        // Close receiver to test Closed error
        drop(rx);
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let task3 = create_test_task("socket3");
        let result = sender.try_send(task3.clone());
        assert!(result.is_err());

        match result.unwrap_err() {
            mpsc::error::TrySendError::Closed(returned_task) => {
                assert_eq!(returned_task.socket_id.0, task3.socket_id.0);
                assert_eq!(returned_task.app_id, task3.app_id);
            }
            other => panic!("Expected Closed error, got: {:?}", other),
        }
    }
}
