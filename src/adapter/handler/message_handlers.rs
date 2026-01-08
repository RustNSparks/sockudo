// src/adapter/handler/message_handlers.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::protocol::messages::PusherMessage;
use crate::websocket::SocketId;

impl ConnectionHandler {
    pub async fn handle_ping(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        // Reset connection status to Active when we receive a ping from client
        {
            let mut connection_manager = self.connection_manager.lock().await;
            if let Some(connection) = connection_manager.get_connection(socket_id, app_id).await {
                let mut conn_locked = connection.inner.lock().await;
                conn_locked.state.status = crate::websocket::ConnectionStatus::Active;
            } else {
                tracing::warn!("Ping received for unknown socket: {}", socket_id);
            }
        }

        let pong_message = PusherMessage::pong();
        self.send_message_to_socket(app_id, socket_id, pong_message)
            .await
    }

    pub async fn handle_pong(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        tracing::debug!("Received pong from socket: {}", socket_id);
        let mut connection_manager = self.connection_manager.lock().await;
        if let Some(connection) = connection_manager.get_connection(socket_id, app_id).await {
            let mut conn_locked = connection.inner.lock().await;
            // Note: activity timestamp is already updated by handle_message() for ALL messages
            // We just need to reset connection status to Active when we receive a pong
            conn_locked.state.status = crate::websocket::ConnectionStatus::Active;
        } else {
            tracing::warn!("Pong received for unknown socket: {}", socket_id);
        }
        Ok(())
    }

    pub async fn handle_subscribe_request(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: SubscriptionRequest,
    ) -> Result<()> {
        // Validate the request
        self.validate_subscription_request(app_config, &request)
            .await?;

        // Check authentication if required
        let is_authenticated = match self
            .verify_channel_authentication(app_config, socket_id, &request)
            .await
        {
            Ok(authenticated) => authenticated,
            Err(e) => {
                // Track authentication failures
                if let Some(ref metrics) = self.metrics {
                    let metrics_locked = metrics.lock().await;
                    let error_type = match &e {
                        Error::Auth(_) => "authentication_failed",
                        _ => "authentication_error",
                    };
                    metrics_locked.mark_connection_error(&app_config.id, error_type);
                }
                return Err(e);
            }
        };

        // Validate presence channel specifics
        if request.channel.starts_with("presence-") {
            self.validate_presence_subscription(app_config, &request)
                .await?;
        }

        // Perform the subscription
        let subscription_result = self
            .execute_subscription(socket_id, app_config, &request, is_authenticated)
            .await?;

        // Handle post-subscription logic
        if let Err(e) = self
            .handle_post_subscription(socket_id, app_config, &request, &subscription_result)
            .await
        {
            // ROLLBACK: If post-subscription fails (e.g., webhook buffer full, serialization error),
            // we MUST unsubscribe the socket to prevent "occupied with no subscriber" (zombie) channels.
            tracing::warn!(
                "Post-subscription failed for socket {} on channel {}: {}. Rolling back subscription.",
                socket_id,
                request.channel,
                e
            );

            // Attempt to unsubscribe using the connection manager directly since we are handling an error
            // within the subscription flow.
            // Attempt to unsubscribe using the connection manager directly since we are handling an error
            // within the subscription flow.
            use crate::channel::ChannelManager;
            // DO NOT lock connection_manager here, ChannelManager::unsubscribe handles locking internally.
            if let Err(unsubscribe_err) = ChannelManager::unsubscribe(
                &self.connection_manager,
                &socket_id.0,
                &request.channel,
                &app_config.id,
                None, // We don't have user_id readily available here without lookups, and for rollback None is acceptable (skips member info retrieval)
            )
            .await
            {
                tracing::error!(
                    "Failed to rollback subscription for socket {} on channel {}: {}",
                    socket_id,
                    request.channel,
                    unsubscribe_err
                );
            }
            return Err(e);
        }

        Ok(())
    }

    pub async fn handle_signin_request(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: SignInRequest,
    ) -> Result<()> {
        // Validate signin is enabled
        if !app_config.enable_user_authentication.unwrap_or(false) {
            return Err(Error::Auth(
                "User authentication is disabled for this app".into(),
            ));
        }

        // Parse and validate user data
        let user_info = self.parse_and_validate_user_data(&request.user_data)?;

        // Verify authentication
        self.verify_signin_authentication(socket_id, app_config, &request)
            .await?;

        // Update connection state
        self.update_connection_with_user_info(socket_id, app_config, &user_info)
            .await?;

        // Handle watchlist functionality
        self.handle_signin_watchlist(socket_id, app_config, &user_info)
            .await?;

        // Send success response
        self.send_signin_success(socket_id, app_config, &request)
            .await?;

        Ok(())
    }

    pub async fn handle_client_event_request(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: ClientEventRequest,
    ) -> Result<()> {
        // Validate the client event
        self.validate_client_event(app_config, &request).await?;

        // Check if socket is subscribed to the channel
        self.verify_channel_subscription(socket_id, app_config, &request.channel)
            .await?;

        // Rate limit check is handled in the main message handler

        // Send the event
        let message = PusherMessage {
            channel: Some(request.channel.clone()),
            event: Some(request.event.clone()),
            data: Some(crate::protocol::messages::MessageData::Json(
                request.data.clone(),
            )),
            name: None,
            user_id: None,
        };

        self.broadcast_to_channel(app_config, &request.channel, message, Some(socket_id))
            .await?;

        // Send webhook asynchronously (non-blocking for client)
        if let Some(webhook_integration) = self.webhook_integration.clone() {
            let socket_id = socket_id.clone();
            let app_config = app_config.clone();
            let request = request.clone();
            tokio::spawn(async move {
                if let Err(e) = webhook_integration
                    .send_client_event(
                        &app_config,
                        &request.channel,
                        &request.event,
                        request.data.clone(),
                        Some(socket_id.as_ref()),
                        None, // Skip user_id for async webhooks to avoid lock dependencies
                    )
                    .await
                {
                    tracing::error!("Failed to send client event webhook: {}", e);
                }
            });
        }

        Ok(())
    }
}
