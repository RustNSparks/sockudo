#[cfg(test)]
mod tests {
    use sockudo::cleanup::{CleanupConfig, WorkerThreadsConfig};

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
        let expected = (cpu_count / 4).clamp(1, 4);
        assert_eq!(resolved, expected);
    }
}
