use crate::websocket::SocketId;
use std::time::Instant;
use tokio::sync::mpsc;

pub mod multi_worker;
pub mod worker;

/// Unified cleanup sender that abstracts over single vs multi-worker implementations
#[derive(Clone)]
pub enum CleanupSender {
    /// Direct sender for single worker (optimized path)
    Direct(mpsc::Sender<DisconnectTask>),
    /// Multi-worker sender with round-robin distribution
    Multi(multi_worker::MultiWorkerSender),
}

impl CleanupSender {
    /// Send a disconnect task to the cleanup system
    pub fn try_send(
        &self,
        task: DisconnectTask,
    ) -> Result<(), Box<mpsc::error::TrySendError<DisconnectTask>>> {
        match self {
            CleanupSender::Direct(sender) => sender.try_send(task).map_err(Box::new),
            CleanupSender::Multi(sender) => {
                // Convert MultiWorkerSender's SendError to TrySendError
                sender.send(task).map_err(|e| {
                    // MultiWorkerSender returns SendError when all queues are full or closed
                    // We treat this as "Full" for backpressure handling
                    Box::new(mpsc::error::TrySendError::Full(e.0))
                })
            }
        }
    }

    /// Check if the sender is still operational
    pub fn is_closed(&self) -> bool {
        match self {
            CleanupSender::Direct(sender) => sender.is_closed(),
            CleanupSender::Multi(sender) => !sender.is_available(),
        }
    }
}

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

impl CleanupConfig {
    /// Validate the configuration values
    pub fn validate(&self) -> Result<(), String> {
        if self.queue_buffer_size == 0 {
            return Err("queue_buffer_size must be greater than 0".to_string());
        }

        if self.batch_size == 0 {
            return Err("batch_size must be greater than 0".to_string());
        }

        if self.batch_timeout_ms == 0 {
            return Err("batch_timeout_ms must be greater than 0".to_string());
        }

        if let WorkerThreadsConfig::Fixed(n) = self.worker_threads
            && n == 0
        {
            return Err("worker_threads must be greater than 0 when using fixed count".to_string());
        }

        // Warn if potentially problematic configurations
        if self.queue_buffer_size < self.batch_size {
            return Err(format!(
                "queue_buffer_size ({}) should be at least as large as batch_size ({})",
                self.queue_buffer_size, self.batch_size
            ));
        }

        if self.batch_timeout_ms > 60000 {
            return Err(format!(
                "batch_timeout_ms ({}) is unusually high (> 60 seconds), this may cause delays",
                self.batch_timeout_ms
            ));
        }

        if !self.async_enabled && !self.fallback_to_sync {
            return Err("Either async_enabled or fallback_to_sync must be true".to_string());
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum WorkerThreadsConfig {
    Auto,
    Fixed(usize),
}

impl serde::Serialize for WorkerThreadsConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            WorkerThreadsConfig::Auto => serializer.serialize_str("auto"),
            WorkerThreadsConfig::Fixed(n) => serializer.serialize_u64(*n as u64),
        }
    }
}

impl<'de> serde::Deserialize<'de> for WorkerThreadsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Visitor};

        struct WorkerThreadsVisitor;

        impl<'de> Visitor<'de> for WorkerThreadsVisitor {
            type Value = WorkerThreadsConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a positive integer or the string \"auto\"")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value.to_lowercase().as_str() {
                    "auto" => Ok(WorkerThreadsConfig::Auto),
                    _ => {
                        // Try to parse as number
                        match value.parse::<usize>() {
                            Ok(n) if n > 0 => Ok(WorkerThreadsConfig::Fixed(n)),
                            Ok(_) => {
                                Err(de::Error::custom("worker_threads must be greater than 0"))
                            }
                            Err(_) => Err(de::Error::custom(format!(
                                "expected \"auto\" or positive integer, got \"{}\"",
                                value
                            ))),
                        }
                    }
                }
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > 0 && value <= usize::MAX as u64 {
                    Ok(WorkerThreadsConfig::Fixed(value as usize))
                } else if value == 0 {
                    Err(de::Error::custom("worker_threads must be greater than 0"))
                } else {
                    Err(de::Error::custom("worker_threads value too large"))
                }
            }
        }

        deserializer.deserialize_any(WorkerThreadsVisitor)
    }
}

impl WorkerThreadsConfig {
    pub fn resolve(&self) -> usize {
        match self {
            WorkerThreadsConfig::Auto => {
                // Use 25% of available CPUs, minimum 1, maximum 4
                let cpu_count = num_cpus::get();
                let auto_threads = (cpu_count / 4).clamp(1, 4);
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
            queue_buffer_size: 50000, // ~30MB (each DisconnectTask ~625 bytes)
            batch_size: 25,           // Process 25 disconnects at once
            batch_timeout_ms: 50,     // 50ms max wait for batch fill
            worker_threads: WorkerThreadsConfig::Auto, // Auto-detect based on CPU count (recommended)
            max_retry_attempts: 2,                     // Don't retry too much
            async_enabled: true,                       // Enable by default
            fallback_to_sync: true,                    // Safety fallback enabled
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebhookEvent {
    pub event_type: String,
    pub app_id: String,
    pub channel: String,
    pub user_id: Option<String>,
    pub data: sonic_rs::Value,
}
