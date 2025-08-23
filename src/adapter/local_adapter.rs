use crate::adapter::ConnectionManager;
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};

use crate::namespace::Namespace;
use crate::protocol::messages::PusherMessage;
use crate::websocket::{SocketId, WebSocketRef};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::WriteHalf;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct LocalAdapter {
    pub namespaces: DashMap<String, Arc<Namespace>>,
    pub broadcast_streaming_threshold: usize,
}

impl Default for LocalAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalAdapter {
    pub fn new() -> Self {
        Self {
            namespaces: DashMap::new(),
            broadcast_streaming_threshold: 1500, // Default value
        }
    }

    pub fn new_with_threshold(threshold: usize) -> Self {
        Self {
            namespaces: DashMap::new(),
            broadcast_streaming_threshold: threshold,
        }
    }

    /// Send messages to sockets in batches with controlled concurrency
    async fn send_batched_messages(
        &self,
        target_socket_refs: Vec<WebSocketRef>,
        message_bytes: Arc<Vec<u8>>,
    ) -> Vec<std::result::Result<Vec<Result<()>>, tokio::task::JoinError>> {
        // Determine optimal batch size and concurrency based on socket count
        let socket_count = target_socket_refs.len();
        let (batch_size, max_concurrent_batches) = if socket_count <= 100 {
            (socket_count, 1) // Small broadcasts: no batching needed
        } else if socket_count <= 1000 {
            (50, 4) // Medium broadcasts: small batches, limited concurrency
        } else {
            (100, 8) // Large broadcasts: larger batches, more concurrency
        };

        // Create semaphore to limit concurrent batch processing
        let semaphore = Arc::new(Semaphore::new(max_concurrent_batches));

        // Split sockets into batches and spawn tasks
        let batch_tasks: Vec<JoinHandle<Vec<Result<()>>>> = target_socket_refs
            .chunks(batch_size)
            .map(|batch| {
                let batch = batch.to_vec();
                let bytes = message_bytes.clone();
                let permit = semaphore.clone();

                tokio::spawn(async move {
                    let _permit = permit.acquire().await.expect("Semaphore closed");
                    let mut batch_results = Vec::with_capacity(batch.len());

                    // Process all sockets in this batch sequentially to reduce task overhead
                    for socket_ref in batch {
                        let result = socket_ref.send_raw_message(bytes.clone()).await;
                        batch_results.push(result);
                    }

                    batch_results
                })
            })
            .collect();

        // Wait for all batch tasks to complete and flatten results

        join_all(batch_tasks).await
    }

    /// Send messages using streaming pipeline with buffer_unordered for optimal throughput
    async fn send_streaming_messages(
        &self,
        target_socket_refs: Vec<WebSocketRef>,
        message_bytes: Arc<Vec<u8>>,
    ) -> Vec<Result<()>> {
        let socket_count = target_socket_refs.len();

        // Determine optimal buffer size based on socket count and system capabilities
        let buffer_size = if socket_count <= 100 {
            socket_count.min(16) // Small broadcasts: limited buffering
        } else if socket_count <= 1000 {
            64 // Medium broadcasts: moderate buffering
        } else {
            128 // Large broadcasts: high buffering for maximum throughput
        };

        // Create streaming pipeline using buffer_unordered for concurrent processing
        let results: Vec<Result<()>> =
            stream::iter(target_socket_refs.into_iter().map(|socket_ref| {
                let bytes = message_bytes.clone();
                async move { socket_ref.send_raw_message(bytes).await }
            }))
            .buffer_unordered(buffer_size)
            .collect()
            .await;

        results
    }

    /// Hybrid approach: choose between streaming and batching based on subscriber count and threshold
    async fn send_hybrid_messages(
        &self,
        target_socket_refs: Vec<WebSocketRef>,
        message_bytes: Arc<Vec<u8>>,
    ) -> Vec<Result<()>> {
        let subscriber_count = target_socket_refs.len();

        if subscriber_count <= self.broadcast_streaming_threshold {
            // Use streaming pipeline for smaller broadcasts
            self.send_streaming_messages(target_socket_refs, message_bytes)
                .await
        } else {
            // Use batched processing for larger broadcasts
            let batch_results = self
                .send_batched_messages(target_socket_refs, message_bytes)
                .await;

            // Flatten batched results to match streaming format
            let mut flattened_results = Vec::new();
            for batch_result in batch_results {
                match batch_result {
                    Ok(results) => flattened_results.extend(results),
                    Err(_) => {
                        // If batch task failed, assume all sends in that batch failed
                        // This is a simplification - in practice we'd want better error handling
                        flattened_results.push(Err(crate::error::Error::Connection(
                            "Batch task failed".to_string(),
                        )));
                    }
                }
            }
            flattened_results
        }
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
    ) -> Result<()> {
        let start_time = std::time::Instant::now();
        debug!("Sending message to channel: {}", channel);
        debug!("Message: {:?}", message);

        if channel.starts_with("#server-to-user-") {
            let user_id = channel.trim_start_matches("#server-to-user-");
            let namespace = self.get_namespace(app_id).await.unwrap();
            let socket_refs = namespace.get_user_sockets(user_id).await?;

            // Collect socket IDs that should receive the message, using WebSocketRef methods
            let mut target_socket_refs = Vec::new();
            for socket_ref in socket_refs.iter() {
                let socket_id = socket_ref.get_socket_id().await;
                if except != Some(&socket_id) {
                    target_socket_refs.push(socket_ref.clone());
                }
            }

            let subscriber_count = target_socket_refs.len();
            info!(
                "Broadcasting to user channel '{}': {} user sockets",
                channel, subscriber_count
            );

            // Serialize message once to avoid repeated serialization overhead
            let message_bytes =
                Arc::new(serde_json::to_vec(&message).map_err(|e| {
                    Error::InvalidMessageFormat(format!("Serialization failed: {e}"))
                })?);

            // Send messages using hybrid approach (streaming for small, batching for large)
            let results = self
                .send_hybrid_messages(target_socket_refs, message_bytes)
                .await;

            let elapsed = start_time.elapsed();
            info!(
                "User broadcast completed for channel '{}': {} sockets in {:?} ({:.2}ms)",
                channel,
                subscriber_count,
                elapsed,
                elapsed.as_secs_f64() * 1000.0
            );

            // Handle any errors from the hybrid messaging
            for send_result in results {
                if let Err(e) = send_result {
                    error!("Failed to send message to user socket: {}", e);
                }
            }
        } else {
            let namespace = self.get_namespace(app_id).await.unwrap();

            // Get socket references with exclusion handled efficiently during collection
            let target_socket_refs = namespace.get_channel_socket_refs_except(channel, except);

            let subscriber_count = target_socket_refs.len();
            info!(
                "Broadcasting to channel '{}': {} subscribers",
                channel, subscriber_count
            );

            // Serialize message once to avoid repeated serialization overhead
            let message_bytes =
                Arc::new(serde_json::to_vec(&message).map_err(|e| {
                    Error::InvalidMessageFormat(format!("Serialization failed: {e}"))
                })?);

            // Send messages using hybrid approach (streaming for small, batching for large)
            let results = self
                .send_hybrid_messages(target_socket_refs, message_bytes)
                .await;

            let elapsed = start_time.elapsed();
            info!(
                "Broadcast completed for channel '{}': {} subscribers in {:?} ({:.2}ms)",
                channel,
                subscriber_count,
                elapsed,
                elapsed.as_secs_f64() * 1000.0
            );

            // Handle any errors from the hybrid messaging
            for send_result in results {
                if let Err(e) = send_result {
                    error!("Failed to send message to channel socket: {}", e);
                }
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

    async fn batch_remove_from_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_ids: &[SocketId],
    ) -> Result<usize> {
        let namespace = self.get_or_create_namespace(app_id).await;
        Ok(namespace.batch_remove_from_channel(channel, socket_ids))
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
            let ws_guard = ws_ref.0.lock().await;
            ws_guard.state.get_app_key()
        };
        let namespace = self.get_namespace(&app_id).await.unwrap();
        namespace.add_user(ws_ref).await
    }

    // Updated to use WebSocketRef
    async fn remove_user(&mut self, ws_ref: WebSocketRef) -> Result<()> {
        // Get app_id using WebSocketRef async method
        let app_id = {
            let ws_guard = ws_ref.0.lock().await;
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
