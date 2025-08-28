// src/adapter/handler/core_methods.rs
use super::ConnectionHandler;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::protocol::messages::{ErrorData, MessageData, PusherMessage};
use crate::websocket::SocketId;
use serde_json::Value;
use std::collections::HashSet;
use tracing::{debug, error, info, warn};

impl ConnectionHandler {
    pub async fn send_connection_established(
        &self,
        app_id: &str,
        socket_id: &SocketId,
    ) -> Result<()> {
        let connection_message =
            PusherMessage::connection_established(socket_id.as_ref().to_string());
        self.send_message_to_socket(app_id, socket_id, connection_message)
            .await
    }

    pub async fn send_error(
        &self,
        app_id: &str,
        socket_id: &SocketId,
        error: &Error,
        channel: Option<String>,
    ) -> Result<()> {
        let error_data = ErrorData {
            message: error.to_string(),
            code: Some(error.close_code()),
        };
        let error_message =
            PusherMessage::error(error_data.code.unwrap_or(4000), error_data.message, channel);
        self.send_message_to_socket(app_id, socket_id, error_message)
            .await
    }

    pub async fn handle_unsubscribe(
        &self,
        socket_id: &SocketId,
        message: &PusherMessage,
        app_config: &App,
    ) -> Result<()> {
        // Extract channel name from message
        let channel_name = self.extract_channel_from_unsubscribe_message(message)?;

        // Get user ID before unsubscribing (for presence channels)
        let user_id = self.get_user_id_for_socket(socket_id, app_config).await?;

        // Perform unsubscription through channel manager
        {
            let channel_manager = self.channel_manager.write().await;
            channel_manager
                .unsubscribe(
                    socket_id.as_ref(),
                    &channel_name,
                    &app_config.id,
                    user_id.as_deref(),
                )
                .await?;
        }

        // Update connection state
        self.update_connection_unsubscribe_state(socket_id, app_config, &channel_name)
            .await?;

        // Get current subscription count after unsubscribe
        let current_sub_count = self
            .connection_manager
            .lock()
            .await
            .get_channel_socket_count(&app_config.id, &channel_name)
            .await;

        // Track unsubscription metrics
        if let Some(ref metrics) = self.metrics {
            let channel_type = crate::channel::ChannelType::from_name(&channel_name);
            let channel_type_str = channel_type.as_str();

            // Mark unsubscription metric
            {
                let metrics_locked = metrics.lock().await;
                metrics_locked.mark_channel_unsubscription(&app_config.id, channel_type_str);
            }

            // Update active channel count if this was the last connection to the channel
            if current_sub_count == 0 {
                // Channel became inactive - decrement the count for this channel type
                // Pass the Arc directly to avoid holding any locks
                self.decrement_active_channel_count(
                    &app_config.id,
                    channel_type_str,
                    metrics.clone(),
                )
                .await;
            }
        }

        // Handle presence channel member removal
        if channel_name.starts_with("presence-") {
            if let Some(user_id_str) = user_id {
                let has_other_connections = self
                    .user_has_other_connections_in_presence_channel(
                        &app_config.id,
                        &channel_name,
                        &user_id_str,
                    )
                    .await?;

                if !has_other_connections {
                    // Send member_removed webhook
                    if let Some(webhook_integration) = &self.webhook_integration {
                        webhook_integration
                            .send_member_removed(app_config, &channel_name, &user_id_str)
                            .await
                            .ok();
                    }

                    // Send member_removed event to channel
                    let member_removed_msg =
                        PusherMessage::member_removed(channel_name.clone(), user_id_str);
                    self.broadcast_to_channel(
                        app_config,
                        &channel_name,
                        member_removed_msg,
                        Some(socket_id),
                    )
                    .await?;
                }
            }
        } else {
            // Send subscription count webhook for non-presence channels
            if let Some(webhook_integration) = &self.webhook_integration {
                webhook_integration
                    .send_subscription_count_changed(app_config, &channel_name, current_sub_count)
                    .await
                    .ok();
            }
        }

        // Send channel_vacated webhook if no subscribers left
        if current_sub_count == 0
            && let Some(webhook_integration) = &self.webhook_integration
        {
            webhook_integration
                .send_channel_vacated(app_config, &channel_name)
                .await
                .ok();
        }

        Ok(())
    }

    pub async fn handle_disconnect(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        debug!("Handling disconnect for socket: {}", socket_id);

        // Check if already disconnecting and set flag atomically
        // Release connection_manager lock quickly to reduce contention
        let conn = {
            let mut connection_manager = self.connection_manager.lock().await;
            connection_manager.get_connection(socket_id, app_id).await
        };

        let already_disconnecting = if let Some(conn) = conn {
            // Use try_lock to avoid blocking on nested locks (prevents deadlock)
            if let Ok(mut conn_locked) = conn.inner.try_lock() {
                let was_disconnecting = conn_locked.state.disconnecting;
                conn_locked.state.disconnecting = true;
                was_disconnecting
            } else {
                // Connection is busy being accessed by another thread - assume it's being handled
                debug!(
                    "Connection {} is busy, assuming disconnect already in progress",
                    socket_id
                );
                true // Skip processing
            }
        } else {
            // Connection doesn't exist - it may have been cleaned up already
            true
        };

        if already_disconnecting {
            debug!(
                "Connection {} already disconnecting or doesn't exist, skipping",
                socket_id
            );
            return Ok(());
        }

        // Clear timeouts
        self.clear_activity_timeout(app_id, socket_id).await?;
        self.clear_user_authentication_timeout(app_id, socket_id)
            .await?;

        // Clean up client event rate limiter
        if self.client_event_limiters.remove(socket_id).is_some() {
            debug!(
                "Removed client event rate limiter for socket: {}",
                socket_id
            );
        }

        // Get app configuration
        let app_config = match self.app_manager.find_by_id(app_id).await? {
            Some(app) => app,
            None => {
                error!("App not found during disconnect: {}", app_id);
                // Attempt cleanup even if app is gone
                self.cleanup_connection_from_manager(socket_id, app_id)
                    .await;
                return Err(Error::ApplicationNotFound);
            }
        };

        // Extract connection state before cleanup
        let (subscribed_channels, user_id, user_watchlist) = self
            .extract_connection_state_for_disconnect(socket_id, &app_config)
            .await?;

        // Process channel unsubscriptions
        if !subscribed_channels.is_empty() {
            self.process_channel_unsubscriptions_on_disconnect(
                socket_id,
                &app_config,
                &subscribed_channels,
                &user_id,
            )
            .await?;
        }

        // Handle watchlist offline events
        if let Some(ref user_id_str) = user_id {
            self.handle_disconnect_watchlist_events(
                &app_config,
                user_id_str,
                socket_id,
                user_watchlist,
            )
            .await?;
        }

        // Final cleanup from connection manager
        self.cleanup_connection_from_manager(socket_id, app_id)
            .await;

        // Update metrics
        if let Some(ref metrics) = self.metrics {
            let metrics_locked = metrics.lock().await;
            metrics_locked.mark_disconnection(app_id, socket_id);
        }

        debug!(
            "Successfully processed disconnect for socket: {}",
            socket_id
        );
        Ok(())
    }

    // Helper methods for the main disconnect handler
    async fn extract_connection_state_for_disconnect(
        &self,
        socket_id: &SocketId,
        app_config: &App,
    ) -> Result<(HashSet<String>, Option<String>, Option<Vec<String>>)> {
        let mut connection_manager = self.connection_manager.lock().await;
        match connection_manager
            .get_connection(socket_id, &app_config.id)
            .await
        {
            Some(conn_arc) => {
                let mut conn_locked = conn_arc.inner.lock().await;

                // Cancel any active timeouts
                conn_locked.state.timeouts.clear_all();

                let watchlist = conn_locked
                    .state
                    .user_info
                    .as_ref()
                    .and_then(|ui| ui.watchlist.clone());

                Ok((
                    conn_locked.state.subscribed_channels.clone(),
                    conn_locked.state.user_id.clone(),
                    watchlist,
                ))
            }
            None => {
                warn!(
                    "No connection found for socket during disconnect: {}",
                    socket_id
                );
                Ok((HashSet::new(), None, None))
            }
        }
    }

    async fn process_channel_unsubscriptions_on_disconnect(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        subscribed_channels: &HashSet<String>,
        user_id: &Option<String>,
    ) -> Result<()> {
        let channel_manager = self.channel_manager.write().await;

        for channel_str in subscribed_channels {
            debug!(
                "Processing channel {} for disconnect of socket {}",
                channel_str, socket_id
            );

            match channel_manager
                .unsubscribe(
                    socket_id.as_ref(),
                    channel_str,
                    &app_config.id,
                    user_id.as_deref(),
                )
                .await
            {
                Ok(_) => {
                    let current_sub_count = self
                        .connection_manager
                        .lock()
                        .await
                        .get_channel_socket_count(&app_config.id, channel_str)
                        .await;

                    self.handle_post_unsubscribe_webhooks(
                        app_config,
                        channel_str,
                        user_id,
                        current_sub_count,
                        socket_id,
                    )
                    .await?;
                }
                Err(e) => {
                    error!(
                        "Error unsubscribing socket {} from channel {} during disconnect: {}",
                        socket_id, channel_str, e
                    );
                }
            }
        }

        Ok(())
    }

    async fn handle_post_unsubscribe_webhooks(
        &self,
        app_config: &App,
        channel_str: &str,
        user_id: &Option<String>,
        current_sub_count: usize,
        socket_id: &SocketId,
    ) -> Result<()> {
        if channel_str.starts_with("presence-") {
            if let Some(disconnected_user_id) = user_id {
                let has_other_connections = self
                    .user_has_other_connections_in_presence_channel(
                        &app_config.id,
                        channel_str,
                        disconnected_user_id,
                    )
                    .await?;

                if !has_other_connections {
                    // Send member_removed webhook
                    if let Some(webhook_integration) = &self.webhook_integration {
                        webhook_integration
                            .send_member_removed(app_config, channel_str, disconnected_user_id)
                            .await
                            .ok();
                    }

                    // Send member_removed event to channel
                    let member_removed_msg = PusherMessage::member_removed(
                        channel_str.to_string(),
                        disconnected_user_id.clone(),
                    );
                    self.broadcast_to_channel(
                        app_config,
                        channel_str,
                        member_removed_msg,
                        Some(socket_id),
                    )
                    .await
                    .ok();
                }
            }
        } else {
            // Send subscription count webhook for non-presence channels
            if let Some(webhook_integration) = &self.webhook_integration {
                webhook_integration
                    .send_subscription_count_changed(app_config, channel_str, current_sub_count)
                    .await
                    .ok();
            }
        }

        // Send channel_vacated webhook if no subscribers left
        if current_sub_count == 0
            && let Some(webhook_integration) = &self.webhook_integration
        {
            webhook_integration
                .send_channel_vacated(app_config, channel_str)
                .await
                .ok();
        }

        Ok(())
    }

    async fn handle_disconnect_watchlist_events(
        &self,
        app_config: &App,
        user_id_str: &str,
        socket_id: &SocketId,
        user_watchlist: Option<Vec<String>>,
    ) -> Result<()> {
        if app_config.enable_watchlist_events.unwrap_or(false) && user_watchlist.is_some() {
            info!(
                "Processing watchlist disconnect for user {} on socket {}",
                user_id_str, socket_id
            );

            // Remove user connection from watchlist manager
            let offline_events = self
                .watchlist_manager
                .remove_user_connection(&app_config.id, user_id_str, socket_id)
                .await?;

            // Send offline events to watchers if user went offline
            if !offline_events.is_empty() {
                let watchers_to_notify = self
                    .get_watchers_for_user(&app_config.id, user_id_str)
                    .await?;

                for event in offline_events {
                    for watcher_socket_id in &watchers_to_notify {
                        if let Err(e) = self
                            .send_message_to_socket(
                                &app_config.id,
                                watcher_socket_id,
                                event.clone(),
                            )
                            .await
                        {
                            warn!(
                                "Failed to send offline notification to watcher {}: {}",
                                watcher_socket_id, e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn cleanup_connection_from_manager(&self, socket_id: &SocketId, app_id: &str) {
        let mut connection_manager = self.connection_manager.lock().await;

        // Cleanup connection resources
        if let Some(conn_to_cleanup) = connection_manager.get_connection(socket_id, app_id).await {
            connection_manager
                .cleanup_connection(app_id, conn_to_cleanup)
                .await;
        }

        // Remove connection from primary tracking
        connection_manager
            .remove_connection(socket_id, app_id)
            .await
            .ok();
    }

    // Helper methods for extracting data from messages
    fn extract_channel_from_unsubscribe_message(&self, message: &PusherMessage) -> Result<String> {
        let message_data = message.data.as_ref().ok_or_else(|| {
            Error::InvalidMessageFormat("Missing data in unsubscribe message".into())
        })?;

        match message_data {
            MessageData::String(channel_str) => Ok(channel_str.clone()),
            MessageData::Json(data) => data
                .get("channel")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    Error::InvalidMessageFormat("Missing channel in unsubscribe message".into())
                }),
            MessageData::Structured { channel, .. } => {
                channel.as_ref().map(|s| s.to_string()).ok_or_else(|| {
                    Error::InvalidMessageFormat("Missing channel in unsubscribe message".into())
                })
            }
        }
    }

    async fn get_user_id_for_socket(
        &self,
        socket_id: &SocketId,
        app_config: &App,
    ) -> Result<Option<String>> {
        let mut connection_manager = self.connection_manager.lock().await;
        if let Some(conn) = connection_manager
            .get_connection(socket_id, &app_config.id)
            .await
        {
            let conn_locked = conn.inner.lock().await;
            Ok(conn_locked.state.user_id.clone())
        } else {
            Ok(None)
        }
    }

    async fn update_connection_unsubscribe_state(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        channel_name: &str,
    ) -> Result<()> {
        let mut connection_manager = self.connection_manager.lock().await;
        if let Some(conn_arc) = connection_manager
            .get_connection(socket_id, &app_config.id)
            .await
        {
            let mut conn_locked = conn_arc.inner.lock().await;
            conn_locked.unsubscribe_from_channel(channel_name);

            // Remove presence info if it's a presence channel
            if channel_name.starts_with("presence-") {
                conn_locked.remove_presence_info(channel_name);
            }
        }
        Ok(())
    }

    /// Helper to check if a user has any other connections to a specific presence channel.
    async fn user_has_other_connections_in_presence_channel(
        &self,
        app_id: &str,
        channel_name: &str,
        user_id: &str,
    ) -> Result<bool> {
        let mut connection_manager = self.connection_manager.lock().await;
        let user_sockets = connection_manager.get_user_sockets(user_id, app_id).await?;

        for ws_ref in user_sockets.iter() {
            let socket_state_guard = ws_ref.inner.lock().await;
            if socket_state_guard.state.is_subscribed(channel_name) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn send_missed_cache_if_exists(
        &self,
        app_id: &str,
        socket_id: &SocketId,
        channel: &str,
    ) -> Result<()> {
        let mut cache_manager = self.cache_manager.lock().await;
        let cache_key = format!("app:{app_id}:channel:{channel}:cache_miss");

        match cache_manager.get(&cache_key).await {
            Ok(Some(cache_content)) => {
                // Found cached content, send it to the socket
                let cache_message: PusherMessage =
                    serde_json::from_str(&cache_content).map_err(|e| {
                        Error::InvalidMessageFormat(format!("Invalid cached message format: {e}"))
                    })?;

                self.send_message_to_socket(app_id, socket_id, cache_message)
                    .await?;
                info!(
                    "Sent cached content to socket {} for channel {}",
                    socket_id, channel
                );
            }
            Ok(None) => {
                // No cache found, send cache miss event
                let cache_miss_message = PusherMessage {
                    channel: Some(channel.to_string()),
                    name: None,
                    event: Some("pusher:cache_miss".to_string()),
                    data: None,
                };

                self.send_message_to_socket(app_id, socket_id, cache_miss_message)
                    .await?;

                // Send cache miss webhook if configured
                if let Some(app_config) = self.app_manager.find_by_id(app_id).await?
                    && let Some(webhook_integration) = &self.webhook_integration
                    && let Err(e) = webhook_integration
                        .send_cache_missed(&app_config, channel)
                        .await
                {
                    warn!(
                        "Failed to send cache_missed webhook for channel {}: {}",
                        channel, e
                    );
                }

                info!(
                    "No cached content found for channel: {}, sent cache_miss event",
                    channel
                );
            }
            Err(e) => {
                error!("Failed to get cache for channel {}: {}", channel, e);

                // Send cache miss event as fallback
                let cache_miss_message = PusherMessage {
                    channel: Some(channel.to_string()),
                    name: None,
                    event: Some("pusher:cache_miss".to_string()),
                    data: None,
                };

                self.send_message_to_socket(app_id, socket_id, cache_miss_message)
                    .await?;

                return Err(Error::Internal(format!(
                    "Cache retrieval failed for channel {channel}: {e}"
                )));
            }
        }

        Ok(())
    }

    /// Store a message in cache for a channel
    pub async fn store_cache_for_channel(
        &self,
        app_id: &str,
        channel: &str,
        message: &PusherMessage,
        ttl_seconds: Option<u64>,
    ) -> Result<()> {
        let mut cache_manager = self.cache_manager.lock().await;
        let cache_key = format!("app:{app_id}:channel:{channel}:cache_miss");

        let message_json = serde_json::to_string(message).map_err(|e| {
            Error::InvalidMessageFormat(format!("Failed to serialize message for cache: {e}"))
        })?;

        match ttl_seconds {
            Some(ttl) => {
                cache_manager
                    .set(&cache_key, &message_json, ttl)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to store cache with TTL: {e}")))?;
            }
            None => {
                cache_manager
                    .set(&cache_key, &message_json, 0)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to store cache: {e}")))?;
            }
        }

        debug!("Stored cache for channel {} in app {}", channel, app_id);
        Ok(())
    }

    /// Clear cache for a specific channel
    pub async fn clear_cache_for_channel(&self, app_id: &str, channel: &str) -> Result<()> {
        let mut cache_manager = self.cache_manager.lock().await;
        let cache_key = format!("app:{app_id}:channel:{channel}:cache_miss");

        cache_manager.remove(&cache_key).await.map_err(|e| {
            Error::Internal(format!("Failed to clear cache for channel {channel}: {e}"))
        })?;

        debug!("Cleared cache for channel {} in app {}", channel, app_id);
        Ok(())
    }

    /// Check if a channel has cached content
    pub async fn has_cache_for_channel(&self, app_id: &str, channel: &str) -> Result<bool> {
        let mut cache_manager = self.cache_manager.lock().await;
        let cache_key = format!("app:{app_id}:channel:{channel}:cache_miss");

        match cache_manager.get(&cache_key).await {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => {
                warn!("Error checking cache for channel {}: {}", channel, e);
                Ok(false) // Assume no cache on error
            }
        }
    }
}
