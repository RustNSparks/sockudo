use super::{CleanupConfig, ConnectionCleanupInfo, DisconnectTask, WebhookEvent};
use crate::adapter::connection_manager::ConnectionManager;
use crate::app::manager::AppManager;
use crate::channel::manager::ChannelManager;
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
    #[allow(dead_code)]
    channel_manager: Arc<RwLock<ChannelManager>>,
    app_manager: Arc<dyn AppManager + Send + Sync>,
    webhook_integration: Option<Arc<WebhookIntegration>>,
    config: CleanupConfig,
}

impl CleanupWorker {
    pub fn new(
        connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        channel_manager: Arc<RwLock<ChannelManager>>,
        app_manager: Arc<dyn AppManager + Send + Sync>,
        webhook_integration: Option<Arc<WebhookIntegration>>,
        config: CleanupConfig,
    ) -> Self {
        Self {
            connection_manager,
            channel_manager,
            app_manager,
            webhook_integration,
            config,
        }
    }

    pub fn get_config(&self) -> &CleanupConfig {
        &self.config
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<DisconnectTask>) {
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
                    .or_default()
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

        let mut total_removed = 0;
        let mut total_channels_processed = 0;

        // Process each channel with batch removal - single lock per channel
        for ((app_id, channel), socket_ids) in channel_operations {
            let socket_count = socket_ids.len();
            debug!(
                "Batch removing {} sockets from channel {}/{}",
                socket_count, app_id, channel
            );

            // Single lock acquisition for all sockets in this channel
            let mut connection_manager = self.connection_manager.lock().await;
            match connection_manager
                .batch_remove_from_channel(&app_id, &channel, &socket_ids)
                .await
            {
                Ok(removed_count) => {
                    total_removed += removed_count;
                    total_channels_processed += 1;
                    if removed_count < socket_count {
                        debug!(
                            "Channel {}/{}: removed {}/{} sockets (some may have been already removed)",
                            app_id, channel, removed_count, socket_count
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to batch remove {} sockets from channel {}/{}: {}",
                        socket_count, app_id, channel, e
                    );
                }
            }
            // Lock released here before processing next channel
        }

        debug!(
            "Batch channel cleanup completed: {} sockets removed from {} channels",
            total_removed, total_channels_processed
        );
    }

    async fn batch_connection_removal(&self, connections: Vec<(SocketId, String)>) {
        if connections.is_empty() {
            return;
        }

        debug!("Removing {} connections", connections.len());

        // Process each connection removal with minimal lock duration
        for (socket_id, app_id) in connections {
            // Acquire lock for each individual removal to minimize contention
            let result = {
                let mut connection_manager = self.connection_manager.lock().await;
                connection_manager
                    .remove_connection(&socket_id, &app_id)
                    .await
            }; // Lock released here after each removal

            if let Err(e) = result {
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

        // Check if channels are now empty for channel_vacated events
        // IMPORTANT: Acquire and release lock for each channel to minimize lock contention
        for channel in &task.subscribed_channels {
            // Acquire lock for minimal duration - just for this single channel check
            let socket_count = {
                let mut connection_manager = self.connection_manager.lock().await;
                connection_manager
                    .get_channel_socket_count(&task.app_id, channel)
                    .await
            }; // Lock released here immediately after getting the count

            if socket_count == 0 {
                events.push(WebhookEvent {
                    event_type: "channel_vacated".to_string(),
                    app_id: task.app_id.clone(),
                    channel: channel.clone(),
                    user_id: None,
                    data: serde_json::json!({
                        "channel": channel
                    }),
                });
            }
        }

        events
    }

    async fn process_webhooks_async(&self, webhook_events: Vec<WebhookEvent>) {
        if let Some(webhook_integration) = &self.webhook_integration {
            debug!("Processing {} webhook events", webhook_events.len());

            // Group events by app_id to get app configs efficiently
            let mut events_by_app: HashMap<String, Vec<WebhookEvent>> = HashMap::new();
            for event in webhook_events {
                events_by_app
                    .entry(event.app_id.clone())
                    .or_default()
                    .push(event);
            }

            for (app_id, events) in events_by_app {
                let webhook_integration = webhook_integration.clone();
                let app_manager = self.app_manager.clone();

                tokio::spawn(async move {
                    // Get app config once for all events
                    let app_config = match app_manager.find_by_id(&app_id).await {
                        Ok(Some(app)) => app,
                        Ok(None) => {
                            warn!("App {} not found for webhook events", app_id);
                            return;
                        }
                        Err(e) => {
                            error!("Failed to get app config for {}: {}", app_id, e);
                            return;
                        }
                    };

                    // Process all events for this app
                    for event in events {
                        if let Err(e) =
                            Self::send_webhook_event(&webhook_integration, &app_config, &event)
                                .await
                        {
                            error!("Failed to send webhook event {}: {}", event.event_type, e);
                        }
                    }
                });
            }
        }
    }

    async fn send_webhook_event(
        webhook_integration: &WebhookIntegration,
        app_config: &crate::app::config::App,
        event: &WebhookEvent,
    ) -> crate::error::Result<()> {
        debug!(
            "Sending {} webhook for app {} channel {}",
            event.event_type, app_config.id, event.channel
        );

        match event.event_type.as_str() {
            "member_removed" => {
                if let Some(user_id) = &event.user_id {
                    webhook_integration
                        .send_member_removed(app_config, &event.channel, user_id)
                        .await?;
                    debug!(
                        "Sent member_removed webhook for user {} in channel {}",
                        user_id, event.channel
                    );
                }
            }
            "channel_vacated" => {
                webhook_integration
                    .send_channel_vacated(app_config, &event.channel)
                    .await?;
                debug!("Sent channel_vacated webhook for channel {}", event.channel);
            }
            _ => {
                warn!("Unknown webhook event type: {}", event.event_type);
            }
        }

        Ok(())
    }
}
