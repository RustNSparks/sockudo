// Make sure App is in scope
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result}; // Error should be in scope

use crate::websocket::{SocketId, WebSocket, WebSocketRef};
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use futures::future::join_all;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::WriteHalf;
use tracing::{debug, error, info, warn};

// Represents a namespace, typically tied to a specific application ID.
// Manages WebSocket connections, channel subscriptions, and user presence within that app.
pub struct Namespace {
    pub app_id: String,
    // Stores all active WebSocket connections, keyed by their unique SocketId.
    pub sockets: DashMap<SocketId, WebSocketRef>,
    // Maps channel names (String) to a set of SocketIds subscribed to that channel.
    pub channels: DashMap<String, DashSet<SocketId>>,
    // Maps user IDs (String) to a set of WebSocket references associated with that user.
    pub users: DashMap<String, DashSet<WebSocketRef>>,
}

impl Namespace {
    // Creates a new Namespace for a given application ID.
    pub fn new(app_id: String) -> Self {
        Self {
            app_id,
            sockets: DashMap::new(),
            channels: DashMap::new(),
            users: DashMap::new(),
        }
    }

    // Adds a new WebSocket connection to the namespace.
    pub async fn add_socket(
        &self,
        socket_id: SocketId,
        socket_writer: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>,
        app_manager: Arc<dyn AppManager + Send + Sync>,
    ) -> Result<WebSocketRef> {
        // Fetch the application configuration first
        let app_config = match app_manager.find_by_id(&self.app_id).await {
            Ok(Some(app)) => app,
            Ok(None) => {
                error!(
                    "App not found for app_id: {}. Cannot initialize socket: {}",
                    self.app_id, socket_id
                );
                return Err(Error::ApplicationNotFound);
            }
            Err(e) => {
                error!(
                    "Failed to get app {} for socket {}: {}",
                    self.app_id, socket_id, e
                );
                return Err(Error::Internal(format!(
                    "Failed to retrieve app config: {e}"
                )));
            }
        };

        // Create the WebSocket using new structure
        let mut websocket = WebSocket::new(socket_id.clone(), socket_writer);

        // Set the app configuration
        websocket.state.app = Some(app_config);

        // Create a WebSocketRef for thread-safe access
        let websocket_ref = WebSocketRef::new(websocket);

        // Store the connection in the central map
        self.sockets
            .insert(socket_id.clone(), websocket_ref.clone());

        debug!(socket_id = %socket_id, "WebSocket connection added successfully");

        Ok(websocket_ref)
    }

    // Retrieves a connection by SocketId.
    pub fn get_connection(&self, socket_id: &SocketId) -> Option<WebSocketRef> {
        self.sockets
            .get(socket_id)
            .map(|conn_ref| conn_ref.value().clone())
    }

    // Retrieves presence information for all members in a presence channel.
    pub async fn get_channel_members(
        &self,
        channel: &str,
    ) -> Result<HashMap<String, PresenceMemberInfo>> {
        let mut presence_members = HashMap::new();

        if let Some(socket_ids_ref) = self.channels.get(channel) {
            let socket_ids_snapshot = socket_ids_ref.clone();
            drop(socket_ids_ref);

            for socket_id_entry in socket_ids_snapshot.iter() {
                let socket_id = socket_id_entry.key();
                if let Some(connection) = self.get_connection(socket_id) {
                    let presence_data = {
                        let conn_guard = connection.inner.lock().await;
                        conn_guard
                            .state
                            .presence
                            .as_ref()
                            .and_then(|p_map| p_map.get(channel).cloned())
                    };
                    if let Some(presence_info) = presence_data {
                        presence_members.insert(presence_info.user_id.clone(), presence_info);
                    }
                }
            }
        } else {
            info!(
                "get_channel_members called on non-existent channel: {}",
                channel
            );
        }
        Ok(presence_members)
    }

    // Retrieves all socket IDs for sockets subscribed to a specific channel.
    pub fn get_channel_sockets(&self, channel: &str) -> DashSet<SocketId> {
        let sockets_in_channel = DashSet::<SocketId>::new();
        if let Some(channel_sockets_ref) = self.channels.get(channel) {
            let channel_sockets = channel_sockets_ref.value().clone();
            for socket_id in channel_sockets.iter() {
                sockets_in_channel.insert(socket_id.clone());
            }
        } else {
            debug!(
                "get_channel_sockets called on non-existent channel: {}",
                channel
            );
        }

        sockets_in_channel
    }

    // Get socket references for a channel with optional exclusion, minimizing lock contention
    pub fn get_channel_socket_refs_except(
        &self,
        channel: &str,
        except: Option<&SocketId>,
    ) -> Vec<WebSocketRef> {
        let mut socket_refs = Vec::new();

        if let Some(channel_sockets_ref) = self.channels.get(channel) {
            // Take snapshot of socket IDs, filtering out excluded socket
            let socket_ids: Vec<SocketId> = channel_sockets_ref
                .value()
                .iter()
                .filter_map(|entry| {
                    let socket_id = entry.key();
                    if except == Some(socket_id) {
                        None // Skip excluded socket
                    } else {
                        Some(socket_id.clone())
                    }
                })
                .collect();

            // Release the channel lock by dropping the reference
            drop(channel_sockets_ref);

            // Now get socket references without holding channel lock
            for socket_id in socket_ids {
                if let Some(socket_ref) = self.get_connection(&socket_id) {
                    socket_refs.push(socket_ref);
                }
            }
        } else {
            debug!(
                "get_channel_socket_refs_except called on non-existent channel: {}",
                channel
            );
        }

        socket_refs
    }

    // Retrieves references to WebSockets associated with a specific user ID.
    pub async fn get_user_sockets(&self, user_id: &str) -> Result<DashSet<WebSocketRef>> {
        match self.users.get(user_id) {
            Some(user_sockets_ref) => {
                let user_sockets = user_sockets_ref.clone();
                Ok(user_sockets)
            }
            None => Ok(DashSet::new()),
        }
    }

    // Cleans up a WebSocket connection: sends disconnect messages and removes from internal state.
    pub async fn cleanup_connection(&self, ws_ref: WebSocketRef) {
        let socket_id = ws_ref.get_socket_id().await;
        // Remove socket from all channels it was subscribed to.
        self.channels.retain(|_channel_name, socket_set| {
            socket_set.remove(&socket_id);
            !socket_set.is_empty() // Keep the channel entry if other sockets remain.
        });

        // Remove socket reference from user tracking.
        let user_id_option = ws_ref.get_user_id().await;

        if let Some(user_id) = user_id_option
            && let Some(user_sockets_ref) = self.users.get_mut(&user_id)
        {
            user_sockets_ref.remove(&ws_ref);
            let is_empty = user_sockets_ref.is_empty();
            drop(user_sockets_ref);
            if is_empty {
                self.users.remove(&user_id);
                debug!("Removed empty user entry for: {}", user_id);
            }
        }

        // Finally, remove the socket from the main sockets map.
        if self.sockets.remove(&socket_id).is_some() {
            debug!("Removed socket {} from main map.", socket_id);
        } else {
            warn!(
                "Socket {} already removed from main map during cleanup.",
                socket_id
            );
        }
    }

    // Terminates all connections associated with a specific user ID.
    pub async fn terminate_user_connections(&self, user_id: &str) -> Result<()> {
        if let Some(user_sockets_ref) = self.users.get(user_id) {
            let user_sockets_snapshot = user_sockets_ref.clone();
            drop(user_sockets_ref);

            let cleanup_tasks: Vec<_> = user_sockets_snapshot
                .iter()
                .map(|ws_ref| async move {
                    if let Err(e) = ws_ref
                        .close(4009, "User terminated by app.".to_string())
                        .await
                    {
                        warn!("Failed to close connection: {}", e);
                    }
                })
                .collect();

            // Wait for all cleanup tasks to complete.
            join_all(cleanup_tasks).await;
        }
        Ok(())
    }

    // Subscribes a socket to a channel. Returns true if the socket was newly added.
    pub fn add_channel_to_socket(&self, channel: &str, socket_id: &SocketId) -> bool {
        self.channels
            .entry(channel.to_string())
            .or_default()
            .insert(socket_id.clone())
    }

    // Unsubscribes a socket from a channel.
    pub fn remove_channel_from_socket(&self, channel: &str, socket_id: &SocketId) -> bool {
        if let Some(channel_sockets_ref) = self.channels.get_mut(channel) {
            let removed = channel_sockets_ref.remove(socket_id);
            let is_empty = channel_sockets_ref.is_empty();
            drop(channel_sockets_ref);
            if is_empty {
                self.channels.remove(channel);
                debug!("Removed empty channel entry: {}", channel);
            }
            return removed.is_some();
        }
        false
    }

    // Removes a connection entirely from the main socket map.
    pub fn remove_connection(&self, socket_id: &SocketId) {
        if self.sockets.remove(socket_id).is_some() {
            debug!("Explicitly removed socket: {}", socket_id);
        }
    }

    // Retrieves the set of socket IDs for a channel.
    pub fn get_channel(&self, channel: &str) -> Result<DashSet<SocketId>> {
        let channel_data = self.channels.entry(channel.to_string()).or_default();
        Ok(channel_data.value().clone())
    }

    // Removes a channel entry entirely, regardless of subscribers.
    pub fn remove_channel(&self, channel: &str) {
        self.channels.remove(channel);
        debug!("Removed channel entry: {}", channel);
    }

    // Checks if a specific socket is subscribed to a specific channel.
    pub fn is_in_channel(&self, channel: &str, socket_id: &SocketId) -> bool {
        self.channels
            .get(channel)
            .is_some_and(|channel_sockets| channel_sockets.contains(socket_id))
    }

    // Retrieves presence information for a specific socket within a channel.
    pub async fn get_presence_member(
        &self,
        channel: &str,
        socket_id: &SocketId,
    ) -> Option<PresenceMemberInfo> {
        if let Some(connection) = self.get_connection(socket_id) {
            let conn_guard = connection.inner.lock().await;
            conn_guard
                .state
                .presence
                .as_ref()
                .and_then(|presence_map| presence_map.get(channel))
                .cloned()
        } else {
            None
        }
    }

    // Associates an authenticated user with a WebSocket connection.
    pub async fn add_user(&self, ws_ref: WebSocketRef) -> Result<()> {
        let user_id_option = ws_ref.get_user_id().await;

        if let Some(user_id) = user_id_option {
            let user_sockets_ref = self.users.entry(user_id.clone()).or_default();
            user_sockets_ref.insert(ws_ref.clone());
            let socket_id = ws_ref.get_socket_id().await;
            debug!("Added socket {} to user {}", socket_id, user_id);
        } else {
            let socket_id = ws_ref.get_socket_id().await;
            warn!(
                "User data found for socket {} but missing user ID.",
                socket_id
            );
        }
        Ok(())
    }

    // Disassociates a user from a WebSocket connection upon disconnect or logout.
    pub async fn remove_user(&self, ws_ref: WebSocketRef) -> Result<()> {
        let user_id_option = ws_ref.get_user_id().await;
        let socket_id = ws_ref.get_socket_id().await;

        if let Some(user_id) = user_id_option {
            if let Some(user_sockets_ref) = self.users.get_mut(&user_id) {
                let removed = user_sockets_ref.remove(&ws_ref);
                let is_empty = user_sockets_ref.is_empty();
                drop(user_sockets_ref);
                if removed.is_some() {
                    debug!("Removed socket {} from user {}", socket_id, user_id);
                }
                if is_empty {
                    self.users.remove(&user_id);
                    debug!("Removed empty user entry for: {}", user_id);
                }
            }
        } else {
            warn!(
                "User data found for socket {} during removal but missing user ID.",
                socket_id
            );
        }
        Ok(())
    }

    pub async fn remove_user_socket(&self, user_id: &str, socket_id: &SocketId) -> Result<()> {
        if let Some(user_sockets_ref) = self.users.get_mut(user_id) {
            // Find and remove the first matching socket (socket_id should be unique)
            let mut found_socket = None;

            for ws_ref in user_sockets_ref.iter() {
                let ws_socket_id = ws_ref.get_socket_id().await;
                if ws_socket_id == *socket_id {
                    found_socket = Some(ws_ref.clone());
                    break;
                }
            }

            // Remove the matching socket if found
            if let Some(ws_ref) = found_socket {
                user_sockets_ref.remove(&ws_ref);
            }

            let is_empty = user_sockets_ref.is_empty();
            drop(user_sockets_ref);

            if is_empty {
                self.users.remove(user_id);
                debug!("Removed empty user entry for: {}", user_id);
            }

            debug!("Removed socket {} from user {}", socket_id, user_id);
        } else {
            debug!(
                "User {} not found when removing socket {}",
                user_id, socket_id
            );
        }
        Ok(())
    }

    pub async fn count_user_connections_in_channel(
        &self,
        user_id: &str,
        channel: &str,
        excluding_socket: Option<&SocketId>,
    ) -> Result<usize> {
        let mut count = 0;

        if let Some(user_sockets_ref) = self.users.get(user_id) {
            for ws_ref in user_sockets_ref.iter() {
                let socket_state_guard = ws_ref.inner.lock().await;
                let socket_id = &socket_state_guard.state.socket_id;
                let is_subscribed = socket_state_guard.state.is_subscribed(channel);

                // Skip this socket if it's the one we're excluding
                let should_exclude = excluding_socket == Some(socket_id);

                if is_subscribed && !should_exclude {
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    // Retrieves a map of channel names to their current subscriber counts.
    pub async fn get_channels_with_socket_count(&self) -> Result<DashMap<String, usize>> {
        let channels_with_count: DashMap<String, usize> = DashMap::new();
        for channel_ref in self.channels.iter() {
            let channel_name = channel_ref.key().clone();
            let socket_count = channel_ref.value().len();
            channels_with_count.insert(channel_name, socket_count);
        }
        Ok(channels_with_count)
    }

    // Updated to return WebSocketRef instead of Arc<Mutex<WebSocket>>
    pub async fn get_sockets(&self) -> Result<DashMap<SocketId, WebSocketRef>> {
        let sockets = self.sockets.clone();
        Ok(sockets)
    }
}
