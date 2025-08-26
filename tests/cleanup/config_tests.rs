#[cfg(test)]
mod tests {
    use serde_json;
    use sockudo::cleanup::{CleanupConfig, WorkerThreadsConfig};

    #[test]
    fn test_cleanup_config_defaults() {
        let config = CleanupConfig::default();

        assert_eq!(config.queue_buffer_size, 50000);
        assert_eq!(config.batch_size, 25);
        assert_eq!(config.batch_timeout_ms, 50);
        assert!(matches!(config.worker_threads, WorkerThreadsConfig::Auto));
        assert_eq!(config.max_retry_attempts, 2);
        assert!(config.async_enabled);
        assert!(config.fallback_to_sync);
    }

    #[test]
    fn test_worker_threads_config_resolve() {
        // Test Auto resolution
        let auto_config = WorkerThreadsConfig::Auto;
        let resolved = auto_config.resolve();
        assert!(resolved >= 1);
        assert!(resolved <= 4);

        // Should be 25% of CPU count, min 1, max 4
        let cpu_count = num_cpus::get();
        let expected = (cpu_count / 4).max(1).min(4);
        assert_eq!(resolved, expected);

        // Test Fixed resolution
        let fixed_config = WorkerThreadsConfig::Fixed(8);
        assert_eq!(fixed_config.resolve(), 8);
    }

    #[test]
    fn test_worker_threads_config_serialization() {
        // Test Auto serialization
        let auto_config = WorkerThreadsConfig::Auto;
        let json = serde_json::to_string(&auto_config).unwrap();
        assert_eq!(json, "\"auto\"");

        // Test Fixed serialization
        let fixed_config = WorkerThreadsConfig::Fixed(4);
        let json = serde_json::to_string(&fixed_config).unwrap();
        assert_eq!(json, "4");
    }

    #[test]
    fn test_worker_threads_config_deserialization() {
        // Test Auto deserialization
        let auto_config: WorkerThreadsConfig = serde_json::from_str("\"auto\"").unwrap();
        assert!(matches!(auto_config, WorkerThreadsConfig::Auto));

        // Test case insensitive
        let auto_config: WorkerThreadsConfig = serde_json::from_str("\"AUTO\"").unwrap();
        assert!(matches!(auto_config, WorkerThreadsConfig::Auto));

        // Test Fixed deserialization from number
        let fixed_config: WorkerThreadsConfig = serde_json::from_str("4").unwrap();
        assert!(matches!(fixed_config, WorkerThreadsConfig::Fixed(4)));

        // Test Fixed deserialization from string number
        let fixed_config: WorkerThreadsConfig = serde_json::from_str("\"8\"").unwrap();
        assert!(matches!(fixed_config, WorkerThreadsConfig::Fixed(8)));
    }

    #[test]
    fn test_worker_threads_config_deserialization_errors() {
        // Test zero value error
        let result: Result<WorkerThreadsConfig, _> = serde_json::from_str("0");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("greater than 0"));

        // Test negative value (should be caught by u64 parsing)
        let result: Result<WorkerThreadsConfig, _> = serde_json::from_str("-1");
        assert!(result.is_err());

        // Test invalid string
        let result: Result<WorkerThreadsConfig, _> = serde_json::from_str("\"invalid\"");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected \"auto\" or positive integer")
        );

        // Test string zero
        let result: Result<WorkerThreadsConfig, _> = serde_json::from_str("\"0\"");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("greater than 0"));
    }

    #[test]
    fn test_cleanup_config_full_serialization_roundtrip() {
        let original_config = CleanupConfig {
            queue_buffer_size: 10000,
            batch_size: 50,
            batch_timeout_ms: 100,
            worker_threads: WorkerThreadsConfig::Fixed(3),
            max_retry_attempts: 5,
            async_enabled: false,
            fallback_to_sync: false,
        };

        // Serialize
        let json = serde_json::to_string(&original_config).unwrap();

        // Deserialize
        let deserialized: CleanupConfig = serde_json::from_str(&json).unwrap();

        // Verify all fields match
        assert_eq!(
            deserialized.queue_buffer_size,
            original_config.queue_buffer_size
        );
        assert_eq!(deserialized.batch_size, original_config.batch_size);
        assert_eq!(
            deserialized.batch_timeout_ms,
            original_config.batch_timeout_ms
        );
        assert!(matches!(
            deserialized.worker_threads,
            WorkerThreadsConfig::Fixed(3)
        ));
        assert_eq!(
            deserialized.max_retry_attempts,
            original_config.max_retry_attempts
        );
        assert_eq!(deserialized.async_enabled, original_config.async_enabled);
        assert_eq!(
            deserialized.fallback_to_sync,
            original_config.fallback_to_sync
        );
    }
}
