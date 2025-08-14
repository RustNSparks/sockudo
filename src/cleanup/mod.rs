use crate::websocket::SocketId;
use std::time::Instant;

pub mod webhooks;
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

#[derive(Debug, Clone)]
pub struct CleanupConfig {
    pub queue_buffer_size: usize,
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub worker_threads: usize,
    pub max_retry_attempts: u32,
    pub async_enabled: bool,
    pub fallback_to_sync: bool,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            queue_buffer_size: 1000,
            batch_size: 50,
            batch_timeout_ms: 100,
            worker_threads: 2,
            max_retry_attempts: 3,
            async_enabled: true,
            fallback_to_sync: true,
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
