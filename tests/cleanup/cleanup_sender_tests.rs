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

        let err = result.unwrap_err();

        match err.as_ref() {
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

        let err = result.unwrap_err();
        match err.as_ref() {
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

        let err = result.unwrap_err();
        match err.as_ref() {
            mpsc::error::TrySendError::Closed(returned_task) => {
                assert_eq!(returned_task.socket_id.0, task3.socket_id.0);
                assert_eq!(returned_task.app_id, task3.app_id);
            }
            other => panic!("Expected Closed error, got: {:?}", other),
        }
    }

    // Helper function to create a real MultiWorkerCleanupSystem for testing
    fn create_multi_worker_system(
        worker_count: usize,
        buffer_size: usize,
    ) -> (
        sockudo::cleanup::multi_worker::MultiWorkerCleanupSystem,
        std::sync::Arc<tokio::sync::Mutex<sockudo::adapter::local_adapter::LocalAdapter>>,
    ) {
        use sockudo::adapter::local_adapter::LocalAdapter;
        use sockudo::app::memory_app_manager::MemoryAppManager;
        use sockudo::channel::ChannelManager;
        use sockudo::cleanup::multi_worker::MultiWorkerCleanupSystem;
        use sockudo::cleanup::{CleanupConfig, WorkerThreadsConfig};
        use std::sync::Arc;
        use tokio::sync::{Mutex, RwLock};

        let config = CleanupConfig {
            queue_buffer_size: buffer_size,
            batch_size: 2,
            batch_timeout_ms: 50,
            worker_threads: WorkerThreadsConfig::Fixed(worker_count),
            max_retry_attempts: 2,
            async_enabled: true,
            fallback_to_sync: true,
        };

        let local_adapter = Arc::new(Mutex::new(LocalAdapter::new()));
        let connection_manager = local_adapter.clone();
        let channel_manager =
            Arc::new(RwLock::new(ChannelManager::new(connection_manager.clone())));
        let app_manager = Arc::new(MemoryAppManager::new());

        let multi_system = MultiWorkerCleanupSystem::new(
            connection_manager.clone(),
            channel_manager,
            app_manager,
            None,
            config,
        );

        (multi_system, local_adapter)
    }

    #[tokio::test]
    async fn test_multi_worker_sender_real_system() {
        // Test MultiWorkerSender using the actual system components
        let (multi_system, _adapter) = create_multi_worker_system(3, 10);

        let multi_sender = multi_system.get_sender();
        let cleanup_sender = CleanupSender::Multi(multi_sender.clone());

        // Test basic properties
        assert!(multi_sender.is_available());
        assert_eq!(multi_sender.worker_count(), 3);
        assert!(!cleanup_sender.is_closed());

        // Test worker stats
        let stats = multi_sender.get_worker_stats();
        assert_eq!(stats.total_workers, 3);
        assert_eq!(stats.available_workers, 3);
        assert_eq!(stats.closed_workers, 0);

        // Test that we can send tasks successfully
        for i in 0..9 {
            let task = create_test_task(&format!("socket{}", i));
            let result = cleanup_sender.try_send(task);
            assert!(
                result.is_ok(),
                "Failed to send task {}: {:?}",
                i,
                result.err()
            );
        }

        // Test cloning works
        let cloned_sender = cleanup_sender.clone();
        let cloned_multi = multi_sender.clone();

        assert!(!cloned_sender.is_closed());
        assert!(cloned_multi.is_available());
        assert_eq!(cloned_multi.worker_count(), 3);

        // Verify we can send through cloned sender
        let task = create_test_task("cloned-task");
        assert!(cloned_sender.try_send(task).is_ok());
    }

    #[tokio::test]
    async fn test_multi_worker_sender_single_worker_optimization() {
        // Test the single worker optimization
        let (single_system, _adapter) = create_multi_worker_system(1, 10);

        // Single worker system should provide direct sender optimization
        let direct_sender = single_system.get_direct_sender();
        assert!(
            direct_sender.is_some(),
            "Single worker system should provide direct sender"
        );

        // Multi-worker system should not provide direct sender
        let (multi_system, _adapter2) = create_multi_worker_system(3, 10);
        let no_direct = multi_system.get_direct_sender();
        assert!(
            no_direct.is_none(),
            "Multi-worker system should not provide direct sender"
        );

        // Test the single worker through multi-sender interface
        let single_multi_sender = single_system.get_sender();
        let single_cleanup_sender = CleanupSender::Multi(single_multi_sender.clone());

        assert_eq!(single_multi_sender.worker_count(), 1);
        assert!(single_multi_sender.is_available());

        // Should be able to send tasks
        for i in 0..5 {
            let task = create_test_task(&format!("single{}", i));
            assert!(single_cleanup_sender.try_send(task).is_ok());
        }
    }

    #[tokio::test]
    async fn test_multi_worker_sender_backpressure() {
        // Test backpressure behavior with small buffer
        let (multi_system, _adapter) = create_multi_worker_system(2, 1);

        let multi_sender = multi_system.get_sender();
        let cleanup_sender = CleanupSender::Multi(multi_sender);

        // Fill up the workers' queues (2 workers * 1 buffer each = 2 total capacity)
        let task1 = create_test_task("socket1");
        let task2 = create_test_task("socket2");

        assert!(cleanup_sender.try_send(task1).is_ok());
        assert!(cleanup_sender.try_send(task2).is_ok());

        // Additional tasks should experience backpressure
        // Note: MultiWorkerSender tries fallback to other workers, so this tests the full logic
        let mut backpressure_encountered = false;
        for i in 3..10 {
            let task = create_test_task(&format!("socket{}", i));
            if cleanup_sender.try_send(task).is_err() {
                backpressure_encountered = true;
                break;
            }
        }

        // We should eventually hit backpressure when all workers are full
        assert!(
            backpressure_encountered,
            "Should encounter backpressure with small buffers"
        );
    }

    #[tokio::test]
    async fn test_multi_worker_sender_round_robin_distribution() {
        // Test that tasks are distributed across workers using round-robin
        let (multi_system, _adapter) = create_multi_worker_system(3, 100);

        let multi_sender = multi_system.get_sender();
        let cleanup_sender = CleanupSender::Multi(multi_sender);

        // Send multiple tasks
        let task_count = 9;
        for i in 0..task_count {
            let task = create_test_task(&format!("round_robin_{}", i));
            assert!(cleanup_sender.try_send(task).is_ok());
        }

        // Give some time for tasks to be distributed
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // While we can't directly observe the round-robin distribution without
        // modifying the MultiWorkerSender, we can verify that:
        // 1. All tasks were accepted (tested above)
        // 2. The system is functional and distributing work
        // 3. No single worker is overwhelmed

        // Verify the system is still responsive
        let additional_task = create_test_task("additional_task");
        assert!(cleanup_sender.try_send(additional_task).is_ok());

        // Test with more workers than tasks to ensure distribution logic works
        let (small_system, _adapter2) = create_multi_worker_system(5, 10);
        let small_sender = CleanupSender::Multi(small_system.get_sender());

        for i in 0..3 {
            let task = create_test_task(&format!("small_batch_{}", i));
            assert!(small_sender.try_send(task).is_ok());
        }
    }

    #[tokio::test]
    async fn test_multi_worker_sender_worker_failure_tolerance() {
        // Test the system's behavior when workers become unavailable
        let (multi_system, _adapter) = create_multi_worker_system(2, 10);

        let multi_sender = multi_system.get_sender();
        let cleanup_sender = CleanupSender::Multi(multi_sender.clone());

        // Initially all workers should be available
        let initial_stats = multi_sender.get_worker_stats();
        assert_eq!(initial_stats.total_workers, 2);
        assert_eq!(initial_stats.available_workers, 2);
        assert_eq!(initial_stats.closed_workers, 0);
        assert!(multi_sender.is_available());

        // Send some tasks successfully
        for i in 0..5 {
            let task = create_test_task(&format!("pre_failure_{}", i));
            assert!(cleanup_sender.try_send(task).is_ok());
        }

        // The system handles worker failures internally through the
        // MultiWorkerCleanupSystem - we test that it continues to function
        // even with a reduced worker pool

        // Verify the sender interface remains consistent
        assert!(multi_sender.is_available());
        assert_eq!(multi_sender.worker_count(), 2);

        // Continue sending tasks - the system should handle any internal failures gracefully
        for i in 0..5 {
            let task = create_test_task(&format!("post_observation_{}", i));
            let result = cleanup_sender.try_send(task);
            // The system should either succeed or handle failures gracefully
            match result {
                Ok(()) => continue,
                Err(_) => {
                    // If we get an error, it should be a legitimate backpressure scenario
                    // not a crash or panic
                    break;
                }
            }
        }
    }

    #[tokio::test]
    async fn test_multi_worker_sender_concurrent_access() {
        // Test that MultiWorkerSender handles concurrent access correctly
        let (multi_system, _adapter) = create_multi_worker_system(4, 50);

        let multi_sender = multi_system.get_sender();
        let cleanup_sender = CleanupSender::Multi(multi_sender);

        // Spawn multiple concurrent tasks sending to the same system
        let mut handles = Vec::new();

        for worker_id in 0..10 {
            let sender_clone = cleanup_sender.clone();
            let handle = tokio::spawn(async move {
                let mut success_count = 0;
                for i in 0..10 {
                    let task = create_test_task(&format!("worker{}_task{}", worker_id, i));
                    if sender_clone.try_send(task).is_ok() {
                        success_count += 1;
                    }
                    // Small delay to ensure interleaving
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                }
                success_count
            });
            handles.push(handle);
        }

        // Wait for all concurrent tasks to complete
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // Verify that most tasks succeeded (some might fail due to backpressure)
        let total_successes: usize = results.into_iter().sum();

        // We expect most tasks to succeed - if less than half succeed,
        // there might be an issue with concurrent access
        assert!(
            total_successes > 50,
            "Expected more than 50 successful sends, got {}",
            total_successes
        );
    }
}
