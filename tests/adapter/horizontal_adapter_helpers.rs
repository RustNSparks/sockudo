use async_trait::async_trait;
use sockudo::adapter::horizontal_adapter::{
    BroadcastMessage, RequestBody, RequestType, ResponseBody,
};
use sockudo::adapter::horizontal_transport::{
    HorizontalTransport, TransportConfig, TransportHandlers,
};
use sockudo::channel::PresenceMemberInfo;
use sockudo::error::{Error, Result};
use sonic_rs::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Realistic state for each simulated node
#[derive(Clone, Debug)]
pub struct MockNodeState {
    pub node_id: String,
    pub sockets: Vec<String>,
    pub channels: HashMap<String, Vec<String>>, // channel -> socket_ids
    pub presence_members: HashMap<String, HashMap<String, Value>>, // channel -> user_id -> user_info
    pub user_sockets: HashMap<String, Vec<String>>,                // user_id -> socket_ids
    pub response_delay_ms: u64,
    pub will_respond: bool,
    pub corrupt_responses: bool,
}

impl MockNodeState {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            sockets: Vec::new(),
            channels: HashMap::new(),
            presence_members: HashMap::new(),
            user_sockets: HashMap::new(),
            response_delay_ms: 10,
            will_respond: true,
            corrupt_responses: false,
        }
    }

    pub fn with_sockets(mut self, sockets: Vec<&str>) -> Self {
        self.sockets = sockets.into_iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn with_channel(mut self, channel: &str, sockets: Vec<&str>) -> Self {
        self.channels.insert(
            channel.to_string(),
            sockets.into_iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    pub fn with_presence_member(mut self, channel: &str, user_id: &str, user_info: Value) -> Self {
        self.presence_members
            .entry(channel.to_string())
            .or_default()
            .insert(user_id.to_string(), user_info);
        self
    }

    pub fn with_user_sockets(mut self, user_id: &str, sockets: Vec<&str>) -> Self {
        self.user_sockets.insert(
            user_id.to_string(),
            sockets.into_iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    pub fn with_response_delay(mut self, delay_ms: u64) -> Self {
        self.response_delay_ms = delay_ms;
        self
    }

    pub fn will_not_respond(mut self) -> Self {
        self.will_respond = false;
        self
    }

    pub fn with_corrupt_responses(mut self) -> Self {
        self.corrupt_responses = true;
        self
    }
}

/// Mock configuration for testing with realistic distributed state
#[derive(Clone)]
pub struct MockConfig {
    pub prefix: String,
    pub request_timeout_ms: u64,
    pub simulate_failures: bool,
    pub healthy: bool,
    /// Each node's simulated state - realistic distributed data
    pub node_states: Vec<MockNodeState>,
}

impl Default for MockConfig {
    fn default() -> Self {
        // Create realistic overlapping distributed state
        let node1 = MockNodeState::new("node-1")
            .with_sockets(vec!["socket-1", "socket-2", "socket-shared"])
            .with_channel("channel-1", vec!["socket-1", "socket-shared"])
            .with_channel("presence-channel", vec!["socket-1"])
            .with_presence_member("presence-channel", "user-1", json!({"name": "Alice"}))
            .with_user_sockets("user-1", vec!["socket-1"])
            .with_user_sockets("shared-user", vec!["socket-shared"]);

        let node2 = MockNodeState::new("node-2")
            .with_sockets(vec!["socket-3", "socket-4", "socket-shared"])
            .with_channel("channel-1", vec!["socket-3", "socket-shared"])
            .with_channel("channel-2", vec!["socket-4"])
            .with_channel("presence-channel", vec!["socket-3"])
            .with_presence_member("presence-channel", "user-2", json!({"name": "Bob"}))
            .with_user_sockets("user-2", vec!["socket-3"])
            .with_user_sockets("shared-user", vec!["socket-shared"]);

        Self {
            prefix: "test".to_string(),
            request_timeout_ms: 1000,
            simulate_failures: false,
            healthy: true,
            node_states: vec![node1, node2],
        }
    }
}

impl TransportConfig for MockConfig {
    fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// Mock transport for testing HorizontalAdapterBase with realistic distributed behavior
#[derive(Clone)]
pub struct MockTransport {
    config: MockConfig,
    handlers: Arc<Mutex<Option<TransportHandlers>>>,
    published_broadcasts: Arc<Mutex<Vec<BroadcastMessage>>>,
    published_requests: Arc<Mutex<Vec<RequestBody>>>,
    published_responses: Arc<Mutex<Vec<ResponseBody>>>,
    healthy: Arc<Mutex<bool>>,
    node_count: Arc<Mutex<usize>>,
}

impl MockTransport {
    /// Create a new MockTransport with shared state for multi-node simulation
    pub fn new_with_shared_state(
        config: MockConfig,
        _shared_state: Arc<Mutex<HashMap<String, MockNodeState>>>,
    ) -> Self {
        Self {
            config,
            handlers: Arc::new(Mutex::new(None)),
            published_broadcasts: Arc::new(Mutex::new(Vec::new())),
            published_requests: Arc::new(Mutex::new(Vec::new())),
            published_responses: Arc::new(Mutex::new(Vec::new())),
            healthy: Arc::new(Mutex::new(true)),
            node_count: Arc::new(Mutex::new(1)),
        }
    }

    /// Set the health status of this transport (for simulating node failures)
    pub async fn set_health_status(&self, healthy: bool) {
        *self.healthy.lock().await = healthy;
    }

    /// Check if this transport is healthy
    #[allow(dead_code)]
    pub async fn is_healthy(&self) -> bool {
        *self.healthy.lock().await
    }

    /// Get access to published requests for testing
    pub async fn get_published_requests(
        &self,
    ) -> Vec<sockudo::adapter::horizontal_adapter::RequestBody> {
        self.published_requests.lock().await.clone()
    }

    /// Create test configurations for specific scenarios
    /// Configuration with conflicting data between nodes
    pub fn conflicting_data() -> MockConfig {
        let node1 = MockNodeState::new("node-1")
            .with_sockets(vec!["socket-1", "socket-shared"])
            .with_channel("test-channel", vec!["socket-1"])
            .with_presence_member(
                "presence-channel",
                "user-1",
                json!({"name": "Alice", "status": "online"}),
            );

        let node2 = MockNodeState::new("node-2")
            .with_sockets(vec!["socket-2", "socket-shared"])
            .with_channel("test-channel", vec!["socket-2", "socket-shared"]) // Different membership!
            .with_presence_member(
                "presence-channel",
                "user-1",
                json!({"name": "Alice Updated", "status": "busy"}),
            ); // Conflicting user info!

        MockConfig {
            prefix: "test".to_string(),
            request_timeout_ms: 1000,
            simulate_failures: false,
            healthy: true,
            node_states: vec![node1, node2],
        }
    }

    /// Configuration with partial node failures
    pub fn partial_failures() -> MockConfig {
        let node1 = MockNodeState::new("node-1")
            .with_sockets(vec!["socket-1", "socket-2"])
            .with_channel("test-channel", vec!["socket-1"]);

        let node2 = MockNodeState::new("node-2")
            .with_sockets(vec!["socket-3", "socket-4"])
            .with_channel("test-channel", vec!["socket-3"])
            .will_not_respond(); // This node will not respond

        let node3 = MockNodeState::new("node-3")
            .with_sockets(vec!["socket-5"])
            .with_channel("test-channel", vec!["socket-5"])
            .with_response_delay(2000); // This node will respond too late

        MockConfig {
            prefix: "test".to_string(),
            request_timeout_ms: 500,
            simulate_failures: false,
            healthy: true,
            node_states: vec![node1, node2, node3],
        }
    }

    /// Configuration with large scale data to test performance
    pub fn large_scale() -> MockConfig {
        let mut nodes = Vec::new();
        for i in 0..10 {
            let mut node = MockNodeState::new(&format!("node-{}", i));
            // Each node has 100 sockets
            node.sockets = (0..100).map(|s| format!("socket-{}-{}", i, s)).collect();
            // Each node has sockets in 20 channels
            for c in 0..20 {
                let channel_sockets: Vec<String> =
                    (0..5).map(|s| format!("socket-{}-{}", i, s)).collect();
                node.channels
                    .insert(format!("channel-{}", c), channel_sockets);
            }
            nodes.push(node);
        }

        MockConfig {
            prefix: "test".to_string(),
            request_timeout_ms: 1000,
            simulate_failures: false,
            healthy: true,
            node_states: nodes,
        }
    }

    /// Configuration with corrupt response data
    pub fn corrupt_responses() -> MockConfig {
        let node1 = MockNodeState::new("node-1")
            .with_sockets(vec!["socket-1"])
            .with_corrupt_responses();

        let node2 = MockNodeState::new("node-2").with_sockets(vec!["socket-2"]);

        MockConfig {
            prefix: "test".to_string(),
            request_timeout_ms: 1000,
            simulate_failures: false,
            healthy: true,
            node_states: vec![node1, node2],
        }
    }
}

impl MockTransport {
    /// Get published broadcasts for verification
    pub async fn get_published_broadcasts(&self) -> Vec<BroadcastMessage> {
        self.published_broadcasts.lock().await.clone()
    }

    /// Simulate a node joining the cluster
    pub async fn add_node(&self) {
        let mut count = self.node_count.lock().await;
        *count += 1;
    }

    /// Simulate a node leaving the cluster
    pub async fn remove_node(&self) {
        let mut count = self.node_count.lock().await;
        if *count > 0 {
            *count -= 1;
        }
    }

    /// Simulate realistic response based on node state and request
    fn create_realistic_response(
        &self,
        node_state: &MockNodeState,
        request: &RequestBody,
    ) -> ResponseBody {
        if node_state.corrupt_responses {
            // Return corrupted data to test error handling
            return ResponseBody {
                request_id: request.request_id.clone(),
                node_id: node_state.node_id.clone(),
                app_id: request.app_id.clone(),
                members: HashMap::new(),
                socket_ids: vec!["CORRUPTED_DATA_💀".to_string()],
                sockets_count: 999_999_999, // Large but safe value to test corruption handling
                channels_with_sockets_count: HashMap::new(),
                exists: true,
                channels: HashSet::new(),
                members_count: 999_999_999,
            };
        }

        let mut response = ResponseBody {
            request_id: request.request_id.clone(),
            node_id: node_state.node_id.clone(),
            app_id: request.app_id.clone(),
            members: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            channels_with_sockets_count: HashMap::new(),
            exists: false,
            channels: HashSet::new(),
            members_count: 0,
        };

        match request.request_type {
            RequestType::Sockets => {
                response.socket_ids = node_state.sockets.clone();
                response.sockets_count = node_state.sockets.len();
            }
            RequestType::ChannelMembers => {
                if let Some(channel) = &request.channel
                    && let Some(members) = node_state.presence_members.get(channel)
                {
                    // Convert Value members to PresenceMemberInfo
                    let presence_members: HashMap<String, PresenceMemberInfo> = members
                        .iter()
                        .map(|(user_id, user_info)| {
                            let member_info = PresenceMemberInfo {
                                user_id: user_id.clone(),
                                user_info: Some(user_info.clone()),
                            };
                            (user_id.clone(), member_info)
                        })
                        .collect();
                    response.members = presence_members;
                    response.members_count = members.len();
                }
            }
            RequestType::ChannelSockets => {
                if let Some(channel) = &request.channel
                    && let Some(sockets) = node_state.channels.get(channel)
                {
                    response.socket_ids = sockets.clone();
                    response.sockets_count = sockets.len();
                }
            }
            RequestType::ChannelSocketsCount => {
                if let Some(channel) = &request.channel
                    && let Some(sockets) = node_state.channels.get(channel)
                {
                    response.sockets_count = sockets.len();
                }
            }
            RequestType::SocketExistsInChannel => {
                if let Some(channel) = &request.channel
                    && let Some(socket_id) = &request.socket_id
                    && let Some(sockets) = node_state.channels.get(channel)
                {
                    response.exists = sockets.contains(socket_id);
                }
            }
            RequestType::Channels => {
                response.channels = node_state.channels.keys().cloned().collect();
            }
            RequestType::SocketsCount => {
                response.sockets_count = node_state.sockets.len();
            }
            RequestType::ChannelMembersCount => {
                if let Some(channel) = &request.channel
                    && let Some(members) = node_state.presence_members.get(channel)
                {
                    response.members_count = members.len();
                }
            }
            RequestType::CountUserConnectionsInChannel => {
                if let Some(channel) = &request.channel
                    && let Some(user_id) = &request.user_id
                {
                    let count = node_state
                        .channels
                        .get(channel)
                        .map(|sockets| {
                            node_state
                                .user_sockets
                                .get(user_id)
                                .map(|user_sockets| {
                                    sockets.iter().filter(|s| user_sockets.contains(s)).count()
                                })
                                .unwrap_or(0)
                        })
                        .unwrap_or(0);
                    response.sockets_count = count;
                }
            }
            _ => {} // Handle other request types as needed
        }

        response
    }
}

#[async_trait]
impl HorizontalTransport for MockTransport {
    type Config = MockConfig;

    async fn new(config: Self::Config) -> Result<Self> {
        if config.simulate_failures && config.prefix == "fail_new" {
            return Err(Error::Internal("Simulated constructor failure".to_string()));
        }

        Ok(Self {
            healthy: Arc::new(Mutex::new(config.healthy)),
            node_count: Arc::new(Mutex::new(config.node_states.len())),
            config,
            handlers: Arc::new(Mutex::new(None)),
            published_broadcasts: Arc::new(Mutex::new(Vec::new())),
            published_requests: Arc::new(Mutex::new(Vec::new())),
            published_responses: Arc::new(Mutex::new(Vec::new())),
        })
    }

    async fn publish_broadcast(&self, message: &BroadcastMessage) -> Result<()> {
        if self.config.simulate_failures && message.message.contains("simulate_error") {
            return Err(Error::Internal("Simulated broadcast failure".to_string()));
        }

        self.published_broadcasts.lock().await.push(message.clone());

        // Simulate receiving our own broadcast with realistic handler behavior
        let handlers_guard = self.handlers.lock().await;
        if let Some(handlers) = handlers_guard.as_ref() {
            let on_broadcast = handlers.on_broadcast.clone();
            let message = message.clone();
            drop(handlers_guard);
            tokio::spawn(async move {
                // Simulate network delay
                if let Ok(delay) = std::env::var("MOCK_NETWORK_DELAY_MS")
                    && let Ok(delay_ms) = delay.parse::<u64>()
                {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                (on_broadcast)(message).await;
            });
        }

        Ok(())
    }

    async fn publish_request(&self, request: &RequestBody) -> Result<()> {
        if self.config.simulate_failures && request.app_id == "fail_request" {
            return Err(Error::Internal("Simulated request failure".to_string()));
        }

        self.published_requests.lock().await.push(request.clone());

        // Simulate realistic responses from configured node states
        let handlers_guard = self.handlers.lock().await;
        if let Some(handlers) = handlers_guard.as_ref() {
            let on_response = handlers.on_response.clone();
            let request = request.clone();
            let node_states = self.config.node_states.clone();

            drop(handlers_guard);

            // Spawn responses from each configured node
            for node_state in node_states {
                if !node_state.will_respond {
                    continue; // Skip nodes that won't respond
                }

                let on_response = on_response.clone();
                let request = request.clone();
                let node_state = node_state.clone();

                tokio::spawn(async move {
                    // Simulate network/processing delay
                    if node_state.response_delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(node_state.response_delay_ms))
                            .await;
                    }

                    // Don't respond if simulating failures for this request
                    if request.request_id.contains("no_response") {
                        return;
                    }

                    // Create realistic response based on node state
                    let dummy_transport = MockTransport {
                        config: MockConfig::default(),
                        handlers: Arc::new(Mutex::new(None)),
                        published_broadcasts: Arc::new(Mutex::new(Vec::new())),
                        published_requests: Arc::new(Mutex::new(Vec::new())),
                        published_responses: Arc::new(Mutex::new(Vec::new())),
                        healthy: Arc::new(Mutex::new(true)),
                        node_count: Arc::new(Mutex::new(1)),
                    };

                    let response = dummy_transport.create_realistic_response(&node_state, &request);

                    (on_response)(response).await;
                });
            }
        }

        Ok(())
    }

    async fn publish_response(&self, response: &ResponseBody) -> Result<()> {
        self.published_responses.lock().await.push(response.clone());
        Ok(())
    }

    async fn start_listeners(&self, handlers: TransportHandlers) -> Result<()> {
        *self.handlers.lock().await = Some(handlers);
        Ok(())
    }

    async fn get_node_count(&self) -> Result<usize> {
        Ok(*self.node_count.lock().await)
    }

    async fn check_health(&self) -> Result<()> {
        if *self.healthy.lock().await {
            Ok(())
        } else {
            Err(Error::Internal("MockTransport is unhealthy".to_string()))
        }
    }
}
