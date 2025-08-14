use crate::websocket::SocketId;
use std::time::Instant;

pub mod worker;

#[derive(Debug, Clone)]
pub struct DisconnectTask {
    pub socket_id: SocketId,
    pub app_id: String,
    pub subscribed_channels: Vec<String>,
    pub user_id: Option<String>,
    pub timestamp: Instant,
    pub connection_info: Option<ConnectionCleanupInfo>,
}

#[derive(Debug, Clone)]
pub struct ConnectionCleanupInfo {
    pub presence_channels: Vec<String>,
    pub client_events_enabled: bool,
    pub auth_info: Option<AuthInfo>,
}

#[derive(Debug, Clone)]
pub struct AuthInfo {
    pub user_id: String,
    pub user_info: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CleanupConfig {
    pub queue_buffer_size: usize,
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub worker_threads: WorkerThreadsConfig,
    pub max_retry_attempts: u32,
    pub async_enabled: bool,
    pub fallback_to_sync: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum WorkerThreadsConfig {
    Auto,
    Fixed(usize),
}

impl WorkerThreadsConfig {
    pub fn resolve(&self) -> usize {
        match self {
            WorkerThreadsConfig::Auto => {
                // Use 25% of available CPUs, minimum 1, maximum 4
                let cpu_count = num_cpus::get();
                let auto_threads = (cpu_count / 4).max(1).min(4);
                tracing::info!(
                    "Auto-detected {} CPUs, using {} cleanup worker threads",
                    cpu_count,
                    auto_threads
                );
                auto_threads
            }
            WorkerThreadsConfig::Fixed(n) => *n,
        }
    }
}

impl Default for CleanupConfig {
    fn default() -> Self {
        // Defaults optimized for 2vCPU/2GB RAM servers
        // Conservative settings to avoid overwhelming smaller instances
        Self {
            queue_buffer_size: 2000, // ~16KB at 8 bytes per pointer
            batch_size: 25,          // Process 25 disconnects at once
            batch_timeout_ms: 50,    // 50ms max wait for batch fill
            worker_threads: 1,       // Single worker (leave CPU for main server)
            max_retry_attempts: 2,   // Don't retry too much
            async_enabled: true,     // Enable by default
            fallback_to_sync: true,  // Safety fallback enabled
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebhookEvent {
    pub event_type: String,
    pub app_id: String,
    pub channel: String,
    pub user_id: Option<String>,
    pub data: serde_json::Value,
}
