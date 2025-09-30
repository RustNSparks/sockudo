pub mod handler;
pub mod transports;

#[cfg(test)]
mod horizontal_adapter_helpers;

#[cfg(test)]
mod horizontal_adapter_base_test;

#[cfg(test)]
mod horizontal_adapter_integration;

#[cfg(test)]
mod horizontal_adapter_failure_tests;

#[cfg(test)]
mod horizontal_adapter_race_tests;

#[cfg(test)]
mod cluster_health_tests;

#[cfg(test)]
mod cluster_health_integration_tests;

#[cfg(test)]
mod presence_synchronization_tests;

#[cfg(test)]
mod dead_node_detection_tests;

#[cfg(test)]
mod cross_node_sync_tests;

#[cfg(test)]
mod redis_adapter_cluster_health_tests;

#[cfg(test)]
mod redis_cluster_adapter_health_tests;

#[cfg(test)]
mod nats_adapter_cluster_health_tests;

#[cfg(test)]
mod presence_state_consistency_tests;

#[cfg(test)]
mod local_adapter_fallback_tests;

#[cfg(test)]
mod single_node_optimization_tests;
