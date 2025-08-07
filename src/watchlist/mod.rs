pub mod manager;

// src/watchlist/manager.rs
use crate::error::Result;
use crate::protocol::messages::PusherMessage;
use crate::websocket::SocketId;
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct WatchlistEntry {
    pub user_id: String,
    pub watchers: HashSet<String>, // Users watching this user
    pub watching: HashSet<String>, // Users this user is watching
}

pub struct WatchlistManager {
    // Map: app_id -> user_id -> WatchlistEntry
    watchlists: Arc<DashMap<String, DashMap<String, WatchlistEntry>>>,
    // Map: app_id -> user_id -> Set<socket_ids>
    online_users: Arc<DashMap<String, DashMap<String, HashSet<SocketId>>>>,
}

impl Default for WatchlistManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchlistManager {
    pub fn new() -> Self {
        Self {
            watchlists: Arc::new(DashMap::new()),
            online_users: Arc::new(DashMap::new()),
        }
    }

    /// Add a user with their watchlist
    pub async fn add_user_with_watchlist(
        &self,
        app_id: &str,
        user_id: &str,
        socket_id: SocketId,
        watchlist: Option<Vec<String>>,
    ) -> Result<Vec<PusherMessage>> {
        let mut events_to_send = Vec::new();

        // Initialize app-level maps if they don't exist
        if !self.watchlists.contains_key(app_id) {
            self.watchlists.insert(app_id.to_string(), DashMap::new());
        }
        if !self.online_users.contains_key(app_id) {
            self.online_users.insert(app_id.to_string(), DashMap::new());
        }

        let app_watchlists = self.watchlists.get(app_id).unwrap();
        let app_online_users = self.online_users.get(app_id).unwrap();

        // Track this socket as online for the user
        let was_offline = {
            let mut user_sockets = app_online_users.entry(user_id.to_string()).or_default();
            let was_empty = user_sockets.is_empty();
            user_sockets.insert(socket_id);
            was_empty
        };

        // Update watchlist relationships
        if let Some(watchlist_vec) = watchlist {
            let watching_set: HashSet<String> = watchlist_vec.into_iter().collect();

            // Update this user's entry
            let mut user_entry = app_watchlists
                .entry(user_id.to_string())
                .or_insert_with(|| WatchlistEntry {
                    user_id: user_id.to_string(),
                    watchers: HashSet::new(),
                    watching: watching_set.clone(),
                });
            user_entry.watching = watching_set.clone();

            // Update reverse relationships (add this user as a watcher)
            for watched_user_id in &watching_set {
                let mut watched_entry = app_watchlists
                    .entry(watched_user_id.clone())
                    .or_insert_with(|| WatchlistEntry {
                        user_id: watched_user_id.clone(),
                        watchers: HashSet::new(),
                        watching: HashSet::new(),
                    });
                watched_entry.watchers.insert(user_id.to_string());
            }

            // If user just came online, notify their watchers
            if was_offline {
                if let Some(user_entry) = app_watchlists.get(user_id) {
                    for _ in &user_entry.watchers {
                        events_to_send.push(PusherMessage::watchlist_online_event(vec![
                            user_id.to_string(),
                        ]));
                    }
                }
            }

            // Send current online status of watched users to this user
            let mut online_watched_users = Vec::new();
            let mut offline_watched_users = Vec::new();

            for watched_user_id in &watching_set {
                if app_online_users.contains_key(watched_user_id)
                    && !app_online_users.get(watched_user_id).unwrap().is_empty()
                {
                    online_watched_users.push(watched_user_id.clone());
                } else {
                    offline_watched_users.push(watched_user_id.clone());
                }
            }

            if !online_watched_users.is_empty() {
                events_to_send.push(PusherMessage::watchlist_online_event(online_watched_users));
            }
            if !offline_watched_users.is_empty() {
                events_to_send.push(PusherMessage::watchlist_offline_event(
                    offline_watched_users,
                ));
            }
        }

        Ok(events_to_send)
    }

    /// Remove a user connection
    pub async fn remove_user_connection(
        &self,
        app_id: &str,
        user_id: &str,
        socket_id: &SocketId,
    ) -> Result<Vec<PusherMessage>> {
        let mut events_to_send = Vec::new();

        if let Some(app_online_users) = self.online_users.get(app_id) {
            if let Some(mut user_sockets) = app_online_users.get_mut(user_id) {
                user_sockets.remove(socket_id);

                // If user has no more connections, they're offline
                if user_sockets.is_empty() {
                    // Notify watchers that this user went offline
                    if let Some(app_watchlists) = self.watchlists.get(app_id) {
                        if let Some(user_entry) = app_watchlists.get(user_id) {
                            for _ in &user_entry.watchers {
                                events_to_send.push(PusherMessage::watchlist_offline_event(vec![
                                    user_id.to_string(),
                                ]));
                            }
                        }
                    }
                }
            }
        }

        Ok(events_to_send)
    }

    /// Get online status for a user's watchlist
    pub async fn get_watchlist_status(
        &self,
        app_id: &str,
        user_id: &str,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let mut online_users = Vec::new();
        let mut offline_users = Vec::new();

        if let Some(app_watchlists) = self.watchlists.get(app_id) {
            if let Some(user_entry) = app_watchlists.get(user_id) {
                if let Some(app_online_users) = self.online_users.get(app_id) {
                    for watched_user_id in &user_entry.watching {
                        if app_online_users.contains_key(watched_user_id)
                            && !app_online_users.get(watched_user_id).unwrap().is_empty()
                        {
                            online_users.push(watched_user_id.clone());
                        } else {
                            offline_users.push(watched_user_id.clone());
                        }
                    }
                }
            }
        }

        Ok((online_users, offline_users))
    }

    /// Clean up when app is removed
    pub async fn cleanup_app(&self, app_id: &str) {
        self.watchlists.remove(app_id);
        self.online_users.remove(app_id);
    }

    pub async fn get_watchers_for_user(&self, app_id: &str, user_id: &str) -> Result<Vec<String>> {
        let mut watchers = Vec::new();

        if let Some(app_watchlists) = self.watchlists.get(app_id) {
            if let Some(user_entry) = app_watchlists.get(user_id) {
                watchers.extend(user_entry.watchers.iter().cloned());
            }
        }

        Ok(watchers)
    }
}
