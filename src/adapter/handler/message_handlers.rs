// src/adapter/handler/message_handlers.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::protocol::messages::PusherMessage;
use crate::websocket::SocketId;

impl ConnectionHandler {
    pub async fn handle_ping(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        let pong_message = PusherMessage::pong();
        self.send_message_to_socket(app_id, socket_id, pong_message)
            .await
    }

    pub async fn handle_pong(&self, app_id: &str, socket_id: &SocketId) -> Result<()> {
        tracing::debug!("Received pong from socket: {}", socket_id);
        let mut connection_manager = self.connection_manager.lock().await;
        if let Some(connection) = connection_manager.get_connection(socket_id, &app_id).await {
            let mut conn_locked = connection.0.lock().await;
            conn_locked.update_activity();
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
        let is_authenticated = self
            .verify_channel_authentication(app_config, socket_id, &request)
            .await?;

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
        self.handle_post_subscription(socket_id, app_config, &request, &subscription_result)
            .await?;

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
        };

        self.broadcast_to_channel(app_config, &request.channel, message, Some(socket_id))
            .await?;

        // Send webhook if configured
        self.send_client_event_webhook(socket_id, app_config, &request)
            .await?;

        Ok(())
    }
}
