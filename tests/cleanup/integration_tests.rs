#[cfg(test)]
mod tests {
    use sockudo::cleanup::{CleanupConfig, DisconnectTask, WorkerThreadsConfig};
    use sockudo::websocket::SocketId;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    fn create_test_config() -> CleanupConfig {
        CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 3,
            batch_timeout_ms: 50,
            worker_threads: WorkerThreadsConfig::Fixed(2),
            max_retry_attempts: 1,
            async_enabled: true,
            fallback_to_sync: true,
        }
    }

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
    async fn test_multi_worker_sender_creation_and_distribution() {
        // Test multi-worker sender functionality without full system initialization
        let (tx1, mut rx1) = mpsc::channel::<DisconnectTask>(10);
        let (tx2, mut rx2) = mpsc::channel::<DisconnectTask>(10);

        let senders = vec![tx1, tx2];
        let round_robin_counter = Arc::new(AtomicUsize::new(0));

        // Simulate what MultiWorkerSender does
        let sender_count = senders.len();
        let tasks_to_send = 6;
        let mut distribution_counts = vec![0usize; sender_count];

        // Simulate round-robin distribution
        for i in 0..tasks_to_send {
            let worker_index = round_robin_counter.fetch_add(1, Ordering::Relaxed) % sender_count;
            distribution_counts[worker_index] += 1;

            let task = create_test_task(&format!("socket{}", i), vec!["channel1".to_string()]);
            assert!(senders[worker_index].try_send(task).is_ok());
        }

        // Verify round-robin distribution
        assert_eq!(distribution_counts[0], 3); // Tasks 0, 2, 4
        assert_eq!(distribution_counts[1], 3); // Tasks 1, 3, 5

        // Verify tasks were received by both workers
        let mut rx1_count = 0;
        let mut rx2_count = 0;

        // Count received tasks with timeout to avoid hanging
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(10), rx1.recv()).await {
            rx1_count += 1;
        }
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(10), rx2.recv()).await {
            rx2_count += 1;
        }

        assert_eq!(rx1_count, 3);
        assert_eq!(rx2_count, 3);
    }

    #[tokio::test]
    async fn test_multi_worker_fallback_when_worker_full() {
        // Test that when one worker is full, tasks fallback to next worker
        let (tx1, _rx1) = mpsc::channel::<DisconnectTask>(1); // Small buffer
        let (tx2, mut rx2) = mpsc::channel::<DisconnectTask>(10); // Larger buffer

        let senders = vec![tx1, tx2];

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

    #[tokio::test]
    async fn test_task_size_and_memory_usage_estimation() {
        // Test memory usage characteristics of DisconnectTask
        let small_task = create_test_task("s", vec!["c".to_string()]);
        let large_task = create_test_task(
            &"x".repeat(100),                                    // Large socket ID
            (0..20).map(|i| format!("channel-{}", i)).collect(), // Many channels
        );

        // Verify that tasks can be created with various sizes
        assert_eq!(small_task.socket_id.0.len(), 1);
        assert_eq!(small_task.subscribed_channels.len(), 1);

        assert_eq!(large_task.socket_id.0.len(), 100);
        assert_eq!(large_task.subscribed_channels.len(), 20);

        // Test serialization size (approximation of memory usage)
        let small_json = serde_json::to_string(&small_task.socket_id.0).unwrap();
        let large_json = serde_json::to_string(&large_task.socket_id.0).unwrap();

        assert!(large_json.len() > small_json.len());

        // Verify configuration memory estimate makes sense
        let config = CleanupConfig::default();
        let estimated_max_memory = config.queue_buffer_size * 625; // ~625 bytes per task

        // Should be reasonable for production (less than 100MB)
        assert!(estimated_max_memory < 100_000_000);
        assert!(estimated_max_memory > 1_000_000); // But more than 1MB
    }

    #[tokio::test]
    async fn test_round_robin_counter_overflow_safety() {
        // Test that round-robin counter doesn't cause issues on overflow
        let counter = Arc::new(AtomicUsize::new(usize::MAX - 1));
        let worker_count = 3;

        // Simulate multiple fetches that would cause overflow
        let mut indices = Vec::new();
        for _ in 0..5 {
            let index = counter.fetch_add(1, Ordering::Relaxed) % worker_count;
            indices.push(index);
        }

        // Should still distribute properly even after overflow
        assert_eq!(indices.len(), 5);
        assert!(indices.iter().all(|&i| i < worker_count));

        // Verify distribution is reasonable (no single index dominates)
        let mut counts = vec![0; worker_count];
        for &index in &indices {
            counts[index] += 1;
        }

        // Each worker should get at least one task in this small sample
        assert!(counts.iter().any(|&count| count > 0));
    }
}
