#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapter::ConnectionManager;
use crate::adapter::local_adapter::LocalAdapter;
use crate::channel::PresenceMemberInfo;
use crate::error::{Error, Result};

use crate::metrics::MetricsInterface;
use crate::websocket::SocketId;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

/// Request types for horizontal communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestType {
    // Original request types
    ChannelMembers,           // Get members in a channel
    ChannelSockets,           // Get sockets in a channel
    ChannelSocketsCount,      // Get count of sockets in a channel
    SocketExistsInChannel,    // Check if socket exists in a channel
    TerminateUserConnections, // Terminate user connections
    ChannelsWithSocketsCount, // Get channels with socket counts

    // New request types
    Sockets,             // Get all sockets
    Channels,            // Get all channels
    SocketsCount,        // Get count of all sockets
    ChannelMembersCount, // Get count of members in a channel
}

/// Request body for horizontal communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    pub request_id: String,
    pub node_id: String,
    pub app_id: String,
    pub request_type: RequestType,
    pub channel: Option<String>,
    pub socket_id: Option<String>,
    pub user_id: Option<String>,
}

/// Response body for horizontal requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseBody {
    pub request_id: String,
    pub node_id: String,
    pub app_id: String,
    pub members: HashMap<String, PresenceMemberInfo>,
    pub channels_with_sockets_count: HashMap<String, usize>,
    pub socket_ids: Vec<String>,
    pub sockets_count: usize,
    pub exists: bool,
    pub channels: HashSet<String>,
    pub members_count: usize, // New field for ChannelMembersCount
}

/// Message for broadcasting events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastMessage {
    pub node_id: String,
    pub app_id: String,
    pub channel: String,
    pub message: String,
    pub except_socket_id: Option<String>,
}

/// Request tracking struct
#[derive(Clone)]
pub struct PendingRequest {
    pub(crate) start_time: Instant,
    pub(crate) app_id: String,
    pub(crate) responses: Vec<ResponseBody>,
    pub(crate) notify: Arc<Notify>,
}

/// Base horizontal adapter
pub struct HorizontalAdapter {
    /// Unique node ID
    pub node_id: String,

    /// Local adapter for handling local connections
    pub local_adapter: LocalAdapter,

    /// Pending requests map - Use DashMap for thread-safe access
    pub pending_requests: DashMap<String, PendingRequest>,

    /// Timeout for requests in milliseconds
    pub requests_timeout: u64,

    pub metrics: Option<Arc<Mutex<dyn MetricsInterface + Send + Sync>>>,
}

impl Default for HorizontalAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl HorizontalAdapter {
    /// Create a new horizontal adapter
    pub fn new() -> Self {
        Self {
            node_id: Uuid::new_v4().to_string(),
            local_adapter: LocalAdapter::new(),
            pending_requests: DashMap::new(),
            requests_timeout: 5000, // Default 5 seconds
            metrics: None,
        }
    }

    /// Start the request cleanup task
    pub fn start_request_cleanup(&mut self) {
        // Clone data needed for the task
        // let node_id = self.node_id.clone();
        let timeout = self.requests_timeout;
        let pending_requests_clone = self.pending_requests.clone();

        // Spawn a background task to clean up stale requests
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_millis(1000)).await;

                // Find and process expired requests
                let now = Instant::now();
                let mut expired_requests = Vec::new();

                // We can't modify pending_requests while iterating
                for entry in &pending_requests_clone {
                    let request_id = entry.key();
                    let request = entry.value();
                    if now.duration_since(request.start_time).as_millis() > timeout as u128 {
                        expired_requests.push(request_id.clone());
                    }
                }

                // Process expired requests
                for request_id in expired_requests {
                    warn!("{}", format!("Request {} expired", request_id));
                    pending_requests_clone.remove(&request_id);
                }
            }
        });
    }

    /// Process a received request from another node
    pub async fn process_request(&mut self, request: RequestBody) -> Result<ResponseBody> {
        info!(
            "{}",
            format!(
                "Processing request from node {}: {:?}",
                request.node_id, request.request_type
            )
        );

        // Skip processing our own requests
        if request.node_id == self.node_id {
            return Err(Error::OwnRequestIgnored);
        }

        // Track metrics for received request
        if let Some(ref metrics) = self.metrics {
            let metrics = metrics.lock().await;
            metrics.mark_horizontal_adapter_request_received(&request.app_id);
        }

        // Initialize empty response
        let mut response = ResponseBody {
            request_id: request.request_id.clone(),
            node_id: self.node_id.clone(),
            app_id: request.app_id.clone(),
            members: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            channels_with_sockets_count: HashMap::new(),
            exists: false,
            channels: HashSet::new(),
            members_count: 0,
        };

        // Process based on request type
        match request.request_type {
            RequestType::ChannelMembers => {
                if let Some(channel) = &request.channel {
                    // Get channel members from local adapter
                    let members = self
                        .local_adapter
                        .get_channel_members(&request.app_id, channel)
                        .await?;
                    response.members = members;
                }
            }
            RequestType::ChannelSockets => {
                if let Some(channel) = &request.channel {
                    // Get channel sockets from local adapter
                    let channel_set = self
                        .local_adapter
                        .get_channel_sockets(&request.app_id, channel)
                        .await?;
                    response.socket_ids = channel_set
                        .iter()
                        .map(|socket_id| socket_id.0.clone())
                        .collect();
                }
            }
            RequestType::ChannelSocketsCount => {
                if let Some(channel) = &request.channel {
                    // Get channel socket count from local adapter
                    response.sockets_count = self
                        .local_adapter
                        .get_channel_socket_count(&request.app_id, channel)
                        .await;
                }
            }
            RequestType::SocketExistsInChannel => {
                if let (Some(channel), Some(socket_id)) = (&request.channel, &request.socket_id) {
                    // Check if socket exists in channel
                    let socket_id = SocketId(socket_id.clone());
                    response.exists = self
                        .local_adapter
                        .is_in_channel(&request.app_id, channel, &socket_id)
                        .await?;
                }
            }
            RequestType::TerminateUserConnections => {
                if let Some(user_id) = &request.user_id {
                    // Terminate user connections locally
                    self.local_adapter
                        .terminate_user_connections(&request.app_id, user_id)
                        .await?;
                    response.exists = true;
                }
            }
            RequestType::ChannelsWithSocketsCount => {
                // Get channels with socket count from local adapter
                let channels = self
                    .local_adapter
                    .get_channels_with_socket_count(&request.app_id)
                    .await?;
                response.channels_with_sockets_count = channels
                    .iter()
                    .map(|entry| (entry.key().clone(), *entry.value()))
                    .collect();
            }
            // New request types
            RequestType::Sockets => {
                // Get all connections for the app
                let connections = self
                    .local_adapter
                    .get_all_connections(&request.app_id)
                    .await;
                response.socket_ids = connections
                    .iter()
                    .map(|entry| entry.key().0.clone())
                    .collect();
                response.sockets_count = connections.len();
            }
            RequestType::Channels => {
                // Get all channels for the app
                let channels = self
                    .local_adapter
                    .get_channels_with_socket_count(&request.app_id)
                    .await?;
                response.channels = channels.iter().map(|entry| entry.key().clone()).collect();
            }
            RequestType::SocketsCount => {
                // Get count of all sockets
                let connections = self
                    .local_adapter
                    .get_all_connections(&request.app_id)
                    .await;
                response.sockets_count = connections.len();
            }
            RequestType::ChannelMembersCount => {
                if let Some(channel) = &request.channel {
                    // Get count of members in a channel
                    let members = self
                        .local_adapter
                        .get_channel_members(&request.app_id, channel)
                        .await?;
                    response.members_count = members.len();
                }
            }
        }

        // Return the response
        Ok(response)
    }

    /// Process a response received from another node
    pub async fn process_response(&self, response: ResponseBody) -> Result<()> {
        // Track received response
        if let Some(metrics_ref) = &self.metrics {
            let metrics = metrics_ref.lock().await;
            metrics.mark_horizontal_adapter_response_received(&response.app_id);
        }

        // Get the pending request and notify waiters
        if let Some(mut request) = self.pending_requests.get_mut(&response.request_id) {
            // Add response to the list
            request.responses.push(response);

            // Notify any waiting send_request calls that a new response has arrived
            request.notify.notify_one();
        }

        Ok(())
    }

    /// Send a request to other nodes and wait for responses
    pub async fn send_request(
        &mut self,
        app_id: &str,
        request_type: RequestType,
        channel: Option<&str>,
        socket_id: Option<&str>,
        user_id: Option<&str>,
        expected_node_count: usize,
    ) -> Result<ResponseBody> {
        let request_id = Uuid::new_v4().to_string();
        let start = Instant::now();

        // Create the request
        let _request = RequestBody {
            request_id: request_id.clone(),
            node_id: self.node_id.clone(),
            app_id: app_id.to_string(),
            request_type: request_type.clone(),
            channel: channel.map(String::from),
            socket_id: socket_id.map(String::from),
            user_id: user_id.map(String::from),
        };

        // Add to pending requests with proper initialization
        self.pending_requests.insert(
            request_id.clone(),
            PendingRequest {
                start_time: start,
                app_id: app_id.to_string(),
                responses: Vec::with_capacity(expected_node_count.saturating_sub(1)),
                notify: Arc::new(Notify::new()),
            },
        );

        // Track sent request in metrics
        if let Some(metrics_ref) = &self.metrics {
            let metrics = metrics_ref.lock().await;
            metrics.mark_horizontal_adapter_request_sent(app_id);
        }

        // ✅ IMPORTANT: Broadcasting is handled by the adapter implementation
        // For Redis: RedisAdapter publishes to request_channel in its listeners
        // For NATS: NatsAdapter would publish to NATS subjects
        // For HTTP: HttpAdapter would POST to other nodes
        info!(
            "Request {} created for type {:?} on app {} - broadcasting handled by adapter",
            request_id, request_type, app_id
        );

        // Wait for responses with proper timeout handling
        let timeout_duration = Duration::from_millis(self.requests_timeout);
        let max_expected_responses = expected_node_count.saturating_sub(1);

        // If we don't expect any responses (single node), return immediately
        if max_expected_responses == 0 {
            info!(
                "Single node deployment, no responses expected for request {}",
                request_id
            );
            self.pending_requests.remove(&request_id);
            return Ok(ResponseBody {
                request_id: request_id.clone(),
                node_id: self.node_id.clone(),
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

        // Improved waiting logic
        let check_interval = Duration::from_millis(50);
        let mut checks = 0;
        let max_checks = (timeout_duration.as_millis() / check_interval.as_millis()) as usize;

        let responses = loop {
            if checks >= max_checks {
                let current_responses = self
                    .pending_requests
                    .get(&request_id)
                    .map(|r| r.responses.len())
                    .unwrap_or(0);

                warn!(
                    "Request {} timed out after {}ms, got {} responses out of {} expected",
                    request_id,
                    start.elapsed().as_millis(),
                    current_responses,
                    max_expected_responses
                );
                break self
                    .pending_requests
                    .remove(&request_id)
                    .map(|(_, req)| req.responses)
                    .unwrap_or_default();
            }

            if let Some(pending_request) = self.pending_requests.get(&request_id) {
                if pending_request.responses.len() >= max_expected_responses {
                    info!(
                        "Request {} completed successfully with {}/{} responses in {}ms",
                        request_id,
                        pending_request.responses.len(),
                        max_expected_responses,
                        start.elapsed().as_millis()
                    );
                    break self
                        .pending_requests
                        .remove(&request_id)
                        .map(|(_, req)| req.responses)
                        .unwrap_or_default();
                }
            } else {
                return Err(Error::Other(format!(
                    "Request {request_id} was removed unexpectedly (possibly by cleanup task)"
                )));
            }

            tokio::time::sleep(check_interval).await;
            checks += 1;
        };

        // Use the aggregation method
        let combined_response = self.aggregate_responses(
            request_id.clone(),
            self.node_id.clone(),
            app_id.to_string(),
            &request_type,
            responses,
        );

        // Validate the aggregated response
        if let Err(e) = self.validate_aggregated_response(&combined_response, &request_type) {
            warn!(
                "Response validation failed for request {}: {}",
                request_id, e
            );
        }

        // Track metrics
        if let Some(metrics_ref) = &self.metrics {
            let metrics = metrics_ref.lock().await;
            let duration_ms = start.elapsed().as_millis() as f64;

            metrics.track_horizontal_adapter_resolve_time(app_id, duration_ms);

            let resolved = combined_response.sockets_count > 0
                || !combined_response.members.is_empty()
                || combined_response.exists
                || !combined_response.channels.is_empty()
                || combined_response.members_count > 0
                || !combined_response.channels_with_sockets_count.is_empty()
                || max_expected_responses == 0;

            metrics.track_horizontal_adapter_resolved_promises(app_id, resolved);
        }

        Ok(combined_response)
    }

    pub fn aggregate_responses(
        &self,
        request_id: String,
        node_id: String,
        app_id: String,
        request_type: &RequestType,
        responses: Vec<ResponseBody>,
    ) -> ResponseBody {
        let mut combined_response = ResponseBody {
            request_id,
            node_id,
            app_id,
            members: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            channels_with_sockets_count: HashMap::new(),
            exists: false,
            channels: HashSet::new(),
            members_count: 0,
        };

        if responses.is_empty() {
            return combined_response;
        }

        // Track unique socket IDs to avoid duplicates when aggregating
        let mut unique_socket_ids = HashSet::new();

        for response in responses {
            match request_type {
                RequestType::ChannelMembers => {
                    // Merge members - later responses can overwrite earlier ones with same user_id
                    // This handles the case where a user might be connected to multiple nodes
                    combined_response.members.extend(response.members);
                }

                RequestType::ChannelSockets => {
                    // Collect unique socket IDs across all nodes
                    for socket_id in response.socket_ids {
                        unique_socket_ids.insert(socket_id);
                    }
                }

                RequestType::ChannelSocketsCount => {
                    // Sum socket counts from all nodes
                    combined_response.sockets_count += response.sockets_count;
                }

                RequestType::SocketExistsInChannel => {
                    // If any node reports the socket exists, it exists
                    combined_response.exists = combined_response.exists || response.exists;
                }

                RequestType::TerminateUserConnections => {
                    // If any node successfully terminated connections, mark as success
                    combined_response.exists = combined_response.exists || response.exists;
                }

                RequestType::ChannelsWithSocketsCount => {
                    // FIXED: Actually add the socket counts instead of just inserting 0
                    for (channel, socket_count) in response.channels_with_sockets_count {
                        *combined_response
                            .channels_with_sockets_count
                            .entry(channel)
                            .or_insert(0) += socket_count;
                    }
                }

                RequestType::Sockets => {
                    // Collect unique socket IDs and sum total count
                    for socket_id in response.socket_ids {
                        unique_socket_ids.insert(socket_id);
                    }
                    combined_response.sockets_count += response.sockets_count;
                }

                RequestType::Channels => {
                    // Union of all channels across nodes
                    combined_response.channels.extend(response.channels);
                }

                RequestType::SocketsCount => {
                    // Sum socket counts from all nodes
                    combined_response.sockets_count += response.sockets_count;
                }

                RequestType::ChannelMembersCount => {
                    // FIXED: Actually sum the members count
                    combined_response.members_count += response.members_count;
                }
            }
        }

        // Convert unique socket IDs back to Vec for responses that need it
        if matches!(
            request_type,
            RequestType::ChannelSockets | RequestType::Sockets
        ) {
            combined_response.socket_ids = unique_socket_ids.into_iter().collect();
            // Update sockets_count to reflect unique count for these request types
            if matches!(request_type, RequestType::ChannelSockets) {
                combined_response.sockets_count = combined_response.socket_ids.len();
            }
        }

        combined_response
    }

    pub fn validate_aggregated_response(
        &self,
        response: &ResponseBody,
        request_type: &RequestType,
    ) -> Result<()> {
        match request_type {
            RequestType::ChannelSocketsCount | RequestType::SocketsCount => {
                if response.sockets_count == 0 && !response.socket_ids.is_empty() {
                    warn!("Inconsistent response: sockets_count is 0 but socket_ids is not empty");
                }
            }
            RequestType::ChannelMembersCount => {
                if response.members_count == 0 && !response.members.is_empty() {
                    warn!("Inconsistent response: members_count is 0 but members map is not empty");
                }
            }
            RequestType::ChannelsWithSocketsCount => {
                let total_from_channels: usize =
                    response.channels_with_sockets_count.values().sum();
                if total_from_channels == 0 && response.sockets_count > 0 {
                    warn!("Inconsistent response: channels show 0 sockets but sockets_count > 0");
                }
            }
            _ => {} // No specific validation needed for other types
        }

        Ok(())
    }
}
