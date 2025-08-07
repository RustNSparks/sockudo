use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use redis::AsyncCommands;
use redis::cluster::{ClusterClient, ClusterClientBuilder};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::WriteHalf;
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::adapter::ConnectionManager;
use crate::adapter::horizontal_adapter::{
    BroadcastMessage, HorizontalAdapter, PendingRequest, RequestBody, RequestType, ResponseBody,
};
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};
use crate::metrics::MetricsInterface;
use crate::namespace::Namespace;
pub(crate) use crate::options::RedisClusterAdapterConfig;
use crate::protocol::messages::PusherMessage;
use crate::websocket::{SocketId, WebSocketRef};

/// Redis channels
pub const DEFAULT_PREFIX: &str = "sockudo";
const BROADCAST_SUFFIX: &str = "#broadcast";
const REQUESTS_SUFFIX: &str = "#requests";
const RESPONSES_SUFFIX: &str = "#responses";

/// Redis Cluster adapter for horizontal scaling
pub struct RedisClusterAdapter {
    pub horizontal: Arc<Mutex<HorizontalAdapter>>,
    pub client: ClusterClient,
    pub connection: redis::cluster_async::ClusterConnection,
    pub prefix: String,
    pub broadcast_channel: String,
    pub request_channel: String,
    pub response_channel: String,
    pub config: RedisClusterAdapterConfig,
}

impl RedisClusterAdapter {
    pub async fn new(config: RedisClusterAdapterConfig) -> Result<Self> {
        let mut horizontal = HorizontalAdapter::new();
        horizontal.requests_timeout = config.request_timeout_ms;

        let client = ClusterClient::new(config.nodes.clone())
            .map_err(|e| Error::Redis(format!("Failed to create Redis client: {e}")))?;

        let connection = client
            .get_async_connection()
            .await
            .map_err(|e| Error::Redis(format!("Failed to connect to Redis: {e}")))?;

        let broadcast_channel = format!("{}:{}", config.prefix, BROADCAST_SUFFIX);
        let request_channel = format!("{}:{}", config.prefix, REQUESTS_SUFFIX);
        let response_channel = format!("{}:{}", config.prefix, RESPONSES_SUFFIX);

        Ok(Self {
            horizontal: Arc::new(Mutex::new(horizontal)),
            client,
            connection,
            prefix: config.prefix.clone(),
            broadcast_channel,
            request_channel,
            response_channel,
            config,
        })
    }

    pub async fn with_nodes(nodes: Vec<String>) -> Result<Self> {
        let config = RedisClusterAdapterConfig {
            nodes,
            ..Default::default()
        };
        Self::new(config).await
    }

    pub async fn set_metrics(
        &mut self,
        metrics: Arc<Mutex<dyn MetricsInterface + Send + Sync>>,
    ) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.metrics = Some(metrics);
        Ok(())
    }

    /// Enhanced send_request that properly integrates with HorizontalAdapter
    pub async fn send_request(
        &self,
        app_id: &str,
        request_type: RequestType,
        channel: Option<&str>,
        socket_id: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<ResponseBody> {
        let node_count = self.get_node_count().await?;

        // Create the request
        let request_id = Uuid::new_v4().to_string();
        let node_id = {
            let horizontal = self.horizontal.lock().await;
            horizontal.node_id.clone()
        };

        let request = RequestBody {
            request_id: request_id.clone(),
            node_id,
            app_id: app_id.to_string(),
            request_type: request_type.clone(),
            channel: channel.map(String::from),
            socket_id: socket_id.map(String::from),
            user_id: user_id.map(String::from),
        };

        // Add to pending requests
        {
            let horizontal = self.horizontal.lock().await;
            horizontal.pending_requests.insert(
                request_id.clone(),
                PendingRequest {
                    start_time: Instant::now(),
                    app_id: app_id.to_string(),
                    responses: Vec::with_capacity(node_count.saturating_sub(1)),
                    notify: Arc::new(Notify::new()),
                },
            );

            if let Some(metrics_ref) = &horizontal.metrics {
                let metrics = metrics_ref.lock().await;
                metrics.mark_horizontal_adapter_request_sent(app_id);
            }
        }

        // Broadcast the request via Redis
        self.broadcast_request(&request).await?;

        // Wait for responses
        let timeout_duration = Duration::from_millis(self.config.request_timeout_ms);
        let max_expected_responses = node_count.saturating_sub(1);

        if max_expected_responses == 0 {
            self.horizontal
                .lock()
                .await
                .pending_requests
                .remove(&request_id);
            return Ok(ResponseBody {
                request_id,
                node_id: request.node_id,
                app_id: app_id.to_string(),
                members: HashMap::new(),
                socket_ids: Vec::new(),
                sockets_count: 0,
                channels_with_sockets_count: HashMap::new(),
                exists: false,
                channels: HashSet::new(),
                members_count: 0,
            });
        }

        // Wait for responses using event-driven approach
        let start = Instant::now();
        let notify = {
            let horizontal = self.horizontal.lock().await;
            horizontal
                .pending_requests
                .get(&request_id)
                .map(|req| req.notify.clone())
                .ok_or_else(|| {
                    Error::Other(format!(
                        "Request {request_id} not found in pending requests"
                    ))
                })?
        };

        let responses = loop {
            // Wait for notification or timeout
            let result = tokio::select! {
                _ = notify.notified() => {
                    // Check if we have enough responses
                    let horizontal = self.horizontal.lock().await;
                    if let Some(pending_request) = horizontal.pending_requests.get(&request_id) {
                        if pending_request.responses.len() >= max_expected_responses {
                            debug!(
                                "Request {} completed with {}/{} responses in {}ms",
                                request_id,
                                pending_request.responses.len(),
                                max_expected_responses,
                                start.elapsed().as_millis()
                            );
                            // Extract responses without removing the entry yet to avoid race condition
                            let responses = pending_request.responses.clone();
                            Some(responses)
                        } else {
                            None // Continue waiting
                        }
                    } else {
                        return Err(Error::Other(format!(
                            "Request {request_id} was removed unexpectedly"
                        )));
                    }
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timeout occurred
                    warn!(
                        "Request {} timed out after {}ms",
                        request_id,
                        start.elapsed().as_millis()
                    );
                    let horizontal = self.horizontal.lock().await;
                    let responses = if let Some(pending_request) = horizontal.pending_requests.get(&request_id) {
                        pending_request.responses.clone()
                    } else {
                        Vec::new()
                    };
                    Some(responses)
                }
            };

            if let Some(responses) = result {
                break responses;
            }
            // If result is None, continue waiting (notification came but not enough responses yet)
        };

        // Clean up the pending request now that we're done
        {
            let horizontal = self.horizontal.lock().await;
            horizontal.pending_requests.remove(&request_id);
        }

        // Aggregate responses
        let combined_response = {
            let horizontal = self.horizontal.lock().await;
            horizontal.aggregate_responses(
                request_id,
                request.node_id,
                app_id.to_string(),
                &request_type,
                responses,
            )
        }; // horizontal lock released here

        // Track metrics
        {
            let horizontal = self.horizontal.lock().await;
            if let Some(metrics_ref) = &horizontal.metrics {
                let metrics = metrics_ref.lock().await;
                let duration_ms = start.elapsed().as_millis() as f64;
                metrics.track_horizontal_adapter_resolve_time(app_id, duration_ms);

                let resolved = combined_response.sockets_count > 0
                    || !combined_response.members.is_empty()
                    || combined_response.exists
                    || !combined_response.channels.is_empty()
                    || combined_response.members_count > 0
                    || !combined_response.channels_with_sockets_count.is_empty();

                metrics.track_horizontal_adapter_resolved_promises(app_id, resolved);
            }
        } // horizontal and metrics locks released here

        Ok(combined_response)
    }

    async fn broadcast_request(&self, request: &RequestBody) -> Result<()> {
        let request_json = serde_json::to_string(request)
            .map_err(|e| Error::Other(format!("Failed to serialize request: {e}")))?;

        let mut conn = self.connection.clone();
        let subscriber_count: i32 = conn
            .publish(&self.request_channel, &request_json)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish request: {e}")))?;

        debug!(
            "Broadcasted request {} to {} subscribers",
            request.request_id, subscriber_count
        );

        Ok(())
    }

    pub async fn start_listeners(&self) -> Result<()> {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.start_request_cleanup();
        }

        self.start_listeners_pubsub().await?;
        Ok(())
    }

    async fn start_listeners_pubsub(&self) -> Result<()> {
        // Clone needed values for the async task
        let horizontal_arc = self.horizontal.clone();
        let pub_connection = self.connection.clone();
        let broadcast_channel = self.broadcast_channel.clone();
        let request_channel = self.request_channel.clone();
        let response_channel = self.response_channel.clone();
        let nodes = self.config.nodes.clone();

        // Get node_id without holding the lock for the whole setup
        let node_id = {
            let horizontal_lock = horizontal_arc.lock().await;
            horizontal_lock.node_id.clone()
        };

        // Create a separate channel for receiving PubSub messages
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Create a new client with RESP3 protocol for PubSub
        let sub_client = ClusterClientBuilder::new(nodes)
            .use_protocol(redis::ProtocolVersion::RESP3)
            .push_sender(tx)
            .build()
            .map_err(|e| Error::Redis(format!("Failed to create PubSub client: {e}")))?;

        // Spawn the main listener task
        tokio::spawn(async move {
            // Create a connection for PubSub
            let mut pubsub = match sub_client.get_async_connection().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Failed to get pubsub connection: {}", e);
                    return;
                }
            };

            // Subscribe to all channels
            if let Err(e) = pubsub
                .subscribe(&[&broadcast_channel, &request_channel, &response_channel])
                .await
            {
                error!("Failed to subscribe to channels: {}", e);
                return;
            }

            debug!(
                "Redis Cluster adapter listening on channels: {}, {}, {}",
                broadcast_channel, request_channel, response_channel
            );

            // Process messages from the channel - PushInfo is the message type for RESP3
            while let Some(push_info) = rx.recv().await {
                // Extract channel and payload from PushInfo
                if push_info.kind != redis::PushKind::Message {
                    continue; // Skip non-message push notifications
                }

                // PushInfo.data for messages should be [channel, payload]
                if push_info.data.len() < 2 {
                    error!("Invalid push message format: {:?}", push_info);
                    continue;
                }

                let channel = match &push_info.data[0] {
                    redis::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
                        Ok(s) => s,
                        Err(_) => {
                            error!("Failed to parse channel name from bulk string bytes");
                            continue;
                        }
                    },
                    redis::Value::SimpleString(s) => s.clone(),
                    redis::Value::VerbatimString { format: _, text } => text.clone(),
                    _ => {
                        error!("Unexpected channel format: {:?}", push_info.data[0]);
                        continue;
                    }
                };

                let payload = match &push_info.data[1] {
                    redis::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
                        Ok(s) => s,
                        Err(_) => {
                            error!("Failed to parse payload from bulk string bytes");
                            continue;
                        }
                    },
                    redis::Value::SimpleString(s) => s.clone(),
                    redis::Value::VerbatimString { format: _, text } => text.clone(),
                    _ => {
                        error!("Unexpected payload format: {:?}", push_info.data[1]);
                        continue;
                    }
                };

                // Process the message in a separate task
                let horizontal_clone = horizontal_arc.clone();
                let node_id_clone = node_id.clone();
                let pub_connection_clone = pub_connection.clone();
                let broadcast_channel_clone = broadcast_channel.clone();
                let request_channel_clone = request_channel.clone();
                let response_channel_clone = response_channel.clone();

                tokio::spawn(async move {
                    if channel == broadcast_channel_clone {
                        // Handle broadcast message
                        if let Ok(broadcast) = serde_json::from_str::<BroadcastMessage>(&payload) {
                            if broadcast.node_id == node_id_clone {
                                return;
                            }

                            if let Ok(message) = serde_json::from_str(&broadcast.message) {
                                let except_id = broadcast
                                    .except_socket_id
                                    .as_ref()
                                    .map(|id| SocketId(id.clone()));

                                let mut horizontal_lock = horizontal_clone.lock().await;
                                let _ = horizontal_lock
                                    .local_adapter
                                    .send(
                                        &broadcast.channel,
                                        message,
                                        except_id.as_ref(),
                                        &broadcast.app_id,
                                    )
                                    .await;
                            }
                        }
                    } else if channel == request_channel_clone {
                        // Handle request message
                        if let Ok(request) = serde_json::from_str::<RequestBody>(&payload) {
                            if request.node_id == node_id_clone {
                                return;
                            }

                            let response = {
                                let mut horizontal_lock = horizontal_clone.lock().await;
                                horizontal_lock.process_request(request).await
                            };

                            if let Ok(response) = response {
                                if let Ok(response_json) = serde_json::to_string(&response) {
                                    let mut conn = pub_connection_clone.clone();
                                    let _ = conn
                                        .publish::<_, _, ()>(&response_channel_clone, response_json)
                                        .await;
                                }
                            }
                        }
                    } else if channel == response_channel_clone {
                        // Handle response message
                        if let Ok(response) = serde_json::from_str::<ResponseBody>(&payload) {
                            if response.node_id == node_id_clone {
                                return;
                            }

                            let horizontal_lock = horizontal_clone.lock().await;
                            let _ = horizontal_lock.process_response(response).await;
                        }
                    }
                });
            }
        });

        Ok(())
    }

    pub async fn get_node_count(&self) -> Result<usize> {
        let mut conn = self.connection.clone();
        let result: redis::RedisResult<Vec<redis::Value>> = redis::cmd("PUBSUB")
            .arg("NUMSUB")
            .arg(&self.request_channel)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(values) => {
                if values.len() >= 2 {
                    if let redis::Value::Int(count) = values[1] {
                        Ok((count as usize).max(1))
                    } else {
                        Ok(1)
                    }
                } else {
                    Ok(1)
                }
            }
            Err(e) => {
                error!("Failed to execute PUBSUB NUMSUB: {}", e);
                Ok(1)
            }
        }
    }
}

#[async_trait]
impl ConnectionManager for RedisClusterAdapter {
    async fn init(&mut self) {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.local_adapter.init().await;
        }

        if let Err(e) = self.start_listeners().await {
            error!("Failed to start Redis listeners: {}", e);
        }
    }

    async fn get_namespace(&mut self, app_id: &str) -> Option<Arc<Namespace>> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.local_adapter.get_namespace(app_id).await
    }

    async fn add_socket(
        &mut self,
        socket_id: SocketId,
        socket: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>,
        app_id: &str,
        app_manager: &Arc<dyn AppManager + Send + Sync>,
    ) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .add_socket(socket_id, socket, app_id, app_manager)
            .await
    }

    async fn get_connection(&mut self, socket_id: &SocketId, app_id: &str) -> Option<WebSocketRef> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .get_connection(socket_id, app_id)
            .await
    }

    async fn remove_connection(&mut self, socket_id: &SocketId, app_id: &str) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .remove_connection(socket_id, app_id)
            .await
    }

    async fn send_message(
        &mut self,
        app_id: &str,
        socket_id: &SocketId,
        message: PusherMessage,
    ) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .send_message(app_id, socket_id, message)
            .await
    }

    async fn send(
        &mut self,
        channel: &str,
        message: PusherMessage,
        except: Option<&SocketId>,
        app_id: &str,
    ) -> Result<()> {
        // Send locally first
        let (node_id, local_result) = {
            let mut horizontal_lock = self.horizontal.lock().await;
            let result = horizontal_lock
                .local_adapter
                .send(channel, message.clone(), except, app_id)
                .await;
            (horizontal_lock.node_id.clone(), result)
        };

        if let Err(e) = local_result {
            warn!("Local send failed for channel {}: {}", channel, e);
        }

        // Broadcast to other nodes
        let message_json = serde_json::to_string(&message)?;
        let broadcast = BroadcastMessage {
            node_id,
            app_id: app_id.to_string(),
            channel: channel.to_string(),
            message: message_json,
            except_socket_id: except.map(|id| id.0.clone()),
        };

        let broadcast_json = serde_json::to_string(&broadcast)?;
        let mut conn = self.connection.clone();
        conn.publish::<_, _, ()>(&self.broadcast_channel, broadcast_json)
            .await
            .map_err(|e| Error::Redis(format!("Failed to publish broadcast: {e}")))?;

        Ok(())
    }

    async fn get_channel_members(
        &mut self,
        app_id: &str,
        channel: &str,
    ) -> Result<HashMap<String, PresenceMemberInfo>> {
        // Get local members
        let mut members = {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .get_channel_members(app_id, channel)
                .await?
        };

        // Get distributed members
        let response = self
            .send_request(
                app_id,
                RequestType::ChannelMembers,
                Some(channel),
                None,
                None,
            )
            .await?;

        members.extend(response.members);
        Ok(members)
    }

    async fn get_channel_sockets(
        &mut self,
        app_id: &str,
        channel: &str,
    ) -> Result<DashSet<SocketId>> {
        let all_socket_ids = DashSet::new();

        // Get local sockets
        {
            let mut horizontal = self.horizontal.lock().await;
            let sockets = horizontal
                .local_adapter
                .get_channel_sockets(app_id, channel)
                .await?;

            for entry in sockets.iter() {
                all_socket_ids.insert(entry.key().clone());
            }
        }

        // Get remote sockets
        let response = self
            .send_request(
                app_id,
                RequestType::ChannelSockets,
                Some(channel),
                None,
                None,
            )
            .await?;

        for socket_id in response.socket_ids {
            all_socket_ids.insert(SocketId(socket_id));
        }

        Ok(all_socket_ids)
    }

    async fn remove_channel(&mut self, app_id: &str, channel: &str) {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .remove_channel(app_id, channel)
            .await
    }

    async fn is_in_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        // Check locally first
        let local_result = {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .is_in_channel(app_id, channel, socket_id)
                .await?
        };

        if local_result {
            return Ok(true);
        }

        // Check other nodes
        let response = self
            .send_request(
                app_id,
                RequestType::SocketExistsInChannel,
                Some(channel),
                Some(&socket_id.0),
                None,
            )
            .await?;

        Ok(response.exists)
    }

    async fn get_user_sockets(
        &mut self,
        user_id: &str,
        app_id: &str,
    ) -> Result<DashSet<WebSocketRef>> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .get_user_sockets(user_id, app_id)
            .await
    }

    async fn cleanup_connection(&mut self, app_id: &str, ws: WebSocketRef) {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .cleanup_connection(app_id, ws)
            .await
    }

    async fn terminate_connection(&mut self, app_id: &str, user_id: &str) -> Result<()> {
        // Terminate locally
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .terminate_connection(app_id, user_id)
                .await?;
        }

        // Broadcast termination to other nodes
        let _response = self
            .send_request(
                app_id,
                RequestType::TerminateUserConnections,
                None,
                None,
                Some(user_id),
            )
            .await?;

        Ok(())
    }

    async fn add_channel_to_sockets(&mut self, app_id: &str, channel: &str, socket_id: &SocketId) {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .add_channel_to_sockets(app_id, channel, socket_id)
            .await
    }

    async fn get_channel_socket_count(&mut self, app_id: &str, channel: &str) -> usize {
        // Get local count
        let local_count = {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .get_channel_socket_count(app_id, channel)
                .await
        };

        // Get distributed count
        match self
            .send_request(
                app_id,
                RequestType::ChannelSocketsCount,
                Some(channel),
                None,
                None,
            )
            .await
        {
            Ok(response) => local_count + response.sockets_count,
            Err(e) => {
                error!("Failed to get remote channel socket count: {}", e);
                local_count
            }
        }
    }

    async fn add_to_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .add_to_channel(app_id, channel, socket_id)
            .await
    }

    async fn remove_from_channel(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Result<bool> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .remove_from_channel(app_id, channel, socket_id)
            .await
    }

    async fn get_presence_member(
        &mut self,
        app_id: &str,
        channel: &str,
        socket_id: &SocketId,
    ) -> Option<PresenceMemberInfo> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .get_presence_member(app_id, channel, socket_id)
            .await
    }

    async fn terminate_user_connections(&mut self, app_id: &str, user_id: &str) -> Result<()> {
        self.terminate_connection(app_id, user_id).await
    }

    async fn add_user(&mut self, ws: WebSocketRef) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.local_adapter.add_user(ws).await
    }

    async fn remove_user(&mut self, ws: WebSocketRef) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.local_adapter.remove_user(ws).await
    }

    async fn get_channels_with_socket_count(
        &mut self,
        app_id: &str,
    ) -> Result<DashMap<String, usize>> {
        // Get local channels
        let channels = {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .get_channels_with_socket_count(app_id)
                .await?
        };

        // Get distributed channels
        match self
            .send_request(
                app_id,
                RequestType::ChannelsWithSocketsCount,
                None,
                None,
                None,
            )
            .await
        {
            Ok(response) => {
                for (channel, count) in response.channels_with_sockets_count {
                    *channels.entry(channel).or_insert(0) += count;
                }
            }
            Err(e) => {
                error!("Failed to get remote channels with socket count: {}", e);
            }
        }

        Ok(channels)
    }

    async fn get_sockets_count(&self, app_id: &str) -> Result<usize> {
        // Get local count
        let local_count = {
            let horizontal = self.horizontal.lock().await;
            horizontal.local_adapter.get_sockets_count(app_id).await?
        };

        // Get distributed count
        match self
            .send_request(app_id, RequestType::SocketsCount, None, None, None)
            .await
        {
            Ok(response) => Ok(local_count + response.sockets_count),
            Err(e) => {
                error!("Failed to get remote socket count: {}", e);
                Ok(local_count)
            }
        }
    }

    async fn get_namespaces(&mut self) -> Result<DashMap<String, Arc<Namespace>>> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.local_adapter.get_namespaces().await
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    async fn check_health(&self) -> Result<()> {
        // Use a dedicated connection for health check to avoid impacting main operations
        let mut conn =
            self.client.get_async_connection().await.map_err(|e| {
                Error::Redis(format!("Failed to acquire health check connection: {e}"))
            })?;

        let response = redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| Error::Redis(format!("Cluster health check PING failed: {e}")))?;

        if response == "PONG" {
            Ok(())
        } else {
            Err(Error::Redis(format!(
                "Cluster PING returned unexpected response: {response}"
            )))
        }
    }
}
