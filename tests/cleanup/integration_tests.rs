#[cfg(test)]
mod tests {
    use sockudo::cleanup::DisconnectTask;
    use sockudo::websocket::SocketId;
    use std::time::Instant;
    use tokio::sync::mpsc;

    fn create_test_task(socket_id: &str, channels: Vec<String>) -> DisconnectTask {
        DisconnectTask {
            socket_id: SocketId(socket_id.to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: channels,
            user_id: Some("user123".to_string()),
            timestamp: Instant::now(),
            connection_info: None,
        }
    }

    #[tokio::test]
    async fn test_multi_worker_fallback_when_worker_full() {
        // Test that when one worker is full, tasks fallback to next worker
        let (tx1, _rx1) = mpsc::channel::<DisconnectTask>(1); // Small buffer
        let (tx2, mut rx2) = mpsc::channel::<DisconnectTask>(10); // Larger buffer

        let senders = [tx1, tx2];

        // Fill first worker
        let filler_task = create_test_task("filler", vec!["channel1".to_string()]);
        assert!(senders[0].try_send(filler_task).is_ok());

        // Simulate MultiWorkerSender fallback logic
        let test_task = create_test_task("test", vec!["channel1".to_string()]);
        let mut send_success = false;

        // Try first worker (should be full)
        if let Err(mpsc::error::TrySendError::Full(task)) = senders[0].try_send(test_task.clone()) {
            // Fallback to second worker
            if senders[1].try_send(task).is_ok() {
                send_success = true;
            }
        }

        assert!(
            send_success,
            "Task should have been sent to second worker via fallback"
        );

        // Verify task was received by second worker
        let received = rx2.recv().await.unwrap();
        assert_eq!(received.socket_id.0, "test");
    }

    #[tokio::test]
    async fn test_multi_worker_all_queues_full_scenario() {
        // Test that when all workers are full, proper error is returned
        let (tx1, _rx1) = mpsc::channel::<DisconnectTask>(1);
        let (tx2, _rx2) = mpsc::channel::<DisconnectTask>(1);

        let senders = vec![tx1, tx2];

        // Fill both workers
        assert!(
            senders[0]
                .try_send(create_test_task("filler1", vec!["ch1".to_string()]))
                .is_ok()
        );
        assert!(
            senders[1]
                .try_send(create_test_task("filler2", vec!["ch2".to_string()]))
                .is_ok()
        );

        // Try to send task when all workers are full
        let test_task = create_test_task("test", vec!["channel1".to_string()]);

        // Simulate MultiWorkerSender logic - try all workers
        let mut all_full = true;
        for sender in &senders {
            if sender.try_send(test_task.clone()).is_ok() {
                all_full = false;
                break;
            }
        }

        assert!(all_full, "All workers should be full, preventing task send");
    }
}
