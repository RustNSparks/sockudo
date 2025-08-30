use crate::adapter::ConnectionManager;
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};

use crate::namespace::Namespace;
use crate::protocol::messages::PusherMessage;
use crate::websocket::{SocketId, WebSocketRef};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::WriteHalf;
use tokio::sync::Semaphore;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct LocalAdapter {
    pub namespaces: DashMap<String, Arc<Namespace>>,
    pub buffer_multiplier_per_cpu: usize,
    pub max_concurrent: usize,
    // Global semaphore to limit total concurrent broadcast operations across all channels
    broadcast_semaphore: Arc<Semaphore>,
}

impl Default for LocalAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalAdapter {
    pub fn new() -> Self {
        Self::new_with_buffer_multiplier(128) // Default: 64 concurrent ops per CPU
    }

    pub fn new_with_buffer_multiplier(multiplier: usize) -> Self {
        let cpu_cores = num_cpus::get();
        let max_concurrent = cpu_cores * multiplier;

        info!(
            "LocalAdapter initialized with {} CPU cores, buffer multiplier {}, max concurrent {}",
            cpu_cores, multiplier, max_concurrent
        );

        Self {
            namespaces: DashMap::new(),
            buffer_multiplier_per_cpu: multiplier,
            max_concurrent,
            broadcast_semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Send messages using chunked processing with semaphore-controlled concurrency
    async fn send_messages_concurrent(
        &self,
        target_socket_refs: Vec<WebSocketRef>,
        message_bytes: Bytes,
    ) -> Vec<Result<()>> {
        use futures::stream::{self, StreamExt};

        let socket_count = target_socket_refs.len();

        // Determine target number of chunks (1-8 based on socket count vs max concurrency)
        let target_chunks = socket_count.div_ceil(self.max_concurrent).clamp(1, 8);

        // Calculate socket chunk size based on socket count divided by target chunks
        // With a max of self.max_concurrent sockets per chunk (better utilization)
        let socket_chunk_size = (socket_count / target_chunks)
            .min(self.max_concurrent)
            .max(1);

        // Process chunks sequentially with controlled concurrency
        let mut results = Vec::with_capacity(socket_count);

        for socket_chunk in target_socket_refs.chunks(socket_chunk_size) {
            let chunk_size = socket_chunk.len();

            // Acquire permits for the entire chunk
            match self
                .broadcast_semaphore
                .acquire_many(chunk_size as u32)
                .await
            {
                Ok(_permits) => {
                    // Process sockets in this chunk using buffered unordered streaming
                    let chunk_vec: Vec<_> = socket_chunk.to_vec();
                    let chunk_results: Vec<Result<()>> = stream::iter(chunk_vec)
                        .map(|socket_ref| {
                            let bytes = message_bytes.clone();
                            async move { socket_ref.send_broadcast(bytes) }
                        })
                        .buffer_unordered(chunk_size)
                        .collect()
                        .await;

                    results.extend(chunk_results);
                }
                Err(_) => {
                    // Return errors for all sockets if semaphore fails
                    for _ in 0..chunk_size {
                        results.push(Err(Error::Connection(
                            "Broadcast semaphore unavailable".to_string(),
                        )));
                    }
                }
            }
        }

        results
    }

    // Helper function to get or create namespace
    async fn get_or_create_namespace(&mut self, app_id: &str) -> Arc<Namespace> {
        if !self.namespaces.contains_key(app_id) {
            let namespace = Arc::new(Namespace::new(app_id.to_string()));
            self.namespaces.insert(app_id.to_string(), namespace);
        }
        self.namespaces.get(app_id).unwrap().clone()
    }

    // Updated to return WebSocketRef instead of Arc<Mutex<WebSocket>>
    pub async fn get_all_connections(&mut self, app_id: &str) -> DashMap<SocketId, WebSocketRef> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.sockets.clone()
    }
}

#[async_trait]
impl ConnectionManager for LocalAdapter {
    async fn init(&mut self) {
        info!("Initializing local adapter");
    }

    async fn get_namespace(&mut self, app_id: &str) -> Option<Arc<Namespace>> {
        Some(self.get_or_create_namespace(app_id).await)
    }

    async fn add_socket(
        &mut self,
        socket_id: SocketId,
        socket: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>,
        app_id: &str,
        app_manager: &Arc<dyn AppManager + Send + Sync>,
    ) -> Result<()> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.add_socket(socket_id, socket, app_manager).await?;
        Ok(())
    }

    // Updated to return WebSocketRef instead of Arc<Mutex<WebSocket>>
    async fn get_connection(&mut self, socket_id: &SocketId, app_id: &str) -> Option<WebSocketRef> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_connection(socket_id)
    }

    async fn remove_connection(&mut self, socket_id: &SocketId, app_id: &str) -> Result<()> {
        if let Some(namespace) = self.namespaces.get(app_id) {
            namespace.remove_connection(socket_id);
            Ok(())
        } else {
            Err(Error::Connection("Namespace not found".to_string()))
        }
    }

    // Updated to use WebSocketRef methods
    async fn send_message(
        &mut self,
        app_id: &str,
        socket_id: &SocketId,
        message: PusherMessage,
    ) -> Result<()> {
        let connection = self
            .get_connection(socket_id, app_id)
            .await
            .ok_or_else(|| Error::Connection("Connection not found".to_string()))?;

        connection.send_message(&message).await
    }

    async fn send(
        &mut self,
        channel: &str,
        message: PusherMessage,
        except: Option<&SocketId>,
        app_id: &str,
        _start_time_ms: Option<f64>,
    ) -> Result<()> {
        debug!("Sending message to channel: {}", channel);
        debug!("Message: {:?}", message);

        // Serialize message once to avoid repeated serialization overhead
        let serialized_message = serde_json::to_vec(&message)
            .map_err(|e| Error::InvalidMessageFormat(format!("Serialization failed: {e}")))?;
        let message_bytes = Bytes::from(serialized_message);

        let namespace = self.get_namespace(app_id).await.unwrap();

        // Get target socket references based on channel type
        let target_socket_refs = if channel.starts_with("#server-to-user-") {
            let user_id = channel.trim_start_matches("#server-to-user-");
            let socket_refs = namespace.get_user_sockets(user_id).await?;

            // Collect socket IDs that should receive the message, applying exclusion
            let mut target_refs = Vec::new();
            for socket_ref in socket_refs.iter() {
                let socket_id = socket_ref.get_socket_id().await;
                if except != Some(&socket_id) {
                    target_refs.push(socket_ref.clone());
                }
            }
            target_refs
        } else {
            // Get socket references with exclusion handled efficiently during collection
            namespace.get_channel_socket_refs_except(channel, except)
        };

        // Send messages using concurrent tasks with semaphore-controlled concurrency
        let results = self
            .send_messages_concurrent(target_socket_refs, message_bytes)
            .await;

        // Handle any errors from concurrent messaging
        for send_result in results {
            if let Err(e) = send_result {
                error!("Failed to send message: {}", e);
            }
        }

        Ok(())
    }

    async fn get_channel_members(
        &mut self,
        app_id: &str,
        channel: &str,
    ) -> Result<HashMap<String, PresenceMemberInfo>> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_channel_members(channel).await
    }

    async fn get_channel_sockets(
        &mut self,
        app_id: &str,
        channel: &str,
    ) -> Result<DashSet<SocketId>> {
        let namespace = self.get_or_create_namespace(app_id).await;
        Ok(namespace.get_channel_sockets(channel))
    }

    async fn remove_channel(&mut self, app_id: &str, channel: &str) {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.remove_channel(channel);
    }

    async fn is_in_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        let namespace = self.get_or_create_namespace(app_id).await;
        Ok(namespace.is_in_channel(channel, socket_id))
    }

    async fn get_user_sockets(
        &mut self,
        user_id: &str,
        app_id: &str,
    ) -> Result<DashSet<WebSocketRef>> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_user_sockets(user_id).await
    }

    async fn cleanup_connection(&mut self, app_id: &str, ws: WebSocketRef) {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.cleanup_connection(ws).await;
    }

    async fn terminate_connection(&mut self, app_id: &str, user_id: &str) -> Result<()> {
        let namespace = self.get_or_create_namespace(app_id).await;
        if let Err(e) = namespace.terminate_user_connections(user_id).await {
            error!("Failed to terminate adapter: {}", e);
        }
        Ok(())
    }

    async fn add_channel_to_sockets(&mut self, app_id: &str, channel: &str, socket_id: &SocketId) {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.add_channel_to_socket(channel, socket_id);
    }

    async fn get_channel_socket_count(&mut self, app_id: &str, channel: &str) -> usize {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_channel_sockets(channel).len()
    }

    async fn add_to_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        let namespace = self.get_or_create_namespace(app_id).await;
        Ok(namespace.add_channel_to_socket(channel, socket_id))
    }

    async fn remove_from_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        let namespace = self.get_or_create_namespace(app_id).await;
        Ok(namespace.remove_channel_from_socket(channel, socket_id))
    }

    async fn get_presence_member(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Option<PresenceMemberInfo> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_presence_member(channel, socket_id).await
    }

    async fn terminate_user_connections(&mut self, app_id: &str, user_id: &str) -> Result<()> {
        let namespace = self.get_or_create_namespace(app_id).await;
        if let Err(e) = namespace.terminate_user_connections(user_id).await {
            error!("Failed to terminate user connections: {}", e);
        }
        Ok(())
    }

    // Updated to use WebSocketRef
    async fn add_user(&mut self, ws_ref: WebSocketRef) -> Result<()> {
        // Get app_id using WebSocketRef async method
        let app_id = {
            let ws_guard = ws_ref.inner.lock().await;
            ws_guard.state.get_app_key()
        };
        let namespace = self.get_namespace(&app_id).await.unwrap();
        namespace.add_user(ws_ref).await
    }

    // Updated to use WebSocketRef
    async fn remove_user(&mut self, ws_ref: WebSocketRef) -> Result<()> {
        // Get app_id using WebSocketRef async method
        let app_id = {
            let ws_guard = ws_ref.inner.lock().await;
            ws_guard.state.get_app_key()
        };
        let namespace = self.get_namespace(&app_id).await.unwrap();
        namespace.remove_user(ws_ref).await
    }

    async fn get_channels_with_socket_count(
        &mut self,
        app_id: &str,
    ) -> Result<DashMap<String, usize>> {
        let namespace = self.get_or_create_namespace(app_id).await;
        namespace.get_channels_with_socket_count().await
    }

    async fn get_sockets_count(&self, app_id: &str) -> Result<usize> {
        if let Some(namespace) = self.namespaces.get(app_id) {
            let count = namespace.sockets.len();
            Ok(count)
        } else {
            Ok(0) // No namespace means no sockets
        }
    }

    async fn get_namespaces(&mut self) -> Result<DashMap<String, Arc<Namespace>>> {
        let namespaces = DashMap::new();
        for entry in self.namespaces.iter() {
            namespaces.insert(entry.key().clone(), entry.value().clone());
        }
        Ok(namespaces)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    async fn check_health(&self) -> Result<()> {
        // Local adapter is always healthy since it's in-memory
        Ok(())
    }
}
