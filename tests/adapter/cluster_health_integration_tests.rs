use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use sockudo::options::ClusterHealthConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Helper to create a cluster of mock nodes for testing
pub struct MockCluster {
    pub nodes: Vec<HorizontalAdapterBase<MockTransport>>,
    pub _node_ids: Vec<String>,
    pub _transports: Vec<MockTransport>,
}

impl MockCluster {
    /// Create a cluster with specified number of nodes
    pub async fn new(node_count: usize) -> Self {
        let mut nodes = Vec::new();
        let mut node_ids = Vec::new();
        let mut transports = Vec::new();

        // Create shared state for all mock transports to simulate real cluster communication
        let shared_state = Arc::new(Mutex::new(HashMap::new()));

        for i in 0..node_count {
            let config = MockConfig {
                prefix: format!("cluster_test_{}", i),
                ..Default::default()
            };

            // Create transport with shared state
            let transport =
                MockTransport::new_with_shared_state(config.clone(), shared_state.clone());
            let adapter = HorizontalAdapterBase::new(config).await.unwrap();

            node_ids.push(adapter.node_id.clone());
            transports.push(transport.clone());
            nodes.push(adapter);
        }

        Self {
            nodes,
            _node_ids: node_ids,
            _transports: transports,
        }
    }

    /// Start all nodes' listeners
    pub async fn start_all(&self) {
        for node in &self.nodes {
            node.start_listeners().await.unwrap();
        }

        // Give nodes time to establish connections
        sleep(Duration::from_millis(100)).await;
    }

    /// Simulate a node going dead by stopping its heartbeat
    pub async fn _kill_node(&mut self, node_index: usize) {
        if node_index < self._transports.len() {
            self._transports[node_index].set_health_status(false).await;
        }
    }

    /// Get a specific node
    pub fn _get_node(&self, index: usize) -> Option<&HorizontalAdapterBase<MockTransport>> {
        self.nodes.get(index)
    }

    /// Configure cluster health on all nodes
    pub async fn configure_cluster_health(&mut self, config: ClusterHealthConfig) {
        for node in &mut self.nodes {
            node.set_cluster_health(&config).await.unwrap();
        }
    }
}

#[tokio::test]
async fn test_cluster_health_configuration_applied() {
    // This test verifies that cluster health configuration is properly applied to nodes
    // without relying on actual cross-node communication
    let mut cluster = MockCluster::new(3).await;

    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    cluster.configure_cluster_health(config.clone()).await;

    // Verify configuration was applied to each node
    for node in &cluster.nodes {
        assert!(node.cluster_health_enabled);
        assert_eq!(node.heartbeat_interval_ms, 100);
        assert_eq!(node.node_timeout_ms, 500);
        assert_eq!(node.cleanup_interval_ms, 200);
    }
}

#[tokio::test]
async fn test_dead_node_detection_logic() {
    // Test the dead node detection logic directly without relying on communication
    let adapter = HorizontalAdapterBase::<MockTransport>::new(MockConfig::default())
        .await
        .unwrap();

    let node_timeout_ms = 1000;

    // Manually add nodes to heartbeat tracking with specific timestamps
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;

        // Add a node that should be considered dead (old timestamp)
        let dead_time = std::time::Instant::now() - Duration::from_millis(node_timeout_ms + 100);
        heartbeats.insert("node-should-be-dead".to_string(), dead_time);

        // Add a node that should still be alive (recent timestamp)
        let alive_time = std::time::Instant::now() - Duration::from_millis(500);
        heartbeats.insert("node-should-be-alive".to_string(), alive_time);
    }

    // Test dead node detection
    let dead_nodes = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.get_dead_nodes(node_timeout_ms).await
    };

    // Verify exact results
    assert_eq!(dead_nodes.len(), 1);
    assert_eq!(dead_nodes[0], "node-should-be-dead");
}

#[tokio::test]
async fn test_leader_election_algorithm() {
    // Test the leader election algorithm directly without communication
    let adapter = HorizontalAdapterBase::<MockTransport>::new(MockConfig::default())
        .await
        .unwrap();

    let _our_node_id = adapter.node_id.clone();

    // Set up a deterministic scenario for leader election
    {
        let horizontal = adapter.horizontal.lock().await;
        let mut heartbeats = horizontal.node_heartbeats.write().await;

        // Add nodes with guaranteed alphabetical ordering
        // Use prefixes that are definitely before/after any UUID
        heartbeats.insert(
            "000-definitely-first".to_string(),
            std::time::Instant::now(),
        );
        heartbeats.insert("zzz-definitely-last".to_string(), std::time::Instant::now());
    }

    // Test 1: When no nodes are dead, the alphabetically first node should be leader
    let is_leader = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.is_cleanup_leader(&Vec::new()).await
    };
    assert!(
        !is_leader,
        "Should not be leader when '000-definitely-first' exists"
    );

    // Test 2: When the alphabetically first node is dead, we should become leader
    let dead_nodes = vec!["000-definitely-first".to_string()];
    let is_leader = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.is_cleanup_leader(&dead_nodes).await
    };
    // Our node_id (UUID) should now be alphabetically first among alive nodes
    // (since UUIDs start with hex digits, they come before 'zzz')
    assert!(
        is_leader,
        "Should be leader when '000-definitely-first' is dead"
    );
}

#[tokio::test]
async fn test_cluster_health_disabled_no_heartbeat() {
    let mut cluster = MockCluster::new(2).await;

    // Configure cluster health as DISABLED
    let config = ClusterHealthConfig {
        enabled: false,
        heartbeat_interval_ms: 100,
        node_timeout_ms: 500,
        cleanup_interval_ms: 200,
    };

    cluster.configure_cluster_health(config).await;
    cluster.start_all().await;

    // Wait a bit
    sleep(Duration::from_millis(300)).await;

    // Nodes should NOT be tracking each other when cluster health is disabled
    for node in &cluster.nodes {
        let heartbeats = {
            let horizontal = node.horizontal.lock().await;
            let heartbeats = horizontal.node_heartbeats.read().await;
            heartbeats.len()
        };

        // Should see 0 nodes since heartbeat system is disabled
        assert_eq!(
            heartbeats, 0,
            "Node should not track heartbeats when cluster health disabled"
        );
    }
}
