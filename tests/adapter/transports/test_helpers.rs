use sockudo::adapter::horizontal_adapter::{BroadcastMessage, RequestBody, ResponseBody};
use sockudo::adapter::transports::RedisAdapterConfig;
use sockudo::options::{NatsAdapterConfig, RedisClusterAdapterConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use uuid::Uuid;

/// Get Redis configuration for single instance testing (localhost:16379)
pub fn get_redis_config() -> RedisAdapterConfig {
    RedisAdapterConfig {
        url: "redis://127.0.0.1:16379/".to_string(),
        prefix: format!("test_{}", Uuid::new_v4().to_string().replace('-', "")),
        request_timeout_ms: 1000, // Reduced timeout
        cluster_mode: false,
    }
}

/// Get Redis Cluster configuration for testing (ports 7001-7003)
pub fn get_redis_cluster_config() -> RedisClusterAdapterConfig {
    RedisClusterAdapterConfig {
        nodes: vec![
            "redis://127.0.0.1:7003".to_string(),
            "redis://127.0.0.1:7001".to_string(),
            "redis://127.0.0.1:7002".to_string(),
        ],
        prefix: format!("test_{}", Uuid::new_v4().to_string().replace('-', "")),
        request_timeout_ms: 1000,      // Reduced timeout
        use_connection_manager: false, // Disable for grokzen cluster compatibility
    }
}

/// Get NATS configuration for testing (ports 14222-14223)  
pub fn get_nats_config() -> NatsAdapterConfig {
    NatsAdapterConfig {
        servers: vec![
            "nats://127.0.0.1:14222".to_string(),
            "nats://127.0.0.1:14223".to_string(),
        ],
        prefix: format!("test_{}", Uuid::new_v4().to_string().replace('-', "")),
        request_timeout_ms: 1000,    // Reduced timeout
        connection_timeout_ms: 1000, // Reduced timeout
        username: None,
        password: None,
        token: None,
        nodes_number: Some(2),
    }
}

/// Create a test broadcast message
pub fn create_test_broadcast(event: &str) -> BroadcastMessage {
    BroadcastMessage {
        node_id: "test-node".to_string(),
        app_id: "test-app".to_string(),
        channel: "test-channel".to_string(),
        message: format!(
            "{{\"event\": \"{}\", \"data\": {{\"test\": \"data\"}}}}",
            event
        ),
        except_socket_id: None,
        timestamp_ms: None,
    }
}

/// Create a test request message
pub fn create_test_request() -> RequestBody {
    RequestBody {
        request_id: Uuid::new_v4().to_string(),
        node_id: "test-node".to_string(),
        app_id: "test-app".to_string(),
        request_type: sockudo::adapter::horizontal_adapter::RequestType::Sockets,
        channel: None,
        socket_id: None,
        user_id: None,
        user_info: None,
        timestamp: None,
        dead_node_id: None,
        target_node_id: None,
    }
}

/// Create a test response message
pub fn create_test_response(request_id: &str) -> ResponseBody {
    use std::collections::{HashMap, HashSet};
    ResponseBody {
        request_id: request_id.to_string(),
        node_id: "test-node".to_string(),
        app_id: "test-app".to_string(),
        members: HashMap::new(),
        channels_with_sockets_count: HashMap::new(),
        socket_ids: vec![],
        sockets_count: 0,
        exists: false,
        channels: HashSet::new(),
        members_count: 0,
    }
}

/// Message collector for testing listeners
#[derive(Clone)]
pub struct MessageCollector {
    broadcasts: Arc<Mutex<Vec<BroadcastMessage>>>,
    requests: Arc<Mutex<Vec<RequestBody>>>,
    responses: Arc<Mutex<Vec<ResponseBody>>>,
}

impl Default for MessageCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageCollector {
    pub fn new() -> Self {
        Self {
            broadcasts: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn collect_broadcast(&self, msg: BroadcastMessage) {
        self.broadcasts.lock().await.push(msg);
    }

    pub async fn collect_request(&self, msg: RequestBody) {
        self.requests.lock().await.push(msg);
    }

    pub async fn collect_response(&self, msg: ResponseBody) {
        self.responses.lock().await.push(msg);
    }

    pub async fn get_broadcasts(&self) -> Vec<BroadcastMessage> {
        self.broadcasts.lock().await.clone()
    }

    pub async fn get_requests(&self) -> Vec<RequestBody> {
        self.requests.lock().await.clone()
    }

    pub async fn get_responses(&self) -> Vec<ResponseBody> {
        self.responses.lock().await.clone()
    }

    pub async fn wait_for_broadcast(&self, timeout_ms: u64) -> Option<BroadcastMessage> {
        let start = tokio::time::Instant::now();
        let timeout_duration = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout_duration {
            let broadcasts = self.broadcasts.lock().await;
            if !broadcasts.is_empty() {
                return Some(broadcasts[0].clone());
            }
            drop(broadcasts);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }

    pub async fn wait_for_request(&self, timeout_ms: u64) -> Option<RequestBody> {
        let start = tokio::time::Instant::now();
        let timeout_duration = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout_duration {
            let requests = self.requests.lock().await;
            if !requests.is_empty() {
                return Some(requests[0].clone());
            }
            drop(requests);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }

    pub async fn wait_for_response(&self, timeout_ms: u64) -> Option<ResponseBody> {
        let start = tokio::time::Instant::now();
        let timeout_duration = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout_duration {
            let responses = self.responses.lock().await;
            if !responses.is_empty() {
                return Some(responses[0].clone());
            }
            drop(responses);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }

    pub async fn clear(&self) {
        self.broadcasts.lock().await.clear();
        self.requests.lock().await.clear();
        self.responses.lock().await.clear();
    }
}

/// Wait for a condition with timeout
pub async fn wait_for_condition<F, Fut>(condition: F, timeout_ms: u64) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let result = timeout(Duration::from_millis(timeout_ms), async {
        loop {
            if condition().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    result.unwrap_or(false)
}

/// Create transport handlers for testing
pub fn create_test_handlers(
    collector: MessageCollector,
) -> sockudo::adapter::horizontal_transport::TransportHandlers {
    use sockudo::adapter::horizontal_transport::BoxFuture;

    let broadcast_collector = collector.clone();
    let request_collector = collector.clone();
    let response_collector = collector.clone();

    sockudo::adapter::horizontal_transport::TransportHandlers {
        on_broadcast: Arc::new(move |msg| {
            let collector = broadcast_collector.clone();
            Box::pin(async move {
                collector.collect_broadcast(msg).await;
            }) as BoxFuture<'static, ()>
        }),
        on_request: Arc::new(move |msg| {
            let collector = request_collector.clone();
            Box::pin(async move {
                collector.collect_request(msg.clone()).await;
                // Return a dummy response for testing
                use std::collections::{HashMap, HashSet};
                Ok(ResponseBody {
                    request_id: msg.request_id,
                    node_id: "test-node".to_string(),
                    app_id: "test-app".to_string(),
                    members: HashMap::new(),
                    channels_with_sockets_count: HashMap::new(),
                    socket_ids: vec![],
                    sockets_count: 0,
                    exists: false,
                    channels: HashSet::new(),
                    members_count: 0,
                })
            }) as BoxFuture<'static, sockudo::error::Result<ResponseBody>>
        }),
        on_response: Arc::new(move |msg| {
            let collector = response_collector.clone();
            Box::pin(async move {
                collector.collect_response(msg).await;
            }) as BoxFuture<'static, ()>
        }),
    }
}
