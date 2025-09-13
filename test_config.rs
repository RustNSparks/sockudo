use sockudo::options::ClusterHealthConfig;

fn main() {
    let config = ClusterHealthConfig {
        enabled: true,
        heartbeat_interval_ms: 5000,
        node_timeout_ms: 15000,
        cleanup_interval_ms: 7000,
    };
    
    println!("Config validation result: {:?}", config.validate());
    
    // Check the ratio: 5000 < 15000/3 = 5000? No, they're equal! 
    println!("heartbeat {} < node_timeout/3 {}: {}", 
        config.heartbeat_interval_ms, 
        config.node_timeout_ms / 3, 
        config.heartbeat_interval_ms < config.node_timeout_ms / 3);
}
