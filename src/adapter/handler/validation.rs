// src/adapter/handler/validation.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::config::App;
use crate::channel::ChannelType;
use crate::error::{Error, Result};
use crate::protocol::constants::*;
use crate::utils;
use sonic_rs::{JsonValueTrait, Value};

impl ConnectionHandler {
    pub async fn validate_subscription_request(
        &self,
        app_config: &App,
        request: &SubscriptionRequest,
    ) -> Result<()> {
        if !app_config.enabled {
            return Err(Error::ApplicationDisabled);
        }

        // Validate channel name
        utils::validate_channel_name(app_config, &request.channel).await?;

        // Check if authentication is required and provided
        let requires_auth =
            request.channel.starts_with("presence-") || request.channel.starts_with("private-");

        if requires_auth && request.auth.is_none() {
            return Err(Error::Auth(
                "Authentication signature required for this channel".into(),
            ));
        }

        Ok(())
    }

    pub async fn validate_presence_subscription(
        &self,
        app_config: &App,
        request: &SubscriptionRequest,
    ) -> Result<()> {
        let channel_data = request.channel_data.as_ref().ok_or_else(|| {
            Error::InvalidMessageFormat("Missing channel_data for presence channel".into())
        })?;

        // Parse and validate user info size
        let user_info_payload: Value = sonic_rs::from_str(channel_data).map_err(|_| {
            Error::InvalidMessageFormat("Invalid channel_data JSON for presence".into())
        })?;

        let user_info = user_info_payload
            .get("user_info")
            .cloned()
            .unwrap_or_default();
        let user_info_size_kb = utils::data_to_bytes_flexible(vec![user_info]) / 1024;

        if let Some(max_size) = app_config.max_presence_member_size_in_kb
            && user_info_size_kb > max_size as usize
        {
            return Err(Error::Channel(format!(
                "Presence member data size ({user_info_size_kb}KB) exceeds limit ({max_size}KB)"
            )));
        }

        // Check member count limit
        if let Some(max_members) = app_config.max_presence_members_per_channel {
            let current_count = self
                .get_channel_member_count(app_config, &request.channel)
                .await?;
            if current_count >= max_members as usize {
                return Err(Error::OverCapacity);
            }
        }

        Ok(())
    }

    pub async fn validate_client_event(
        &self,
        app_config: &App,
        request: &ClientEventRequest,
    ) -> Result<()> {
        // Check if client events are enabled
        if !app_config.enable_client_messages {
            return Err(Error::ClientEvent(
                "Client events are not enabled for this app".into(),
            ));
        }

        // Check for reserved prefixes that clients cannot use
        if request.event.starts_with("pusher:") || request.event.starts_with("pusher_internal:") {
            return Err(Error::InvalidEventName(
                "Client events cannot use reserved prefixes 'pusher:' or 'pusher_internal:'".into(),
            ));
        }

        // Validate event name
        if !request.event.starts_with(CLIENT_EVENT_PREFIX) {
            return Err(Error::InvalidEventName(
                "Client events must start with 'client-'".into(),
            ));
        }

        // Validate event name length
        let max_event_len = app_config
            .max_event_name_length
            .unwrap_or(DEFAULT_EVENT_NAME_MAX_LENGTH as u32);
        if request.event.len() > max_event_len as usize {
            return Err(Error::InvalidEventName(format!(
                "Event name exceeds maximum length of {max_event_len}"
            )));
        }

        // Validate channel name length
        let max_channel_len = app_config
            .max_channel_name_length
            .unwrap_or(DEFAULT_CHANNEL_NAME_MAX_LENGTH as u32);
        if request.channel.len() > max_channel_len as usize {
            return Err(Error::InvalidChannelName(format!(
                "Channel name exceeds maximum length of {max_channel_len}"
            )));
        }

        // Validate channel type
        let channel_type = ChannelType::from_name(&request.channel);
        if !matches!(channel_type, ChannelType::Private | ChannelType::Presence) {
            return Err(Error::ClientEvent(
                "Client events can only be sent to private or presence channels".into(),
            ));
        }

        // Validate payload size
        if let Some(max_payload_kb) = app_config.max_event_payload_in_kb {
            let payload_size = utils::data_to_bytes_flexible(vec![request.data.clone()]);
            if payload_size > (max_payload_kb as usize * 1024) {
                return Err(Error::ClientEvent(format!(
                    "Event payload size ({payload_size} bytes) exceeds limit ({max_payload_kb}KB)"
                )));
            }
        }

        Ok(())
    }
}
