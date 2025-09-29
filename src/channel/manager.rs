use super::PresenceMemberInfo;
use super::types::ChannelType;
use crate::adapter::ConnectionManager;
use crate::app::config::App;
use crate::error::Error;
use crate::protocol::messages::{MessageData, PusherMessage};
use crate::token::{Token, secure_compare};
use crate::websocket::SocketId;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap as StdHashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceMember {
    pub(crate) user_id: Box<str>,
    pub(crate) user_info: Value,
    pub(crate) socket_id: Option<Box<str>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JoinResponse {
    pub(crate) success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_connections: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<PresenceMember>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaveResponse {
    pub(crate) left: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_connections: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<PresenceMember>,
}

pub struct ChannelManager;

// Static cache for channel types to avoid repeated parsing
// Using LRU cache to prevent memory growth and improve cache efficiency
static CHANNEL_TYPE_CACHE: std::sync::LazyLock<std::sync::Mutex<LruCache<String, ChannelType>>> =
    std::sync::LazyLock::new(|| {
        std::sync::Mutex::new(LruCache::new(
            NonZeroUsize::new(1000).expect("Cache size must be non-zero")
        ))
    });

impl ChannelManager {
    fn get_channel_type(channel_name: &str) -> ChannelType {
        let mut cache = CHANNEL_TYPE_CACHE.lock().unwrap();

        // Check if already in cache (LRU will move to front)
        if let Some(channel_type) = cache.get(channel_name) {
            return *channel_type;
        }

        // Not in cache, compute the channel type
        let channel_type = ChannelType::from_name(channel_name);

        // Insert into LRU cache (automatically evicts least recently used if full)
        cache.put(channel_name.to_string(), channel_type);

        channel_type
    }

    fn create_success_join_response(
        channel_connections: usize,
        member: Option<PresenceMember>,
    ) -> JoinResponse {
        JoinResponse {
            success: true,
            channel_connections: Some(channel_connections),
            member,
            auth_error: None,
            error_message: None,
            error_code: None,
            _type: None,
        }
    }

    fn create_leave_response(
        left: bool,
        remaining_connections: usize,
        member: Option<PresenceMember>,
    ) -> LeaveResponse {
        LeaveResponse {
            left,
            remaining_connections: Some(remaining_connections),
            member,
        }
    }

    pub async fn subscribe(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        socket_id: &str,
        data: &PusherMessage,
        channel_name: &str,
        is_authenticated: bool,
        app_id: &str,
    ) -> Result<JoinResponse, Error> {
        let channel_type = Self::get_channel_type(channel_name);

        if channel_type.requires_authentication() && !is_authenticated {
            return Err(Error::Auth("Channel requires authentication".into()));
        }

        let socket_id_owned = SocketId(socket_id.to_string());

        // Parse presence data early to fail fast before any locking
        let member = if channel_type == ChannelType::Presence {
            Some(Self::parse_presence_data(&data.data)?)
        } else {
            None
        };

        // Use add_to_channel's idempotent behavior to atomically handle subscription
        // This eliminates the race condition by combining check and add operations
        let (was_newly_added, total_connections) = {
            let mut conn_mgr = connection_manager.lock().await;

            // add_to_channel returns true if newly added, false if already existed
            let newly_added = conn_mgr
                .add_to_channel(app_id, channel_name, &socket_id_owned)
                .await?;

            // Get the total connection count
            let total = conn_mgr
                .get_channel_sockets(app_id, channel_name)
                .await?
                .len();

            (newly_added, total)
        };

        // If socket was already subscribed, return without the member data
        // (member data is only sent on initial subscription)
        let response_member = if was_newly_added { member } else { None };

        Ok(Self::create_success_join_response(
            total_connections,
            response_member,
        ))
    }

    pub async fn unsubscribe(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        socket_id: &str,
        channel_name: &str,
        app_id: &str,
        user_id: Option<&str>,
    ) -> Result<LeaveResponse, Error> {
        let socket_id_owned = SocketId(socket_id.to_string());
        let channel_type = Self::get_channel_type(channel_name);

        // Get presence member info before removal if needed (separate lock scope)
        let member = if channel_type == ChannelType::Presence {
            if let Some(user_id) = user_id {
                let mut conn_mgr = connection_manager.lock().await;
                let members = conn_mgr.get_channel_members(app_id, channel_name).await?;
                drop(conn_mgr); // Release lock immediately

                members.get(user_id).map(|member| PresenceMember {
                    user_id: member.user_id.clone().into_boxed_str(),
                    user_info: member.user_info.clone().unwrap_or_default(),
                    socket_id: None,
                })
            } else {
                None
            }
        } else {
            None
        };

        // Remove socket and handle cleanup atomically
        let (socket_removed, remaining_connections) = {
            let mut conn_mgr = connection_manager.lock().await;

            let socket_removed = conn_mgr
                .remove_from_channel(app_id, channel_name, &socket_id_owned)
                .await?;

            let remaining = conn_mgr
                .get_channel_sockets(app_id, channel_name)
                .await?
                .len();

            // Clean up empty channels
            if remaining == 0 {
                conn_mgr.remove_channel(app_id, channel_name).await;
            }

            (socket_removed, remaining)
        };

        Ok(Self::create_leave_response(
            socket_removed,
            remaining_connections,
            member,
        ))
    }

    fn parse_presence_data(data: &Option<MessageData>) -> Result<PresenceMember, Error> {
        let channel_data = data
            .as_ref()
            .ok_or_else(|| Error::Channel("Missing presence data".into()))?;

        match channel_data {
            MessageData::Structured {
                channel_data,
                extra,
                ..
            } => {
                let channel_data_str = channel_data
                    .as_ref()
                    .ok_or_else(|| Error::Channel("Missing channel_data".into()))?;

                // Parse JSON directly into the fields we need, avoiding intermediate Value
                let parsed: serde_json::Value = serde_json::from_str(channel_data_str)
                    .map_err(|_| Error::Channel("Invalid JSON in channel_data".into()))?;

                Self::extract_presence_member(&parsed, extra)
            }
            MessageData::Json(data) => Self::extract_presence_member(data, &Default::default()),
            _ => Err(Error::Channel("Invalid presence data format".into())),
        }
    }

    fn extract_presence_member(
        data: &Value,
        extra: &StdHashMap<String, Value>,
    ) -> Result<PresenceMember, Error> {
        // For structured data, channel_data is already parsed
        if let Some(channel_data_str) = data.get("channel_data").and_then(|v| v.as_str()) {
            // Parse the inner JSON
            let user_data: Value = serde_json::from_str(channel_data_str)
                .map_err(|_| Error::Channel("Invalid JSON in channel_data".into()))?;

            let user_id = user_data
                .get("user_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Channel("Missing user_id in channel_data".into()))?;

            // Clone user_info only once
            let user_info = user_data
                .get("user_info")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let socket_id = extra.get("socket_id").and_then(|v| v.as_str());

            Ok(PresenceMember {
                user_id: user_id.to_string().into_boxed_str(),
                user_info,
                socket_id: socket_id.map(|s| s.to_string().into_boxed_str()),
            })
        } else {
            // Direct JSON case - look for user_id and user_info directly
            let user_id = data
                .get("user_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Channel("Missing user_id in presence data".into()))?;

            let user_info = data.get("user_info").cloned().unwrap_or_else(|| json!({}));

            let socket_id = extra.get("socket_id").and_then(|v| v.as_str());

            Ok(PresenceMember {
                user_id: user_id.to_string().into_boxed_str(),
                user_info,
                socket_id: socket_id.map(|s| s.to_string().into_boxed_str()),
            })
        }
    }

    pub fn signature_is_valid(
        app_config: App,
        socket_id: &SocketId,
        signature: &str,
        message: PusherMessage,
    ) -> bool {
        let expected = Self::get_expected_signature(app_config, socket_id, message);
        secure_compare(signature, &expected)
    }

    pub fn get_expected_signature(
        app_config: App,
        socket_id: &SocketId,
        message: PusherMessage,
    ) -> String {
        let token = Token::new(app_config.key.clone(), app_config.secret);
        format!(
            "{}:{}",
            app_config.key,
            token.sign(&Self::get_data_to_sign_for_signature(socket_id, message))
        )
    }

    fn get_data_to_sign_for_signature(socket_id: &SocketId, message: PusherMessage) -> String {
        let message_data = message.data.unwrap();

        // Pre-calculate capacity for string building
        let socket_id_str = socket_id.to_string();
        let socket_id_len = socket_id_str.len();

        match message_data {
            MessageData::Structured {
                channel_data,
                channel,
                ..
            } => {
                let channel = channel.unwrap();
                let channel_data = channel_data.unwrap_or_default();
                let is_presence = channel.starts_with("presence-");

                if is_presence && !channel_data.is_empty() {
                    // Pre-allocate with known capacity: socket_id + ":" + channel + ":" + channel_data
                    let mut result = String::with_capacity(
                        socket_id_len + 2 + channel.len() + channel_data.len(),
                    );
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(&channel);
                    result.push(':');
                    result.push_str(&channel_data);
                    result
                } else {
                    // Pre-allocate with known capacity: socket_id + ":" + channel
                    let mut result = String::with_capacity(socket_id_len + 1 + channel.len());
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(&channel);
                    result
                }
            }
            MessageData::Json(data) => {
                let channel = data.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                let channel_data = data
                    .get("channel_data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let is_presence = channel.starts_with("presence-");

                if is_presence && !channel_data.is_empty() {
                    let mut result = String::with_capacity(
                        socket_id_len + 2 + channel.len() + channel_data.len(),
                    );
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(channel);
                    result.push(':');
                    result.push_str(channel_data);
                    result
                } else {
                    let mut result = String::with_capacity(socket_id_len + 1 + channel.len());
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(channel);
                    result
                }
            }
            MessageData::String(data) => {
                let parsed_data: Value = serde_json::from_str(&data).unwrap_or_default();
                let channel = parsed_data
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let channel_data = parsed_data
                    .get("channel_data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let is_presence = channel.starts_with("presence-");

                if is_presence && !channel_data.is_empty() {
                    let mut result = String::with_capacity(
                        socket_id_len + 2 + channel.len() + channel_data.len(),
                    );
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(channel);
                    result.push(':');
                    result.push_str(channel_data);
                    result
                } else {
                    let mut result = String::with_capacity(socket_id_len + 1 + channel.len());
                    result.push_str(&socket_id_str);
                    result.push(':');
                    result.push_str(channel);
                    result
                }
            }
        }
    }

    pub async fn get_channel_members(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        app_id: &str,
        channel: &str,
    ) -> Result<StdHashMap<String, PresenceMemberInfo>, Error> {
        let mut conn_mgr = connection_manager.lock().await;
        conn_mgr.get_channel_members(app_id, channel).await
    }

    /// Batch unsubscribe operation - single lock acquisition for multiple operations
    pub async fn batch_unsubscribe(
        connection_manager: &Arc<Mutex<dyn ConnectionManager + Send + Sync>>,
        operations: Vec<(String, String, String)>, // (socket_id, channel_name, app_id)
    ) -> Result<Vec<Result<(bool, usize), Error>>, Error> {
        if operations.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::with_capacity(operations.len());
        let mut conn_mgr = connection_manager.lock().await;
        let mut channels_to_cleanup = Vec::new();

        for (socket_id, channel_name, app_id) in operations {
            let socket_id_owned = SocketId(socket_id);

            // Remove from channel
            match conn_mgr
                .remove_from_channel(&app_id, &channel_name, &socket_id_owned)
                .await
            {
                Ok(was_removed) => {
                    // Get remaining count
                    match conn_mgr.get_channel_sockets(&app_id, &channel_name).await {
                        Ok(sockets) => {
                            let remaining = sockets.len();
                            results.push(Ok((was_removed, remaining)));

                            // Mark for cleanup if empty
                            if remaining == 0 {
                                channels_to_cleanup.push((app_id.clone(), channel_name.clone()));
                            }
                        }
                        Err(e) => results.push(Err(e)),
                    }
                }
                Err(e) => results.push(Err(e)),
            }
        }

        // Clean up empty channels
        for (app_id, channel_name) in channels_to_cleanup {
            conn_mgr.remove_channel(&app_id, &channel_name).await;
        }

        Ok(results)
    }
}
