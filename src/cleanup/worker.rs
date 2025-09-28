use super::{CleanupConfig, ConnectionCleanupInfo, DisconnectTask, WebhookEvent};
use crate::adapter::connection_manager::ConnectionManager;
use crate::app::manager::AppManager;
use crate::channel::manager::ChannelManager;
use crate::presence::PresenceManager;
use crate::webhook::integration::WebhookIntegration;
use crate::websocket::SocketId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub struct CleanupWorker {
    connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
    app_manager: Arc<dyn AppManager + Send + Sync>,
    webhook_integration: Option<Arc<WebhookIntegration>>,
    config: CleanupConfig,
}

impl CleanupWorker {
    pub fn new(
        connection_manager: Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        app_manager: Arc<dyn AppManager + Send + Sync>,
        webhook_integration: Option<Arc<WebhookIntegration>>,
        config: CleanupConfig,
    ) -> Self {
        Self {
            connection_manager,
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

            // Prepare connection removal with user_id for user socket cleanup
            connections_to_remove.push((
                task.socket_id.clone(),
                task.app_id.clone(),
                task.user_id.clone(),
            ));
        }

        // Execute batch operations
        self.batch_channel_cleanup(channel_operations).await;
        self.batch_connection_removal(connections_to_remove).await;

        // Prepare webhook events AFTER channel cleanup to correctly detect empty channels
        for task in batch.iter() {
            webhook_events.extend(
                self.prepare_webhook_events(task, task.connection_info.as_ref())
                    .await,
            );
        }

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

        let mut total_success = 0;
        let mut total_errors = 0;

        // Group operations by app_id for batch processing
        let mut operations_by_app: HashMap<String, Vec<(String, String, String)>> = HashMap::new();

        for ((app_id, channel), socket_ids) in channel_operations {
            for socket_id in socket_ids {
                operations_by_app.entry(app_id.clone()).or_default().push((
                    socket_id.0.clone(),
                    channel.clone(),
                    app_id.clone(),
                ));
            }
        }

        // Process batch operations per app to minimize lock contention
        for (app_id, operations) in operations_by_app {
            debug!(
                "Processing batch unsubscribe for app {} with {} operations",
                app_id,
                operations.len()
            );

            match ChannelManager::batch_unsubscribe(&self.connection_manager, operations).await {
                Ok(results) => {
                    for result in results {
                        match result {
                            Ok(_) => total_success += 1,
                            Err(e) => {
                                total_errors += 1;
                                warn!(
                                    "Batch unsubscribe operation failed for app {}: {}",
                                    app_id, e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Batch unsubscribe failed for app {}: {}", app_id, e);
                    total_errors += 1;
                }
            }
        }

        debug!(
            "Individual channel cleanup completed: {} successful, {} errors",
            total_success, total_errors
        );
    }

    async fn batch_connection_removal(&self, connections: Vec<(SocketId, String, Option<String>)>) {
        if connections.is_empty() {
            return;
        }

        debug!("Removing {} connections", connections.len());

        // Process each connection removal with minimal lock duration
        for (socket_id, app_id, user_id) in connections {
            // Acquire lock for each individual removal to minimize contention
            let result = {
                let mut connection_manager = self.connection_manager.lock().await;
                // First remove the connection
                let remove_result = connection_manager
                    .remove_connection(&socket_id, &app_id)
                    .await;

                // Then remove from user mapping if user_id exists
                if let Some(ref uid) = user_id
                    && let Err(e) = connection_manager
                        .remove_user_socket(uid, &socket_id, &app_id)
                        .await
                {
                    warn!(
                        "Failed to remove user socket mapping for {}: {}",
                        socket_id, e
                    );
                }

                remove_result
            }; // Lock released here after each removal

            if let Err(e) = result {
                warn!("Failed to remove connection {}: {}", socket_id, e);
            }
        }
    }

    async fn prepare_webhook_events(
        &self,
        task: &DisconnectTask,
        info: Option<&ConnectionCleanupInfo>,
    ) -> Vec<WebhookEvent> {
        let mut events = Vec::new();

        // Generate member_removed events for presence channels (only if we have connection info)
        if let Some(info) = info
            && let Some(user_id) = &task.user_id
        {
            for channel in &info.presence_channels {
                events.push(WebhookEvent {
                    event_type: "member_removed".to_string(),
                    app_id: task.app_id.clone(),
                    channel: channel.clone(),
                    user_id: Some(user_id.clone()),
                    data: sonic_rs::json!({
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
                    data: sonic_rs::json!({
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

            // Collect webhook task handles for error tracking
            let mut webhook_handles = Vec::new();

            for (app_id, events) in events_by_app {
                let webhook_integration = webhook_integration.clone();
                let app_manager = self.app_manager.clone();

                let connection_manager = self.connection_manager.clone();
                let handle = tokio::spawn(async move {
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
                        if let Err(e) = Self::send_webhook_event(
                            &connection_manager,
                            &webhook_integration,
                            &app_config,
                            &event,
                        )
                        .await
                        {
                            error!("Failed to send webhook event {}: {}", event.event_type, e);
                        }
                    }
                });

                webhook_handles.push(handle);
            }

            // Wait for all webhook tasks to complete and log any failures
            for handle in webhook_handles {
                if let Err(e) = handle.await {
                    error!("Webhook task panicked or was cancelled: {}", e);
                }
            }
        }
    }

    async fn send_webhook_event(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        webhook_integration: &Arc<WebhookIntegration>,
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
                    // Use centralized presence member removal logic (handles both webhook and broadcast)
                    PresenceManager::handle_member_removed(
                        connection_manager,
                        Some(webhook_integration),
                        app_config,
                        &event.channel,
                        user_id,
                        None, // No socket to exclude in cleanup (connection already gone)
                    )
                    .await?;
                    debug!(
                        "Processed centralized member_removed for user {} in channel {}",
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
