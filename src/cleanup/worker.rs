use super::{CleanupConfig, ConnectionCleanupInfo, DisconnectTask, WebhookEvent};
use crate::adapter::connection_manager::ConnectionManager;
use crate::channel::manager::ChannelManager;
use crate::metrics::MetricsInterface;
use crate::webhook::integration::WebhookIntegration;
use crate::websocket::SocketId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub struct CleanupWorker {
    connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
    channel_manager: Arc<RwLock<ChannelManager>>,
    webhook_integration: Option<Arc<WebhookIntegration>>,
    metrics: Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>>,
    config: CleanupConfig,
}

impl CleanupWorker {
    pub fn new(
        connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        channel_manager: Arc<RwLock<ChannelManager>>,
        webhook_integration: Option<Arc<WebhookIntegration>>,
        metrics: Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>>,
        config: CleanupConfig,
    ) -> Self {
        Self {
            connection_manager,
            channel_manager,
            webhook_integration,
            metrics,
            config,
        }
    }

    pub async fn run(&self, mut receiver: mpsc::UnboundedReceiver<DisconnectTask>) {
        info!("Cleanup worker started with config: {:?}", self.config);

        let mut batch = Vec::with_capacity(self.config.batch_size);
        let mut last_batch_time = Instant::now();

        loop {
            tokio::select! {
                // Receive new disconnect tasks
                task = receiver.recv() => {
                    match task {
                        Some(task) => {
                            debug!("Received disconnect task for socket: {}", task.socket_id);
                            batch.push(task);

                            // Process batch if full or timeout reached
                            if batch.len() >= self.config.batch_size
                                || last_batch_time.elapsed() >= Duration::from_millis(self.config.batch_timeout_ms) {
                                self.process_batch(&mut batch).await;
                                last_batch_time = Instant::now();
                            }
                        }
                        None => {
                            // Channel closed, process remaining batch and exit
                            if !batch.is_empty() {
                                info!("Processing final batch of {} tasks before shutdown", batch.len());
                                self.process_batch(&mut batch).await;
                            }
                            info!("Cleanup worker shutting down");
                            break;
                        }
                    }
                }

                // Timeout to ensure batches don't wait too long
                _ = sleep(Duration::from_millis(self.config.batch_timeout_ms)) => {
                    if !batch.is_empty() && last_batch_time.elapsed() >= Duration::from_millis(self.config.batch_timeout_ms) {
                        debug!("Processing batch due to timeout: {} tasks", batch.len());
                        self.process_batch(&mut batch).await;
                        last_batch_time = Instant::now();
                    }
                }
            }
        }
    }

    async fn process_batch(&self, batch: &mut Vec<DisconnectTask>) {
        let batch_start = Instant::now();
        let batch_size = batch.len();

        debug!("Processing cleanup batch of {} tasks", batch_size);

        // Group by channels for efficient cleanup
        let mut channel_operations: HashMap<(String, String), Vec<SocketId>> = HashMap::new();
        let mut webhook_events = Vec::new();
        let mut connections_to_remove = Vec::new();

        // Prepare all operations without holding locks
        for task in batch.iter() {
            // Group channel operations by app_id and channel
            for channel in &task.subscribed_channels {
                channel_operations
                    .entry((task.app_id.clone(), channel.clone()))
                    .or_insert_with(Vec::new)
                    .push(task.socket_id.clone());
            }

            // Prepare connection removal
            connections_to_remove.push((task.socket_id.clone(), task.app_id.clone()));

            // Prepare webhook events without holding locks
            if let Some(info) = &task.connection_info {
                webhook_events.extend(self.prepare_webhook_events(task, info).await);
            }
        }

        // Execute batch operations
        self.batch_channel_cleanup(channel_operations).await;
        self.batch_connection_removal(connections_to_remove).await;

        // Process webhooks asynchronously (non-blocking)
        if !webhook_events.is_empty() {
            self.process_webhooks_async(webhook_events).await;
        }

        // Update metrics
        self.update_cleanup_metrics(batch_size, batch_start.elapsed())
            .await;

        batch.clear();

        debug!("Completed cleanup batch in {:?}", batch_start.elapsed());
    }

    async fn batch_channel_cleanup(
        &self,
        channel_operations: HashMap<(String, String), Vec<SocketId>>,
    ) {
        if channel_operations.is_empty() {
            return;
        }

        debug!(
            "Processing {} channel cleanup operations",
            channel_operations.len()
        );

        // Process each app's channels together
        let mut apps_channels: HashMap<String, Vec<(String, Vec<SocketId>)>> = HashMap::new();
        for ((app_id, channel), socket_ids) in channel_operations {
            apps_channels
                .entry(app_id)
                .or_insert_with(Vec::new)
                .push((channel, socket_ids));
        }

        for (app_id, channels) in apps_channels {
            if let Err(e) = self.process_app_channel_cleanup(&app_id, channels).await {
                error!("Failed to cleanup channels for app {}: {}", app_id, e);
            }
        }
    }

    async fn process_app_channel_cleanup(
        &self,
        app_id: &str,
        channels: Vec<(String, Vec<SocketId>)>,
    ) -> crate::error::Result<()> {
        let channel_manager = self.channel_manager.write().await;

        for (channel, socket_ids) in channels {
            for socket_id in socket_ids {
                if let Err(e) = channel_manager
                    .unsubscribe(&socket_id.0, &channel, app_id, None)
                    .await
                {
                    warn!(
                        "Failed to remove socket {} from channel {}: {}",
                        socket_id, channel, e
                    );
                }
            }
        }

        Ok(())
    }

    async fn batch_connection_removal(&self, connections: Vec<(SocketId, String)>) {
        if connections.is_empty() {
            return;
        }

        debug!("Removing {} connections", connections.len());

        let mut connection_manager = self.connection_manager.lock().await;

        for (socket_id, app_id) in connections {
            if let Err(e) = connection_manager
                .remove_connection(&socket_id, &app_id)
                .await
            {
                warn!("Failed to remove connection {}: {}", socket_id, e);
            }
        }
    }

    async fn prepare_webhook_events(
        &self,
        task: &DisconnectTask,
        info: &ConnectionCleanupInfo,
    ) -> Vec<WebhookEvent> {
        let mut events = Vec::new();

        // Generate member_removed events for presence channels
        for channel in &info.presence_channels {
            if let Some(user_id) = &task.user_id {
                events.push(WebhookEvent {
                    event_type: "member_removed".to_string(),
                    app_id: task.app_id.clone(),
                    channel: channel.clone(),
                    user_id: Some(user_id.clone()),
                    data: serde_json::json!({
                        "user_id": user_id,
                        "socket_id": task.socket_id.0
                    }),
                });
            }
        }

        // Generate channel_vacated events if needed
        // Note: This would require checking if channel is empty, which we'll do in the worker

        events
    }

    async fn process_webhooks_async(&self, events: Vec<WebhookEvent>) {
        if let Some(webhook_integration) = &self.webhook_integration {
            debug!("Processing {} webhook events", events.len());

            let webhook_tasks: Vec<_> = events
                .into_iter()
                .map(|event| {
                    let webhook = webhook_integration.clone();
                    tokio::spawn(async move {
                        // Note: This is a simplified webhook sending - actual implementation
                        // would need to match the existing webhook integration interface
                        debug!(
                            "Sending webhook event: {} for channel: {}",
                            event.event_type, event.channel
                        );
                        // webhook.send_event(event).await
                    })
                })
                .collect();

            // Let webhooks run in background - don't await to avoid blocking cleanup
            tokio::spawn(async move {
                for task in webhook_tasks {
                    if let Err(e) = task.await {
                        error!("Webhook task failed: {}", e);
                    }
                }
            });
        }
    }

    async fn update_cleanup_metrics(&self, batch_size: usize, duration: Duration) {
        if let Some(metrics) = &self.metrics {
            if let Ok(metrics_guard) = metrics.try_lock() {
                // Update cleanup metrics
                // Note: This would need to integrate with the existing metrics system
                debug!("Processed batch of {} in {:?}", batch_size, duration);
            }
        }
    }
}
