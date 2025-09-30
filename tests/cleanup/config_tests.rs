#[cfg(test)]
mod tests {
    use sockudo::cleanup::{CleanupConfig, WorkerThreadsConfig};
    use sonic_rs;

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
        let expected = (cpu_count / 4).clamp(1, 4);
        assert_eq!(resolved, expected);

        // Test Fixed resolution
        let fixed_config = WorkerThreadsConfig::Fixed(8);
        assert_eq!(fixed_config.resolve(), 8);
    }

    #[test]
    fn test_worker_threads_config_serialization() {
        // Test Auto serialization
        let auto_config = WorkerThreadsConfig::Auto;
        let json = sonic_rs::to_string(&auto_config).unwrap();
        assert_eq!(json, "\"auto\"");

        // Test Fixed serialization
        let fixed_config = WorkerThreadsConfig::Fixed(4);
        let json = sonic_rs::to_string(&fixed_config).unwrap();
        assert_eq!(json, "4");
    }

    #[test]
    fn test_worker_threads_config_deserialization() {
        // Test Auto deserialization
        let auto_config: WorkerThreadsConfig = sonic_rs::from_str("\"auto\"").unwrap();
        assert!(matches!(auto_config, WorkerThreadsConfig::Auto));

        // Test case insensitive
        let auto_config: WorkerThreadsConfig = sonic_rs::from_str("\"AUTO\"").unwrap();
        assert!(matches!(auto_config, WorkerThreadsConfig::Auto));

        // Test Fixed deserialization from number
        let fixed_config: WorkerThreadsConfig = sonic_rs::from_str("4").unwrap();
        assert!(matches!(fixed_config, WorkerThreadsConfig::Fixed(4)));

        // Test Fixed deserialization from string number
        let fixed_config: WorkerThreadsConfig = sonic_rs::from_str("\"8\"").unwrap();
        assert!(matches!(fixed_config, WorkerThreadsConfig::Fixed(8)));
    }

    #[test]
    fn test_worker_threads_config_deserialization_errors() {
        // Test zero value error
        let result: Result<WorkerThreadsConfig, _> = sonic_rs::from_str("0");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("greater than 0"));

        // Test negative value (should be caught by u64 parsing)
        let result: Result<WorkerThreadsConfig, _> = sonic_rs::from_str("-1");
        assert!(result.is_err());

        // Test invalid string
        let result: Result<WorkerThreadsConfig, _> = sonic_rs::from_str("\"invalid\"");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected \"auto\" or positive integer")
        );

        // Test string zero
        let result: Result<WorkerThreadsConfig, _> = sonic_rs::from_str("\"0\"");
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
        let json = sonic_rs::to_string(&original_config).unwrap();

        // Deserialize
        let deserialized: CleanupConfig = sonic_rs::from_str(&json).unwrap();

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

    #[test]
    fn test_cleanup_config_validation_valid_configs() {
        // Test default config is valid
        let config = CleanupConfig::default();
        assert!(config.validate().is_ok());

        // Test valid config with Auto workers
        let config = CleanupConfig {
            queue_buffer_size: 1000,
            batch_size: 10,
            batch_timeout_ms: 100,
            worker_threads: WorkerThreadsConfig::Auto,
            max_retry_attempts: 3,
            async_enabled: true,
            fallback_to_sync: true,
        };
        assert!(config.validate().is_ok());

        // Test valid config with Fixed workers
        let config = CleanupConfig {
            queue_buffer_size: 1000,
            batch_size: 10,
            batch_timeout_ms: 100,
            worker_threads: WorkerThreadsConfig::Fixed(4),
            max_retry_attempts: 3,
            async_enabled: true,
            fallback_to_sync: false,
        };
        assert!(config.validate().is_ok());

        // Test edge case: only fallback_to_sync enabled
        let config = CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 10,
            batch_timeout_ms: 100,
            worker_threads: WorkerThreadsConfig::Auto,
            max_retry_attempts: 1,
            async_enabled: false,
            fallback_to_sync: true,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cleanup_config_validation_invalid_queue_buffer_size() {
        let config = CleanupConfig {
            queue_buffer_size: 0,
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "queue_buffer_size must be greater than 0"
        );
    }

    #[test]
    fn test_cleanup_config_validation_invalid_batch_size() {
        let config = CleanupConfig {
            batch_size: 0,
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "batch_size must be greater than 0");
    }

    #[test]
    fn test_cleanup_config_validation_invalid_batch_timeout() {
        let config = CleanupConfig {
            batch_timeout_ms: 0,
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "batch_timeout_ms must be greater than 0"
        );
    }

    #[test]
    fn test_cleanup_config_validation_invalid_fixed_workers() {
        let config = CleanupConfig {
            worker_threads: WorkerThreadsConfig::Fixed(0),
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "worker_threads must be greater than 0 when using fixed count"
        );
    }

    #[test]
    fn test_cleanup_config_validation_queue_smaller_than_batch() {
        let config = CleanupConfig {
            queue_buffer_size: 5,
            batch_size: 10,
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "queue_buffer_size (5) should be at least as large as batch_size (10)"
        );
    }

    #[test]
    fn test_cleanup_config_validation_timeout_too_high() {
        let config = CleanupConfig {
            batch_timeout_ms: 70000, // 70 seconds
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("batch_timeout_ms (70000) is unusually high")
        );
    }

    #[test]
    fn test_cleanup_config_validation_neither_async_nor_sync() {
        let config = CleanupConfig {
            async_enabled: false,
            fallback_to_sync: false,
            ..CleanupConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Either async_enabled or fallback_to_sync must be true"
        );
    }

    #[test]
    fn test_cleanup_config_validation_auto_workers_passes() {
        // Specifically test that Auto worker configuration passes validation
        let config = CleanupConfig {
            worker_threads: WorkerThreadsConfig::Auto,
            ..CleanupConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cleanup_config_validation_edge_cases() {
        // Test maximum reasonable timeout (60 seconds exactly should pass)
        let config = CleanupConfig {
            batch_timeout_ms: 60000,
            ..CleanupConfig::default()
        };
        assert!(config.validate().is_ok());

        // Test 60001ms should fail
        let config = CleanupConfig {
            batch_timeout_ms: 60001,
            ..CleanupConfig::default()
        };
        assert!(config.validate().is_err());

        // Test queue_buffer_size equals batch_size (should pass)
        let config = CleanupConfig {
            queue_buffer_size: 100,
            batch_size: 100,
            ..CleanupConfig::default()
        };
        assert!(config.validate().is_ok());
    }
}
