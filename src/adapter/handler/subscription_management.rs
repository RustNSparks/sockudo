// src/adapter/handler/subscription_management.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::config::App;
use crate::channel::ChannelManager;
use crate::channel::{ChannelType, PresenceMemberInfo};
use crate::error::Result;
use crate::presence::PresenceManager;
use crate::protocol::messages::{MessageData, PresenceData, PusherMessage};
use crate::utils::is_cache_channel;
use crate::websocket::SocketId;
use sonic_rs::Value;
use std::collections::HashMap;

#[derive(Debug)]
pub struct SubscriptionResult {
    pub success: bool,
    pub auth_error: Option<String>,
    pub member: Option<PresenceMember>,
    pub channel_connections: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PresenceMember {
    pub user_id: String,
    pub user_info: Value,
}

impl ConnectionHandler {
    pub async fn execute_subscription(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SubscriptionRequest,
        is_authenticated: bool,
    ) -> Result<SubscriptionResult> {
        let temp_message = PusherMessage {
            channel: Some(request.channel.clone()),
            event: Some("pusher:subscribe".to_string()),
            data: Some(MessageData::Json(sonic_rs::json!({
                "channel": request.channel,
                "auth": request.auth,
                "channel_data": request.channel_data
            }))),
            name: None,
            user_id: None,
        };

        let subscription_result = ChannelManager::subscribe(
            &self.connection_manager,
            socket_id.as_ref(),
            &temp_message,
            &request.channel,
            is_authenticated,
            &app_config.id,
        )
        .await?;

        // Track subscription metrics if successful
        if subscription_result.success
            && let Some(ref metrics) = self.metrics
        {
            let channel_type = ChannelType::from_name(&request.channel);
            let channel_type_str = channel_type.as_str();

            // Mark subscription metric
            {
                let metrics_locked = metrics.lock().await;
                metrics_locked.mark_channel_subscription(&app_config.id, channel_type_str);
            }

            // Update active channel count if this is the first connection to the channel
            if subscription_result.channel_connections == Some(1) {
                // Channel became active - increment the count for this channel type
                // Pass the Arc directly to avoid holding any locks
                self.increment_active_channel_count(
                    &app_config.id,
                    channel_type_str,
                    metrics.clone(),
                )
                .await;
            }
        }

        // Convert the channel manager result to our result type
        Ok(SubscriptionResult {
            success: subscription_result.success,
            auth_error: subscription_result.auth_error,
            member: subscription_result.member.map(|m| PresenceMember {
                user_id: m.user_id.to_string(),
                user_info: m.user_info,
            }),
            channel_connections: subscription_result.channel_connections,
        })
    }

    pub async fn handle_post_subscription(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SubscriptionRequest,
        subscription_result: &SubscriptionResult,
    ) -> Result<()> {
        // Update connection state
        self.update_connection_subscription_state(
            socket_id,
            app_config,
            request,
            subscription_result,
        )
        .await?;

        // Handle channel-specific logic - send subscription success response first
        let channel_type = ChannelType::from_name(&request.channel);
        match channel_type {
            ChannelType::Presence => {
                self.handle_presence_subscription_success(
                    socket_id,
                    app_config,
                    request,
                    subscription_result,
                )
                .await?;
            }
            _ => {
                self.send_subscription_succeeded(socket_id, app_config, &request.channel, None)
                    .await?;
            }
        }

        // Send webhooks after subscription success response (non-blocking for client)
        if subscription_result.channel_connections == Some(1)
            && let Some(webhook_integration) = self.webhook_integration.clone()
        {
            let app_config = app_config.clone();
            let channel = request.channel.clone();
            tokio::spawn(async move {
                if let Err(e) = webhook_integration
                    .send_channel_occupied(&app_config, &channel)
                    .await
                {
                    tracing::warn!(
                        "Failed to send channel_occupied webhook for {}: {}",
                        channel,
                        e
                    );
                }
            });
        }

        // Send subscription count webhook for non-presence channels
        if !request.channel.starts_with("presence-")
            && let Some(webhook_integration) = &self.webhook_integration
        {
            let current_count = self
                .connection_manager
                .lock()
                .await
                .get_channel_socket_count(&app_config.id, &request.channel)
                .await;

            webhook_integration
                .send_subscription_count_changed(app_config, &request.channel, current_count)
                .await
                .ok();
        }

        // Handle cache channels
        if is_cache_channel(&request.channel) {
            self.send_missed_cache_if_exists(&app_config.id, socket_id, &request.channel)
                .await?;
        }

        Ok(())
    }

    async fn update_connection_subscription_state(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SubscriptionRequest,
        subscription_result: &SubscriptionResult,
    ) -> Result<()> {
        let mut connection_manager = self.connection_manager.lock().await;
        if let Some(conn_arc) = connection_manager
            .get_connection(socket_id, &app_config.id)
            .await
        {
            let mut conn_locked = conn_arc.inner.lock().await;
            conn_locked.subscribe_to_channel(request.channel.clone());

            // Handle presence data
            if let Some(ref member) = subscription_result.member {
                conn_locked.state.user_id = Some(member.user_id.clone());

                let presence_info = PresenceMemberInfo {
                    user_id: member.user_id.clone(),
                    user_info: Some(member.user_info.clone()),
                };

                conn_locked.add_presence_info(request.channel.clone(), presence_info);

                // Release the connection lock before calling add_user
                drop(conn_locked);
                drop(connection_manager);

                // Add user to the user-socket mapping so get_user_sockets() can find it
                self.connection_manager
                    .lock()
                    .await
                    .add_user(conn_arc.clone())
                    .await?;
            } else {
                // Release locks when not needed
                drop(conn_locked);
                drop(connection_manager);
            }
        }

        Ok(())
    }

    async fn handle_presence_subscription_success(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SubscriptionRequest,
        subscription_result: &SubscriptionResult,
    ) -> Result<()> {
        if let Some(ref presence_member) = subscription_result.member {
            // Use centralized presence member addition logic (handles both webhook and broadcast)
            PresenceManager::handle_member_added(
                &self.connection_manager,
                self.webhook_integration.as_ref(),
                app_config,
                &request.channel,
                &presence_member.user_id,
                Some(&presence_member.user_info),
                Some(socket_id),
            )
            .await?;

            // Get current members and send presence data to new member
            let members_map = self
                .connection_manager
                .lock()
                .await
                .get_channel_members(&app_config.id, &request.channel)
                .await?;

            let presence_data = PresenceData {
                ids: members_map.keys().cloned().collect::<Vec<String>>(),
                hash: members_map
                    .iter()
                    .map(|(k, v)| (k.clone(), v.user_info.clone()))
                    .collect::<HashMap<String, Option<Value>>>(),
                count: members_map.len(),
            };

            self.send_subscription_succeeded(
                socket_id,
                app_config,
                &request.channel,
                Some(presence_data),
            )
            .await?;
        }

        Ok(())
    }

    async fn send_subscription_succeeded(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        channel: &str,
        data: Option<PresenceData>,
    ) -> Result<()> {
        let response_msg = PusherMessage::subscription_succeeded(channel.to_string(), data);
        self.connection_manager
            .lock()
            .await
            .send_message(&app_config.id, socket_id, response_msg)
            .await
    }
}
