// src/adapter/handler/authentication.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::auth::AuthValidator;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::websocket::SocketId;

impl ConnectionHandler {
    pub async fn verify_channel_authentication(
        &self,
        app_config: &App,
        socket_id: &SocketId,
        request: &SubscriptionRequest,
    ) -> Result<bool> {
        // Public channels don't need authentication
        if !request.channel.starts_with("presence-") && !request.channel.starts_with("private-") {
            return Ok(true);
        }

        // Private/presence channels require authentication
        let signature = request.auth.as_ref().ok_or_else(|| {
            Error::Auth("Authentication signature required for this channel".into())
        })?;

        let channel_manager = self.channel_manager.read().await;

        // Create a temporary PusherMessage for signature validation
        let temp_message = crate::protocol::messages::PusherMessage {
            channel: Some(request.channel.clone()),
            event: Some("pusher:subscribe".to_string()),
            data: Some(crate::protocol::messages::MessageData::Json(
                serde_json::json!({
                    "channel": request.channel,
                    "auth": signature,
                    "channel_data": request.channel_data
                }),
            )),
            name: None,
            user_id: None,
        };

        let is_valid = channel_manager.signature_is_valid(
            app_config.clone(),
            socket_id,
            signature,
            temp_message,
        );

        Ok(is_valid)
    }

    pub async fn verify_signin_authentication(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SignInRequest,
    ) -> Result<()> {
        let auth_validator = AuthValidator::new(self.app_manager.clone());

        let is_valid = auth_validator
            .validate_channel_auth(
                socket_id.clone(),
                &app_config.key,
                &request.user_data,
                &request.auth,
            )
            .await?;

        if !is_valid {
            return Err(Error::Auth("Connection not authorized for signin.".into()));
        }

        Ok(())
    }
}
