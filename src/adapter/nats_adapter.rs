use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapter::ConnectionManager;
use crate::adapter::horizontal_adapter::{
    BroadcastMessage, HorizontalAdapter, PendingRequest, RequestBody, RequestType, ResponseBody,
};
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};
use async_nats::{Client as NatsClient, ConnectOptions as NatsOptions, Subject};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use futures::StreamExt;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::WriteHalf;
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::metrics::MetricsInterface;
use crate::namespace::Namespace;
pub(crate) use crate::options::NatsAdapterConfig;
use crate::protocol::messages::PusherMessage;
use crate::websocket::{SocketId, WebSocketRef};

/// NATS channels/subjects
pub const DEFAULT_PREFIX: &str = "sockudo";
const BROADCAST_SUFFIX: &str = ".broadcast";
const REQUESTS_SUFFIX: &str = ".requests";
const RESPONSES_SUFFIX: &str = ".responses";

/// NATS adapter for horizontal scaling
pub struct NatsAdapter {
    pub horizontal: Arc<Mutex<HorizontalAdapter>>,
    pub client: NatsClient,
    pub prefix: String,
    pub broadcast_subject: String,
    pub request_subject: String,
    pub response_subject: String,
    pub config: NatsAdapterConfig,
}

impl NatsAdapter {
    pub async fn new(config: NatsAdapterConfig) -> Result<Self> {
        let mut horizontal = HorizontalAdapter::new();
        info!(
            "NATS adapter config: servers={:?}, prefix={}, request_timeout={}ms, connection_timeout={}ms",
            config.servers, config.prefix, config.request_timeout_ms, config.connection_timeout_ms
        );
        debug!(
            "NATS adapter credentials: username={:?}, password={:?}, token={:?}",
            config.username, config.password, config.token
        );

        horizontal.requests_timeout = config.request_timeout_ms;

        // Build NATS Options
        let mut nats_options = NatsOptions::new();

        // Set credentials conditionally
        if let (Some(username), Some(password)) =
            (config.username.as_deref(), config.password.as_deref())
        {
            nats_options =
                nats_options.user_and_password(username.to_string(), password.to_string());
        } else if let Some(token) = config.token.as_deref() {
            nats_options = nats_options.token(token.to_string());
        }

        // Set connection timeout
        nats_options =
            nats_options.connection_timeout(Duration::from_millis(config.connection_timeout_ms));

        // Connect to NATS
        let client = nats_options
            .connect(&config.servers)
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to NATS: {e}")))?;

        // Build subject names
        let broadcast_subject = format!("{}{}", config.prefix, BROADCAST_SUFFIX);
        let request_subject = format!("{}{}", config.prefix, REQUESTS_SUFFIX);
        let response_subject = format!("{}{}", config.prefix, RESPONSES_SUFFIX);

        Ok(Self {
            horizontal: Arc::new(Mutex::new(horizontal)),
            client,
            prefix: config.prefix.clone(),
            broadcast_subject,
            request_subject,
            response_subject,
            config,
        })
    }

    pub async fn with_servers(servers: Vec<String>) -> Result<Self> {
        let config = NatsAdapterConfig {
            servers,
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

        // Broadcast the request via NATS
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
                            info!(
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
        let request_data = serde_json::to_vec(request)
            .map_err(|e| Error::Other(format!("Failed to serialize request: {e}")))?;

        self.client
            .publish(
                Subject::from(self.request_subject.clone()),
                request_data.into(),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to publish request: {e}")))?;

        debug!("Broadcasted request {} via NATS", request.request_id);
        Ok(())
    }

    pub async fn start_listeners(&self) -> Result<()> {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.start_request_cleanup();
        }

        self.start_subject_listeners().await?;
        Ok(())
    }

    async fn start_subject_listeners(&self) -> Result<()> {
        let horizontal_arc = self.horizontal.clone();
        let nats_client = self.client.clone();
        let broadcast_subject = self.broadcast_subject.clone();
        let request_subject = self.request_subject.clone();
        let response_subject = self.response_subject.clone();

        let node_id = {
            let horizontal_lock = horizontal_arc.lock().await;
            horizontal_lock.node_id.clone()
        };

        // Subscribe to broadcast channel
        let mut broadcast_subscription = nats_client
            .subscribe(Subject::from(broadcast_subject.clone()))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to subscribe to broadcast subject: {e}"))
            })?;

        // Subscribe to requests channel
        let mut request_subscription = nats_client
            .subscribe(Subject::from(request_subject.clone()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to subscribe to request subject: {e}")))?;

        // Subscribe to responses channel
        let mut response_subscription = nats_client
            .subscribe(Subject::from(response_subject.clone()))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to subscribe to response subject: {e}"))
            })?;

        info!(
            "NATS adapter listening on subjects: {}, {}, {}",
            broadcast_subject, request_subject, response_subject
        );

        // Spawn a task to handle broadcast messages
        let broadcast_horizontal = horizontal_arc.clone();
        let broadcast_node_id = node_id.clone();
        tokio::spawn(async move {
            while let Some(msg) = broadcast_subscription.next().await {
                if let Ok(broadcast) = serde_json::from_slice::<BroadcastMessage>(&msg.payload) {
                    if broadcast.node_id == broadcast_node_id {
                        continue;
                    }

                    if let Ok(message) = serde_json::from_str(&broadcast.message) {
                        let except_id = broadcast
                            .except_socket_id
                            .as_ref()
                            .map(|id| SocketId(id.clone()));

                        let mut horizontal_lock = broadcast_horizontal.lock().await;
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
            }
        });

        // Spawn a task to handle request messages
        let request_horizontal = horizontal_arc.clone();
        let request_node_id = node_id.clone();
        let request_client = nats_client.clone();
        let request_response_subject = response_subject.clone();
        tokio::spawn(async move {
            while let Some(msg) = request_subscription.next().await {
                if let Ok(request) = serde_json::from_slice::<RequestBody>(&msg.payload) {
                    if request.node_id == request_node_id {
                        continue;
                    }

                    let response = {
                        let mut horizontal_lock = request_horizontal.lock().await;
                        horizontal_lock.process_request(request).await
                    };

                    if let Ok(response) = response {
                        if let Ok(response_data) = serde_json::to_vec(&response) {
                            let _ = request_client
                                .publish(
                                    Subject::from(request_response_subject.clone()),
                                    response_data.into(),
                                )
                                .await;
                        }
                    }
                }
            }
        });

        // Spawn a task to handle response messages
        let response_horizontal = horizontal_arc.clone();
        let response_node_id = node_id.clone();
        tokio::spawn(async move {
            while let Some(msg) = response_subscription.next().await {
                if let Ok(response) = serde_json::from_slice::<ResponseBody>(&msg.payload) {
                    if response.node_id == response_node_id {
                        continue;
                    }

                    let horizontal_lock = response_horizontal.lock().await;
                    let _ = horizontal_lock.process_response(response).await;
                }
            }
        });

        Ok(())
    }

    pub async fn get_node_count(&self) -> Result<usize> {
        // If nodes_number is explicitly set, use that value
        if let Some(nodes) = self.config.nodes_number {
            return Ok(nodes as usize);
        }

        // NATS doesn't provide an easy way to count subscriptions like Redis
        // For now, we'll assume at least 1 node (ourselves)
        Ok(1)
    }
}

#[async_trait]
impl ConnectionManager for NatsAdapter {
    async fn init(&mut self) {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.local_adapter.init().await;
        }

        if let Err(e) = self.start_listeners().await {
            error!("Failed to start NATS listeners: {}", e);
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

        let broadcast_data = serde_json::to_vec(&broadcast)?;
        self.client
            .publish(
                Subject::from(self.broadcast_subject.clone()),
                broadcast_data.into(),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to publish broadcast: {e}")))?;

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
        // Check if the NATS client connection is still active
        // NATS client maintains internal health state
        let state = self.client.connection_state();
        match state {
            async_nats::connection::State::Connected => Ok(()),
            async_nats::connection::State::Disconnected => Err(crate::error::Error::Connection(
                "NATS client is disconnected".to_string(),
            )),
            other_state => Err(crate::error::Error::Connection(format!(
                "NATS client in transitional state: {other_state:?}"
            ))),
        }
    }
}
