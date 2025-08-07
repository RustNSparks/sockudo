#![allow(unused_variables)]
#![allow(unused_assignments)]
// src/adapter/handler/signin_management.rs
use super::ConnectionHandler;
use super::types::*;
use crate::app::config::App;
use crate::error::{Error, Result};
use crate::protocol::messages::{MessageData, PusherMessage};
use crate::websocket::{SocketId, UserInfo};
use serde_json::{Value, json};
use tracing::{info, warn};

impl ConnectionHandler {
    pub fn parse_and_validate_user_data(&self, user_data_str: &str) -> Result<UserInfo> {
        let user_info_val: Value = serde_json::from_str(user_data_str)
            .map_err(|e| Error::Auth(format!("Invalid user_data JSON: {e}")))?;

        let user_id = user_info_val
            .get("id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| Error::Auth("Missing 'id' field in user_data".into()))?
            .to_string();

        let watchlist = user_info_val
            .get("watchlist")
            .and_then(|w| w.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            });

        Ok(UserInfo {
            id: user_id,
            watchlist,
            info: Some(user_info_val),
        })
    }

    pub async fn update_connection_with_user_info(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        user_info: &UserInfo,
    ) -> Result<()> {
        // Clear authentication timeout since user is now signed in
        self.clear_user_authentication_timeout(&app_config.id, socket_id)
            .await?;

        // Update connection state
        let mut connection_manager = self.connection_manager.lock().await;
        let connection_arc = connection_manager
            .get_connection(socket_id, &app_config.id)
            .await
            .ok_or_else(|| Error::ConnectionNotFound)?;

        {
            let mut conn_locked = connection_arc.0.lock().await;
            conn_locked.set_user_info(user_info.clone());
        }

        // Re-add user to adapter's tracking (this updates user associations)
        connection_manager.add_user(connection_arc.clone()).await?;

        Ok(())
    }

    pub async fn handle_signin_watchlist(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        user_info: &UserInfo,
    ) -> Result<Vec<PusherMessage>> {
        let mut watchlist_events = Vec::new();
        let mut watchers_to_notify = Vec::new();

        if app_config.enable_watchlist_events.unwrap_or(false) && user_info.watchlist.is_some() {
            info!(
                "Processing watchlist for user {} with {} watched users",
                user_info.id,
                user_info.watchlist.as_ref().unwrap().len()
            );

            // Add user to watchlist manager and get initial status events
            let events = self
                .watchlist_manager
                .add_user_with_watchlist(
                    &app_config.id,
                    &user_info.id,
                    socket_id.clone(),
                    user_info.watchlist.clone(),
                )
                .await?;

            watchlist_events = events;

            // Get watchers who should be notified that this user came online
            watchers_to_notify = self
                .get_watchers_for_user(&app_config.id, &user_info.id)
                .await?;

            info!(
                "User {} signin: {} watchlist events to send, notifying {} watchers",
                user_info.id,
                watchlist_events.len(),
                watchers_to_notify.len()
            );

            // Send watchlist events to the newly signed-in user
            for event in &watchlist_events {
                if let Err(e) = self
                    .connection_manager
                    .lock()
                    .await
                    .send_message(&app_config.id, socket_id, event.clone())
                    .await
                {
                    warn!(
                        "Failed to send watchlist event to user {}: {}",
                        user_info.id, e
                    );
                }
            }

            // Notify watchers that this user came online
            if !watchers_to_notify.is_empty() {
                let online_event =
                    PusherMessage::watchlist_online_event(vec![user_info.id.clone()]);

                for watcher_socket_id in watchers_to_notify {
                    if let Err(e) = self
                        .connection_manager
                        .lock()
                        .await
                        .send_message(&app_config.id, &watcher_socket_id, online_event.clone())
                        .await
                    {
                        warn!(
                            "Failed to send online notification to watcher {}: {}",
                            watcher_socket_id, e
                        );
                    }
                }
            }
        }

        Ok(watchlist_events)
    }

    pub async fn send_signin_success(
        &self,
        socket_id: &SocketId,
        app_config: &App,
        request: &SignInRequest,
    ) -> Result<()> {
        let success_message = PusherMessage {
            channel: None,
            name: None,
            event: Some("pusher:signin_success".into()),
            data: Some(MessageData::Json(json!({
                "user_data": request.user_data,
                "auth": request.auth
            }))),
        };

        self.connection_manager
            .lock()
            .await
            .send_message(&app_config.id, socket_id, success_message)
            .await
    }

    pub(crate) async fn get_watchers_for_user(
        &self,
        app_id: &str,
        user_id: &str,
    ) -> Result<Vec<SocketId>> {
        let mut watcher_sockets = Vec::new();

        // Get all users who are watching this user
        let watchers = self
            .watchlist_manager
            .get_watchers_for_user(app_id, user_id)
            .await?;

        // For each watcher, get their active socket IDs
        let mut connection_manager = self.connection_manager.lock().await;
        for watcher_user_id in watchers {
            let user_sockets = connection_manager
                .get_user_sockets(&watcher_user_id, app_id)
                .await?;

            for socket_ref in user_sockets {
                let socket_guard = socket_ref.0.lock().await;
                watcher_sockets.push(socket_guard.state.socket_id.clone());
            }
        }

        Ok(watcher_sockets)
    }
}
