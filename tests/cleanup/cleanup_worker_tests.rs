#[cfg(test)]
mod tests {
    use sockudo::cleanup::{CleanupConfig, DisconnectTask, WorkerThreadsConfig};
    use sockudo::websocket::SocketId;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::{Mutex, mpsc};
    use tokio::time::timeout;

    fn create_test_config() -> CleanupConfig {
        CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 5,
            batch_timeout_ms: 50,
            worker_threads: WorkerThreadsConfig::Fixed(2),
            max_retry_attempts: 2,
            async_enabled: true,
            fallback_to_sync: true,
        }
    }

    fn create_task(id: &str) -> DisconnectTask {
        DisconnectTask {
            socket_id: SocketId(id.to_string()),
            app_id: "test-app".to_string(),
            subscribed_channels: vec!["channel1".to_string()],
            user_id: None,
            timestamp: Instant::now(),
            connection_info: None,
        }
    }

    #[tokio::test]
    async fn test_bounded_channel_respects_buffer_size() {
        // Test that channel buffer size is actually enforced
        let config = CleanupConfig {
            queue_buffer_size: 3, // Small buffer for testing
            ..create_test_config()
        };

        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(config.queue_buffer_size);

        // Should accept up to buffer size
        assert!(tx.try_send(create_task("1")).is_ok());
        assert!(tx.try_send(create_task("2")).is_ok());
        assert!(tx.try_send(create_task("3")).is_ok());

        // Fourth should fail - buffer full
        assert!(tx.try_send(create_task("4")).is_err());

        // Consume one
        rx.recv().await.unwrap();

        // Now should accept one more
        assert!(tx.try_send(create_task("5")).is_ok());
    }

    #[tokio::test]
    async fn test_batch_processing() {
        // Test that tasks are processed in batches
        let config = CleanupConfig {
            batch_size: 3,
            batch_timeout_ms: 100,
            ..create_test_config()
        };

        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(10);
        let processed = Arc::new(AtomicUsize::new(0));
        let processed_clone = processed.clone();

        // Simulate batch processing
        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(config.batch_size);

            while let Ok(task) =
                timeout(Duration::from_millis(config.batch_timeout_ms), rx.recv()).await
            {
                if let Some(task) = task {
                    batch.push(task);

                    if batch.len() >= config.batch_size {
                        // Process batch
                        processed_clone.fetch_add(batch.len(), Ordering::SeqCst);
                        batch.clear();
                    }
                }
            }

            // Process remaining
            if !batch.is_empty() {
                processed_clone.fetch_add(batch.len(), Ordering::SeqCst);
            }
        });

        // Send tasks
        for i in 0..7 {
            tx.try_send(create_task(&i.to_string())).unwrap();
        }

        // Wait for processing
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Should have processed all 7 tasks in batches (3 + 3 + 1)
        assert_eq!(processed.load(Ordering::SeqCst), 7);
    }

    #[tokio::test]
    async fn test_batch_timeout_triggers_processing() {
        // Test that batch timeout triggers processing even if batch isn't full
        let config = CleanupConfig {
            batch_size: 10,       // Large batch size
            batch_timeout_ms: 50, // Short timeout
            ..create_test_config()
        };

        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(10);
        let processed = Arc::new(Mutex::new(Vec::new()));
        let processed_clone = processed.clone();

        tokio::spawn(async move {
            let mut batch = Vec::new();
            let mut last_batch_time = Instant::now();

            loop {
                tokio::select! {
                    task = rx.recv() => {
                        match task {
                            Some(task) => {
                                batch.push(task);

                                if batch.len() >= config.batch_size
                                    || last_batch_time.elapsed() >= Duration::from_millis(config.batch_timeout_ms) {
                                    let mut p = processed_clone.lock().await;
                                    p.extend(batch.clone());
                                    batch.clear();
                                    last_batch_time = Instant::now();
                                }
                            }
                            None => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(config.batch_timeout_ms)) => {
                        if !batch.is_empty() {
                            let mut p = processed_clone.lock().await;
                            p.extend(batch.clone());
                            batch.clear();
                            last_batch_time = Instant::now();
                        }
                    }
                }
            }
        });

        // Send only 2 tasks (less than batch size)
        tx.try_send(create_task("1")).unwrap();
        tx.try_send(create_task("2")).unwrap();

        // Wait for timeout to trigger
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should have processed due to timeout
        let p = processed.lock().await;
        assert_eq!(p.len(), 2);
    }

    #[tokio::test]
    async fn test_worker_threads_auto_detection() {
        // Test that Auto configuration properly detects CPU count
        let config = CleanupConfig {
            worker_threads: WorkerThreadsConfig::Auto,
            ..create_test_config()
        };

        let resolved = config.worker_threads.resolve();

        // Should be between 1 and 4 (25% of CPUs, min 1, max 4)
        assert!(resolved >= 1);
        assert!(resolved <= 4);

        // For most systems, should be reasonable
        let cpu_count = num_cpus::get();
        let expected = (cpu_count / 4).max(1).min(4);
        assert_eq!(resolved, expected);
    }

    #[tokio::test]
    async fn test_backpressure_handling() {
        // Test that system handles backpressure correctly
        let config = CleanupConfig {
            queue_buffer_size: 2,
            ..create_test_config()
        };

        let (tx, mut rx) = mpsc::channel::<DisconnectTask>(config.queue_buffer_size);

        // Fill buffer
        tx.try_send(create_task("1")).unwrap();
        tx.try_send(create_task("2")).unwrap();

        // This should fail - triggering backpressure
        let task3 = create_task("3");
        let result = tx.try_send(task3.clone());

        assert!(result.is_err());
        match result {
            Err(mpsc::error::TrySendError::Full(returned_task)) => {
                // Task is returned for fallback processing
                assert_eq!(returned_task.socket_id.0, "3");
            }
            _ => panic!("Expected Full error"),
        }

        // Drain one task
        let _ = rx.recv().await;

        // Now should accept new task
        assert!(tx.try_send(create_task("4")).is_ok());
    }

    #[tokio::test]
    async fn test_channel_grouping_optimization() {
        // Test that tasks are grouped by channel for efficient processing
        let tasks = vec![
            DisconnectTask {
                socket_id: SocketId("s1".to_string()),
                app_id: "app1".to_string(),
                subscribed_channels: vec!["ch1".to_string(), "ch2".to_string()],
                user_id: None,
                timestamp: Instant::now(),
                connection_info: None,
            },
            DisconnectTask {
                socket_id: SocketId("s2".to_string()),
                app_id: "app1".to_string(),
                subscribed_channels: vec!["ch1".to_string(), "ch3".to_string()],
                user_id: None,
                timestamp: Instant::now(),
                connection_info: None,
            },
        ];

        // Group by channels (simulating what process_batch does)
        let mut channel_operations = std::collections::HashMap::new();

        for task in &tasks {
            for channel in &task.subscribed_channels {
                channel_operations
                    .entry((task.app_id.clone(), channel.clone()))
                    .or_insert_with(Vec::new)
                    .push(task.socket_id.clone());
            }
        }

        // Verify grouping
        assert_eq!(channel_operations.len(), 3); // ch1, ch2, ch3

        let ch1_sockets = &channel_operations[&("app1".to_string(), "ch1".to_string())];
        assert_eq!(ch1_sockets.len(), 2); // Both s1 and s2 are in ch1
    }
}
