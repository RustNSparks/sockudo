use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::options::ClusterHealthConfig;

#[tokio::test]
async fn test_cluster_health_config_validation_success() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 5000,
        node_timeout_ms: 20000, // 4x heartbeat interval
        cleanup_interval_ms: 8000,
    };

    assert!(config.validate().is_ok());
}

#[tokio::test]
async fn test_cluster_health_config_validation_heartbeat_too_high() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 15000,
        node_timeout_ms: 30000, // heartbeat >= node_timeout/3 (10000)
        cleanup_interval_ms: 10000,
    };

    let result = config.validate();
    assert!(result.is_err());
    let error_msg = result.unwrap_err();
    assert!(error_msg.contains("heartbeat_interval_ms"));
    assert!(error_msg.contains("should be at least 3x smaller than node_timeout_ms"));
}

#[tokio::test]
async fn test_cluster_health_config_validation_zero_heartbeat() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 0,
        node_timeout_ms: 30000,
        cleanup_interval_ms: 10000,
    };

    let result = config.validate();
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "heartbeat_interval_ms must be greater than 0"
    );
}

#[tokio::test]
async fn test_cluster_health_config_validation_zero_node_timeout() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 10000,
        node_timeout_ms: 0,
        cleanup_interval_ms: 10000,
    };

    let result = config.validate();
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "node_timeout_ms must be greater than 0"
    );
}

#[tokio::test]
async fn test_cluster_health_config_validation_zero_cleanup_interval() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 10000,
        node_timeout_ms: 30000,
        cleanup_interval_ms: 0,
    };

    let result = config.validate();
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "cleanup_interval_ms must be greater than 0"
    );
}

#[tokio::test]
async fn test_cluster_health_config_validation_cleanup_interval_too_high() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 5000,
        node_timeout_ms: 20000,
        cleanup_interval_ms: 25000, // > node_timeout_ms
    };

    let result = config.validate();
    assert!(result.is_err());
    let error_msg = result.unwrap_err();
    assert!(error_msg.contains("cleanup_interval_ms"));
    assert!(error_msg.contains("should not be larger than node_timeout_ms"));
}

#[tokio::test]
async fn test_cluster_health_config_validation_boundary_case() {
    // Test exact boundary: heartbeat = node_timeout/3
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 10000,
        node_timeout_ms: 30000, // 30000/3 = 10000, so heartbeat == node_timeout/3
        cleanup_interval_ms: 15000,
    };

    let result = config.validate();
    assert!(result.is_ok()); // Should now pass because heartbeat == node_timeout/3 is allowed
}

#[tokio::test]
async fn test_cluster_health_config_default_values() {
    let config = ClusterHealthConfig::default();

    assert!(config.enabled);
    assert_eq!(config.heartbeat_interval_ms, 10000);
    assert_eq!(config.node_timeout_ms, 30000);
    assert_eq!(config.cleanup_interval_ms, 10000);

    // Verify default values pass validation
    assert!(config.validate().is_ok());
}

#[tokio::test]
async fn test_cluster_health_config_default_values_ratios() {
    let config = ClusterHealthConfig::default();

    // Verify good ratios in default config
    assert!(config.heartbeat_interval_ms <= config.node_timeout_ms / 3);
    assert!(config.cleanup_interval_ms <= config.node_timeout_ms);
    assert!(config.heartbeat_interval_ms > 0);
}

#[tokio::test]
async fn test_set_cluster_health_applies_configuration() {
    let config = MockConfig::default();
    let mut adapter: HorizontalAdapterBase<MockTransport> =
        HorizontalAdapterBase::new(config).await.unwrap();

    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 4000, // 4000 < 15000/3 = 5000 ✓
        node_timeout_ms: 15000,
        cleanup_interval_ms: 7000,
    };

    let result = adapter.set_cluster_health(&config).await;
    assert!(result.is_ok());

    // Verify configuration was applied
    assert!(adapter.cluster_health_enabled);
    assert_eq!(adapter.heartbeat_interval_ms, 4000);
    assert_eq!(adapter.node_timeout_ms, 15000);
    assert_eq!(adapter.cleanup_interval_ms, 7000);
}

#[tokio::test]
async fn test_set_cluster_health_with_invalid_config_uses_defaults() {
    let config = MockConfig::default();
    let mut adapter: HorizontalAdapterBase<MockTransport> =
        HorizontalAdapterBase::new(config).await.unwrap();

    let invalid_config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 0, // Invalid
        node_timeout_ms: 15000,
        cleanup_interval_ms: 7000,
    };

    let result = adapter.set_cluster_health(&invalid_config).await;
    assert!(result.is_ok());

    // Should keep existing values (which are now ClusterHealthConfig defaults)
    let defaults = ClusterHealthConfig::default();
    assert_eq!(adapter.cluster_health_enabled, defaults.enabled);
    assert_eq!(
        adapter.heartbeat_interval_ms,
        defaults.heartbeat_interval_ms
    );
    assert_eq!(adapter.node_timeout_ms, defaults.node_timeout_ms);
    assert_eq!(adapter.cleanup_interval_ms, defaults.cleanup_interval_ms);
}

#[tokio::test]
async fn test_cluster_health_disabled_flag_behavior() {
    let config = MockConfig::default();
    let mut adapter: HorizontalAdapterBase<MockTransport> =
        HorizontalAdapterBase::new(config).await.unwrap();

    let config = ClusterHealthConfig {
        enabled: false,
        heartbeat_interval_ms: 4000, // 4000 < 15000/3 = 5000 ✓
        node_timeout_ms: 15000,
        cleanup_interval_ms: 7000,
    };

    let result = adapter.set_cluster_health(&config).await;
    assert!(result.is_ok());

    // Should apply timing values even when disabled
    assert!(!adapter.cluster_health_enabled);
    assert_eq!(adapter.heartbeat_interval_ms, 4000);
    assert_eq!(adapter.node_timeout_ms, 15000);
    assert_eq!(adapter.cleanup_interval_ms, 7000);
}

#[tokio::test]
async fn test_cluster_health_field_direct_access() {
    let config = MockConfig::default();
    let adapter: HorizontalAdapterBase<MockTransport> =
        HorizontalAdapterBase::new(config).await.unwrap();

    // Test direct field access doesn't require async
    let enabled = adapter.cluster_health_enabled;
    let heartbeat = adapter.heartbeat_interval_ms;
    let timeout = adapter.node_timeout_ms;
    let cleanup = adapter.cleanup_interval_ms;

    // Default values should match ClusterHealthConfig defaults (since base adapter now uses them)
    let defaults = ClusterHealthConfig::default();
    assert_eq!(enabled, defaults.enabled);
    assert_eq!(heartbeat, defaults.heartbeat_interval_ms);
    assert_eq!(timeout, defaults.node_timeout_ms);
    assert_eq!(cleanup, defaults.cleanup_interval_ms);
}
