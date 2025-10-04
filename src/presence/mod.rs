use crate::adapter::connection_manager::ConnectionManager;
use crate::app::config::App;
use crate::error::Result;
use crate::protocol::messages::PusherMessage;
use crate::webhook::integration::WebhookIntegration;
use crate::websocket::SocketId;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error};

/// Centralized presence channel management functionality
/// This module handles presence member removal logic that needs to be
/// consistent across different disconnect paths (sync, async, direct unsubscribe)
pub struct PresenceManager;

impl PresenceManager {
    /// Handles presence member addition including both webhook and broadcast
    /// Only sends events if this is the user's FIRST connection to the presence channel
    pub async fn handle_member_added(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        webhook_integration: Option<&Arc<WebhookIntegration>>,
        app_config: &App,
        channel: &str,
        user_id: &str,
        user_info: Option<&sonic_rs::Value>,
        excluding_socket: Option<&SocketId>,
    ) -> Result<()> {
        debug!(
            "Processing presence member addition for user {} in channel {} (app: {})",
            user_id, channel, app_config.id
        );

        // Check if user already had connections in this presence channel (excluding current socket)
        let had_other_connections = Self::user_has_other_connections_in_presence_channel(
            connection_manager,
            &app_config.id,
            channel,
            user_id,
            excluding_socket,
        )
        .await?;

        if !had_other_connections {
            debug!(
                "User {} is joining channel {} for the first time, sending member_added events",
                user_id, channel
            );

            // Send member_added webhook
            if let Some(webhook_integration) = webhook_integration {
                webhook_integration
                    .send_member_added(app_config, channel, user_id)
                    .await
                    .ok(); // Don't fail the entire operation if webhook fails
            }

            // Broadcast member_added event to existing clients in the channel
            let member_added_msg = crate::protocol::messages::PusherMessage::member_added(
                channel.to_string(),
                user_id.to_string(),
                user_info.cloned(),
            );
            Self::broadcast_to_channel(
                connection_manager,
                &app_config.id,
                channel,
                member_added_msg,
                excluding_socket,
            )
            .await?;

            debug!(
                "Successfully processed member_added for user {} in channel {}",
                user_id, channel
            );
        } else {
            debug!(
                "User {} already has connections in channel {}, skipping member_added events",
                user_id, channel
            );
        }

        // Always broadcast presence join to all nodes for cluster replication (for every connection)
        if let Some(excluding_socket) = excluding_socket
            && let Some(horizontal_adapter) =
                connection_manager.lock().await.as_horizontal_adapter()
        {
            horizontal_adapter
                .broadcast_presence_join(
                    &app_config.id,
                    channel,
                    user_id,
                    excluding_socket.as_ref(),
                    user_info.cloned(),
                )
                .await
                .map_err(|e| {
                    error!("Failed to broadcast presence join: {}", e);
                    e
                })
                .ok(); // Log but don't fail the operation
        }

        Ok(())
    }

    /// Handles presence member removal including both webhook and broadcast
    /// This centralizes the logic that was duplicated across sync cleanup,
    /// async cleanup, and direct unsubscribe paths
    pub async fn handle_member_removed(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        webhook_integration: Option<&Arc<WebhookIntegration>>,
        app_config: &App,
        channel: &str,
        user_id: &str,
        excluding_socket: Option<&SocketId>,
    ) -> Result<()> {
        debug!(
            "Processing presence member removal for user {} in channel {} (app: {})",
            user_id, channel, app_config.id
        );

        // Check if user has other connections in this presence channel
        let has_other_connections = Self::user_has_other_connections_in_presence_channel(
            connection_manager,
            &app_config.id,
            channel,
            user_id,
            excluding_socket,
        )
        .await?;

        if !has_other_connections {
            debug!(
                "User {} has no other connections in channel {}, sending member_removed events",
                user_id, channel
            );

            // Send member_removed webhook
            if let Some(webhook_integration) = webhook_integration {
                webhook_integration
                    .send_member_removed(app_config, channel, user_id)
                    .await
                    .ok(); // Don't fail the entire operation if webhook fails
            }

            // Broadcast member_removed event to remaining clients in the channel
            let member_removed_msg =
                PusherMessage::member_removed(channel.to_string(), user_id.to_string());
            Self::broadcast_to_channel(
                connection_manager,
                &app_config.id,
                channel,
                member_removed_msg,
                excluding_socket,
            )
            .await?;

            debug!(
                "Successfully processed member_removed for user {} in channel {}",
                user_id, channel
            );
        } else {
            debug!(
                "User {} has other connections in channel {}, skipping member_removed events",
                user_id, channel
            );
        }

        // Always broadcast presence leave to all nodes for cluster replication (for every disconnection)
        if let Some(excluding_socket) = excluding_socket
            && let Some(horizontal_adapter) =
                connection_manager.lock().await.as_horizontal_adapter()
        {
            horizontal_adapter
                .broadcast_presence_leave(
                    &app_config.id,
                    channel,
                    user_id,
                    excluding_socket.as_ref(),
                )
                .await
                .map_err(|e| {
                    error!("Failed to broadcast presence leave: {}", e);
                    e
                })
                .ok(); // Log but don't fail the operation
        }

        Ok(())
    }

    /// Check if a user has other connections in a presence channel
    /// Uses the same logic as the original working implementation:
    /// Get all user's sockets and check if any are still subscribed to this channel (excluding specified socket)
    async fn user_has_other_connections_in_presence_channel(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        app_id: &str,
        channel: &str,
        user_id: &str,
        excluding_socket: Option<&SocketId>,
    ) -> Result<bool> {
        // Use cluster-wide connection check for multi-node support
        let mut connection_manager = connection_manager.lock().await;
        let subscribed_count = connection_manager
            .count_user_connections_in_channel(user_id, app_id, channel, excluding_socket)
            .await?;

        let has_other_connections = subscribed_count > 0;

        Ok(has_other_connections)
    }

    /// Broadcast a message to all clients in a channel, optionally excluding one socket
    async fn broadcast_to_channel(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        app_id: &str,
        channel: &str,
        message: PusherMessage,
        excluding_socket: Option<&SocketId>,
    ) -> Result<()> {
        let mut connection_manager = connection_manager.lock().await;
        connection_manager
            .send(channel, message, excluding_socket, app_id, None)
            .await
    }
}
