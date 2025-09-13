use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapter::connection_manager::{ConnectionManager, HorizontalAdapterInterface};
use crate::adapter::horizontal_adapter::{
    BroadcastMessage, DeadNodeEvent, HorizontalAdapter, OrphanedMember, PendingRequest,
    RequestBody, RequestType, ResponseBody, current_timestamp, generate_request_id,
};
use crate::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use crate::app::manager::AppManager;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};
use crate::metrics::MetricsInterface;
use crate::namespace::Namespace;
use crate::options::ClusterHealthConfig;
use crate::protocol::messages::PusherMessage;
use crate::websocket::{SocketId, WebSocketRef};
use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::io::WriteHalf;
use tokio::sync::{Mutex, Notify};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Generic base adapter that handles all common horizontal scaling logic
pub struct HorizontalAdapterBase<T: HorizontalTransport> {
    pub horizontal: Arc<Mutex<HorizontalAdapter>>,
    pub transport: T,
    pub config: T::Config,
    pub event_bus: Option<tokio::sync::mpsc::UnboundedSender<DeadNodeEvent>>,
    pub node_id: String,
    pub cluster_health_enabled: bool,
    pub heartbeat_interval_ms: u64,
    pub node_timeout_ms: u64,
    pub cleanup_interval_ms: u64,
}

impl<T: HorizontalTransport + 'static> HorizontalAdapterBase<T>
where
    T::Config: TransportConfig,
{
    pub async fn new(config: T::Config) -> Result<Self> {
        let mut horizontal = HorizontalAdapter::new();
        horizontal.requests_timeout = config.request_timeout_ms();
        let node_id = horizontal.node_id.clone();

        let transport = T::new(config.clone()).await?;

        let cluster_health_defaults = ClusterHealthConfig::default();

        Ok(Self {
            horizontal: Arc::new(Mutex::new(horizontal)),
            transport,
            config,
            event_bus: None,
            node_id,
            cluster_health_enabled: cluster_health_defaults.enabled,
            heartbeat_interval_ms: cluster_health_defaults.heartbeat_interval_ms,
            node_timeout_ms: cluster_health_defaults.node_timeout_ms,
            cleanup_interval_ms: cluster_health_defaults.cleanup_interval_ms,
        })
    }

    pub async fn set_metrics(
        &mut self,
        metrics: Arc<Mutex<dyn MetricsInterface + Send + Sync>>,
    ) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal.metrics = Some(metrics);
        Ok(())
    }

    pub fn set_event_bus(
        &mut self,
        event_sender: tokio::sync::mpsc::UnboundedSender<DeadNodeEvent>,
    ) {
        self.event_bus = Some(event_sender);
    }

    pub async fn set_cluster_health(&mut self, cluster_health: &ClusterHealthConfig) -> Result<()> {
        // Validate cluster health configuration first
        if let Err(validation_error) = cluster_health.validate() {
            warn!(
                "Cluster health configuration validation failed: {}",
                validation_error
            );
            warn!("Keeping current cluster health settings");
            return Ok(());
        }

        // Set cluster health configuration directly on the base adapter
        self.heartbeat_interval_ms = cluster_health.heartbeat_interval_ms;
        self.node_timeout_ms = cluster_health.node_timeout_ms;
        self.cleanup_interval_ms = cluster_health.cleanup_interval_ms;
        self.cluster_health_enabled = cluster_health.enabled;

        // Log warning for high-frequency heartbeats
        if cluster_health.heartbeat_interval_ms < 1000 {
            warn!(
                "High-frequency heartbeat interval ({} ms) may cause unnecessary network load. Consider using >= 1000 ms unless high availability is critical.",
                cluster_health.heartbeat_interval_ms
            );
        }

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
        let node_count = self.transport.get_node_count().await?;

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
            // Cluster presence fields (not used for regular requests)
            user_info: None,
            timestamp: None,
            dead_node_id: None,
            target_node_id: None,
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

        // Broadcast the request via transport
        self.transport.publish_request(&request).await?;

        // Wait for responses
        let timeout_duration = Duration::from_millis(self.config.request_timeout_ms());
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

        // Aggregate responses first, then clean up to prevent race condition
        let combined_response = {
            let horizontal = self.horizontal.lock().await;
            horizontal.aggregate_responses(
                request_id.clone(),
                request.node_id,
                app_id.to_string(),
                &request_type,
                responses,
            )
        }; // horizontal lock released here

        // Clean up the pending request after aggregation is complete
        {
            let horizontal = self.horizontal.lock().await;
            horizontal.pending_requests.remove(&request_id);
        }

        // Track metrics
        {
            let horizontal = self.horizontal.lock().await;
            if let Some(metrics_ref) = &horizontal.metrics {
                let metrics = metrics_ref.lock().await;
                let duration_ms = start.elapsed().as_micros() as f64 / 1000.0; // Convert to milliseconds with 3 decimal places
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

    pub async fn start_listeners(&self) -> Result<()> {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.start_request_cleanup();
        }

        // Start cluster health system only if enabled
        if self.cluster_health_enabled {
            self.start_cluster_health_system().await;
        }

        // Set up transport handlers
        let horizontal_arc = self.horizontal.clone();

        let broadcast_horizontal = horizontal_arc.clone();
        let request_horizontal = horizontal_arc.clone();
        let response_horizontal = horizontal_arc.clone();
        let transport_for_request = self.transport.clone();

        let handlers = TransportHandlers {
            on_broadcast: Arc::new(move |broadcast| {
                let horizontal_clone = broadcast_horizontal.clone();
                Box::pin(async move {
                    let node_id = {
                        let horizontal = horizontal_clone.lock().await;
                        horizontal.node_id.clone()
                    };

                    if broadcast.node_id == node_id {
                        return;
                    }

                    if let Ok(message) = serde_json::from_str(&broadcast.message) {
                        let except_id = broadcast
                            .except_socket_id
                            .as_ref()
                            .map(|id| SocketId(id.clone()));

                        // Send the message first and count local recipients
                        let mut horizontal_lock = horizontal_clone.lock().await;

                        // Count local recipients for this node (adjusts for excluded socket)
                        let local_recipient_count = horizontal_lock
                            .get_local_recipient_count(
                                &broadcast.app_id,
                                &broadcast.channel,
                                except_id.as_ref(),
                            )
                            .await;

                        // Use the timestamp from the broadcast message for end-to-end tracking
                        let send_result = horizontal_lock
                            .local_adapter
                            .send(
                                &broadcast.channel,
                                message,
                                except_id.as_ref(),
                                &broadcast.app_id,
                                broadcast.timestamp_ms, // Pass through the original timestamp
                            )
                            .await;

                        // Track broadcast latency metrics using helper function
                        let metrics_ref = horizontal_lock.metrics.clone();
                        drop(horizontal_lock); // Release horizontal lock to avoid deadlock

                        HorizontalAdapter::track_broadcast_latency_if_successful(
                            &send_result,
                            broadcast.timestamp_ms,
                            Some(local_recipient_count), // Use local count, not sender's count
                            &broadcast.app_id,
                            &broadcast.channel,
                            metrics_ref,
                        )
                        .await;
                    }
                })
            }),
            on_request: Arc::new(move |request| {
                let horizontal_clone = request_horizontal.clone();
                let transport_clone = transport_for_request.clone();
                Box::pin(async move {
                    let node_id = {
                        let horizontal = horizontal_clone.lock().await;
                        horizontal.node_id.clone()
                    };

                    if request.node_id == node_id {
                        return Err(Error::OwnRequestIgnored);
                    }

                    // Check if this is a targeted request for another node
                    if let Some(ref target_node) = request.target_node_id
                        && target_node != &node_id
                    {
                        // Not for us, ignore silently
                        return Err(Error::RequestNotForThisNode);
                    }

                    let mut horizontal_lock = horizontal_clone.lock().await;
                    let response = horizontal_lock.process_request(request.clone()).await?;

                    // Check if this was a heartbeat that detected a new node
                    if request.request_type == RequestType::Heartbeat && response.exists {
                        // New node detected, send our presence state to it
                        // Spawn task to avoid blocking request processing
                        let new_node_id = request.node_id.clone();
                        let horizontal_clone_for_task = horizontal_clone.clone();
                        let transport_for_task = transport_clone.clone();

                        tokio::spawn(async move {
                            // Small delay to let the new node finish initialization
                            tokio::time::sleep(Duration::from_millis(100)).await;

                            if let Err(e) = send_presence_state_to_node(
                                &horizontal_clone_for_task,
                                &transport_for_task,
                                &new_node_id,
                            )
                            .await
                            {
                                error!(
                                    "Failed to send presence state to new node {}: {}",
                                    new_node_id, e
                                );
                            }
                        });
                    }

                    Ok(response)
                })
            }),
            on_response: Arc::new(move |response| {
                let horizontal_clone = response_horizontal.clone();
                Box::pin(async move {
                    let node_id = {
                        let horizontal = horizontal_clone.lock().await;
                        horizontal.node_id.clone()
                    };

                    if response.node_id == node_id {
                        return;
                    }

                    let horizontal_lock = horizontal_clone.lock().await;
                    let _ = horizontal_lock.process_response(response).await;
                })
            }),
        };

        self.transport.start_listeners(handlers).await?;
        Ok(())
    }

    /// Start cluster health monitoring system
    pub async fn start_cluster_health_system(&self) {
        let heartbeat_interval_ms = self.heartbeat_interval_ms;
        let node_timeout_ms = self.node_timeout_ms;

        info!(
            "Starting cluster health system with heartbeat interval: {}ms, timeout: {}ms",
            heartbeat_interval_ms, node_timeout_ms
        );

        // Start heartbeat broadcasting
        self.start_heartbeat_loop().await;

        // Start dead node detection
        self.start_dead_node_detection().await;
    }

    /// Start heartbeat broadcasting loop
    async fn start_heartbeat_loop(&self) {
        let transport = self.transport.clone();
        let node_id = self.node_id.clone();
        let heartbeat_interval_ms = self.heartbeat_interval_ms;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));

            loop {
                interval.tick().await;

                let heartbeat_request = RequestBody {
                    request_id: generate_request_id(),
                    node_id: node_id.clone(),
                    app_id: "cluster".to_string(),
                    request_type: RequestType::Heartbeat,
                    channel: None,
                    socket_id: None,
                    user_id: None,
                    user_info: None,
                    timestamp: Some(current_timestamp()),
                    dead_node_id: None,
                    target_node_id: None,
                };

                if let Err(e) = transport.publish_request(&heartbeat_request).await {
                    error!("Failed to send heartbeat: {}", e);
                }
            }
        });
    }

    /// Start dead node detection loop
    async fn start_dead_node_detection(&self) {
        let transport = self.transport.clone();
        let horizontal = self.horizontal.clone();
        let event_bus = self.event_bus.clone();
        let node_id = self.node_id.clone();
        let cleanup_interval_ms = self.cleanup_interval_ms;
        let node_timeout_ms = self.node_timeout_ms;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(cleanup_interval_ms));

            loop {
                interval.tick().await;

                let dead_nodes = {
                    let horizontal_guard = horizontal.lock().await;
                    horizontal_guard.get_dead_nodes(node_timeout_ms).await
                };

                if !dead_nodes.is_empty() {
                    // Single leader election for entire cleanup round
                    let is_leader = {
                        let horizontal_guard = horizontal.lock().await;
                        horizontal_guard.is_cleanup_leader(&dead_nodes).await
                    };

                    if is_leader {
                        info!(
                            "Elected as cleanup leader for {} dead nodes: {:?}",
                            dead_nodes.len(),
                            dead_nodes
                        );

                        // Process ALL dead nodes as the elected leader
                        for dead_node_id in dead_nodes {
                            debug!("Processing dead node: {}", dead_node_id);

                            // 1. Remove from local heartbeat tracking
                            {
                                let horizontal_guard = horizontal.lock().await;
                                horizontal_guard.remove_dead_node(&dead_node_id).await;
                            }

                            // 2. Get orphaned members and clean up local registry
                            let cleanup_tasks = {
                                let horizontal_guard = horizontal.lock().await;
                                match horizontal_guard
                                    .handle_dead_node_cleanup(&dead_node_id)
                                    .await
                                {
                                    Ok(tasks) => tasks,
                                    Err(e) => {
                                        error!(
                                            "Failed to cleanup dead node {}: {}",
                                            dead_node_id, e
                                        );
                                        continue;
                                    }
                                }
                            };

                            if !cleanup_tasks.is_empty() {
                                // 3. Convert to orphaned members and emit event
                                let orphaned_members: Vec<OrphanedMember> = cleanup_tasks
                                    .into_iter()
                                    .map(|(app_id, channel, user_id, user_info)| OrphanedMember {
                                        app_id,
                                        channel,
                                        user_id,
                                        user_info, // Preserve user_info for proper presence management
                                    })
                                    .collect();

                                let event = DeadNodeEvent {
                                    dead_node_id: dead_node_id.clone(),
                                    orphaned_members: orphaned_members.clone(),
                                };

                                // Emit event for processing by ConnectionHandler
                                if let Some(event_sender) = &event_bus {
                                    if let Err(e) = event_sender.send(event) {
                                        error!("Failed to send dead node event: {}", e);
                                    }
                                } else {
                                    warn!("No event bus configured for dead node cleanup");
                                }

                                info!(
                                    "Emitted cleanup event for {} orphaned presence members from dead node {}",
                                    orphaned_members.len(),
                                    dead_node_id
                                );
                            }

                            // 4. Notify followers to clean their registries
                            let dead_node_request = RequestBody {
                                request_id: generate_request_id(),
                                node_id: node_id.clone(),
                                app_id: "cluster".to_string(),
                                request_type: RequestType::NodeDead,
                                channel: None,
                                socket_id: None,
                                user_id: None,
                                user_info: None,
                                timestamp: Some(current_timestamp()),
                                dead_node_id: Some(dead_node_id.clone()),
                                target_node_id: None,
                            };

                            if let Err(e) = transport.publish_request(&dead_node_request).await {
                                error!("Failed to send dead node notification: {}", e);
                            }
                        }
                    } else {
                        debug!(
                            "Not cleanup leader for {} dead nodes, waiting for leader's broadcasts",
                            dead_nodes.len()
                        );
                    }
                }
            }
        });
    }

    /// Get a snapshot of the cluster presence registry
    pub async fn get_cluster_presence_registry(
        &self,
    ) -> crate::adapter::horizontal_adapter::ClusterPresenceRegistry {
        let horizontal = self.horizontal.lock().await;
        let registry = horizontal.cluster_presence_registry.read().await;
        registry.clone()
    }

    /// Get a snapshot of node heartbeat tracking
    pub async fn get_node_heartbeats(
        &self,
    ) -> std::collections::HashMap<String, std::time::Instant> {
        let horizontal = self.horizontal.lock().await;
        let heartbeats = horizontal.node_heartbeats.read().await;
        heartbeats.clone()
    }
}

#[async_trait]
impl<T: HorizontalTransport + 'static> ConnectionManager for HorizontalAdapterBase<T>
where
    T::Config: TransportConfig,
{
    async fn init(&mut self) {
        {
            let mut horizontal = self.horizontal.lock().await;
            horizontal.local_adapter.init().await;
        }

        if let Err(e) = self.start_listeners().await {
            error!("Failed to start transport listeners: {}", e);
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
        start_time_ms: Option<f64>,
    ) -> Result<()> {
        // BroadcastMessage is already imported at the top

        // Send locally first (tracked in connection manager for metrics)
        let (node_id, local_result) = {
            let mut horizontal_lock = self.horizontal.lock().await;

            let result = horizontal_lock
                .local_adapter
                .send(channel, message.clone(), except, app_id, start_time_ms)
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
            timestamp_ms: start_time_ms.or_else(|| {
                Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as f64
                        / 1_000_000.0, // Convert to milliseconds with microsecond precision
                )
            }),
        };

        self.transport.publish_broadcast(&broadcast).await?;

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

    async fn remove_user_socket(
        &mut self,
        user_id: &str,
        socket_id: &SocketId,
        app_id: &str,
    ) -> Result<()> {
        let mut horizontal = self.horizontal.lock().await;
        horizontal
            .local_adapter
            .remove_user_socket(user_id, socket_id, app_id)
            .await
    }

    async fn count_user_connections_in_channel(
        &mut self,
        user_id: &str,
        app_id: &str,
        channel: &str,
        excluding_socket: Option<&SocketId>,
    ) -> Result<usize> {
        // Get local count (with excluding_socket filter)
        let local_count = {
            let mut horizontal = self.horizontal.lock().await;
            horizontal
                .local_adapter
                .count_user_connections_in_channel(user_id, app_id, channel, excluding_socket)
                .await?
        };

        // Get remote count (no excluding_socket since it's local-only)
        match self
            .send_request(
                app_id,
                RequestType::CountUserConnectionsInChannel,
                Some(channel),
                None,
                Some(user_id),
            )
            .await
        {
            Ok(response) => Ok(local_count + response.sockets_count),
            Err(e) => {
                error!("Failed to get remote user connections count: {}", e);
                Ok(local_count)
            }
        }
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
        self.transport.check_health().await
    }

    fn get_node_id(&self) -> String {
        self.node_id.clone()
    }

    fn as_horizontal_adapter(&self) -> Option<&dyn HorizontalAdapterInterface> {
        Some(self)
    }

    fn configure_dead_node_events(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<DeadNodeEvent>> {
        let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();
        self.set_event_bus(event_sender);
        Some(event_receiver)
    }
}

#[async_trait]
impl<T: HorizontalTransport> HorizontalAdapterInterface for HorizontalAdapterBase<T> {
    /// Broadcast presence member joined to all nodes
    async fn broadcast_presence_join(
        &self,
        app_id: &str,
        channel: &str,
        user_id: &str,
        socket_id: &str,
        user_info: Option<serde_json::Value>,
    ) -> Result<()> {
        // Store in our own registry first with a single lock acquisition
        {
            let horizontal = self.horizontal.lock().await;
            horizontal
                .add_presence_entry(
                    &self.node_id,
                    channel,
                    socket_id,
                    user_id,
                    app_id,
                    user_info.clone(),
                )
                .await;
        }

        // Skip cluster broadcast if cluster health is disabled
        if !self.cluster_health_enabled {
            return Ok(());
        }

        let request = RequestBody {
            request_id: crate::adapter::horizontal_adapter::generate_request_id(),
            node_id: self.node_id.clone(),
            app_id: app_id.to_string(),
            request_type: RequestType::PresenceMemberJoined,
            channel: Some(channel.to_string()),
            socket_id: Some(socket_id.to_string()),
            user_id: Some(user_id.to_string()),
            // Cluster presence fields
            user_info,
            timestamp: None,
            dead_node_id: None,
            target_node_id: None,
        };

        // Send without waiting for response (broadcast)
        self.transport.publish_request(&request).await
    }

    /// Broadcast presence member left to all nodes
    async fn broadcast_presence_leave(
        &self,
        app_id: &str,
        channel: &str,
        user_id: &str,
        socket_id: &str,
    ) -> Result<()> {
        // Remove from our own registry first with a single lock acquisition
        {
            let horizontal = self.horizontal.lock().await;
            horizontal
                .remove_presence_entry(&self.node_id, channel, socket_id)
                .await;
        }

        // Skip cluster broadcast if cluster health is disabled
        if !self.cluster_health_enabled {
            return Ok(());
        }

        let request = RequestBody {
            request_id: crate::adapter::horizontal_adapter::generate_request_id(),
            node_id: self.node_id.clone(),
            app_id: app_id.to_string(),
            request_type: RequestType::PresenceMemberLeft,
            channel: Some(channel.to_string()),
            socket_id: Some(socket_id.to_string()),
            user_id: Some(user_id.to_string()),
            // Cluster presence fields
            user_info: None,
            timestamp: None,
            dead_node_id: None,
            target_node_id: None,
        };

        // Send without waiting for response (broadcast)
        self.transport.publish_request(&request).await
    }
}

/// Helper function to send presence state to a new node
async fn send_presence_state_to_node<T: HorizontalTransport>(
    horizontal: &Arc<tokio::sync::Mutex<HorizontalAdapter>>,
    transport: &T,
    target_node_id: &str,
) -> Result<()> {
    // Get our presence data
    let (our_node_id, data_to_send) = {
        let horizontal_lock = horizontal.lock().await;
        let registry = horizontal_lock.cluster_presence_registry.read().await;

        // Get only our node's data
        if let Some(our_presence_data) = registry.get(&horizontal_lock.node_id) {
            // Clone the data to avoid holding the lock
            (
                horizontal_lock.node_id.clone(),
                Some(our_presence_data.clone()),
            )
        } else {
            (horizontal_lock.node_id.clone(), None)
        }
    };

    if let Some(data_to_send) = data_to_send {
        // Serialize the presence data
        let serialized_data = serde_json::to_value(&data_to_send)?;

        let sync_request = RequestBody {
            request_id: generate_request_id(),
            node_id: our_node_id,
            app_id: "cluster".to_string(),
            request_type: RequestType::PresenceStateSync,
            target_node_id: Some(target_node_id.to_string()),
            user_info: Some(serialized_data), // Reuse this field for bulk data
            channel: None,
            socket_id: None,
            user_id: None,
            timestamp: None,
            dead_node_id: None,
        };

        // This broadcasts but only target_node will process it
        transport.publish_request(&sync_request).await?;

        info!(
            "Sent presence state to new node: {} ({} channels)",
            target_node_id,
            data_to_send.len()
        );
    } else {
        debug!("No presence data to send to new node: {}", target_node_id);
    }

    Ok(())
}
