use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use fastwebsockets::WebSocketWrite;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use sockudo::adapter::ConnectionManager;
use sockudo::adapter::handler::ConnectionHandler;
use sockudo::app::config::App;
use sockudo::app::manager::AppManager;
use sockudo::cache::manager::CacheManager;
use sockudo::channel::ChannelManager;
use sockudo::channel::PresenceMemberInfo;
use sockudo::error::Result;
use sockudo::metrics::MetricsInterface;
use sockudo::namespace::Namespace;
use sockudo::options::ServerOptions;
use sockudo::protocol::messages::PusherMessage;
use sockudo::rate_limiter::{RateLimitResult, RateLimiter};
use sockudo::websocket::{SocketId, WebSocketRef};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::WriteHalf;
use tokio::sync::{Mutex, RwLock};

pub struct MockAdapter {
    signature_valid: AtomicBool,
    expected_channel: Option<String>,
    expected_auth: Option<String>,
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAdapter {
    pub fn new() -> Self {
        Self {
            signature_valid: AtomicBool::new(true),
            expected_channel: None,
            expected_auth: None,
        }
    }

    pub fn expect_channel_auth(&mut self, channel: String, auth: String) {
        self.expected_channel = Some(channel);
        self.expected_auth = Some(auth);
    }

    pub fn set_signature_valid(&mut self, valid: bool) {
        self.signature_valid.store(valid, Ordering::SeqCst);
    }
}

#[async_trait]
impl ConnectionManager for MockAdapter {
    async fn init(&mut self) {}
    async fn get_namespace(&mut self, _app_id: &str) -> Option<Arc<Namespace>> {
        None
    }
    async fn add_socket(
        &mut self,
        _socket_id: SocketId,
        _socket: WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>,
        _app_id: &str,
        _app_manager: &Arc<dyn AppManager + Send + Sync>,
    ) -> Result<()> {
        Ok(())
    }
    async fn get_connection(
        &mut self,
        _socket_id: &SocketId,
        _app_id: &str,
    ) -> Option<WebSocketRef> {
        None
    }
    async fn remove_connection(&mut self, _socket_id: &SocketId, _app_id: &str) -> Result<()> {
        Ok(())
    }
    async fn send_message(
        &mut self,
        _app_id: &str,
        _socket_id: &SocketId,
        _message: PusherMessage,
    ) -> Result<()> {
        Ok(())
    }
    async fn send(
        &mut self,
        _channel: &str,
        _message: PusherMessage,
        _except: Option<&SocketId>,
        _app_id: &str,
        _start_time_ms: Option<f64>,
    ) -> Result<()> {
        Ok(())
    }
    async fn get_channel_members(
        &mut self,
        _app_id: &str,
        _channel: &str,
    ) -> Result<HashMap<String, PresenceMemberInfo>> {
        Ok(HashMap::new())
    }
    async fn get_channel_sockets(
        &mut self,
        _app_id: &str,
        _channel: &str,
    ) -> Result<DashSet<SocketId>> {
        Ok(DashSet::new())
    }
    async fn remove_channel(&mut self, _app_id: &str, _channel: &str) {}
    async fn is_in_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<bool> {
        Ok(false)
    }
    async fn get_user_sockets(
        &mut self,
        _user_id: &str,
        _app_id: &str,
    ) -> Result<DashSet<WebSocketRef>> {
        Ok(DashSet::new())
    }
    async fn cleanup_connection(&mut self, _app_id: &str, _ws: WebSocketRef) {}
    async fn terminate_connection(&mut self, _app_id: &str, _user_id: &str) -> Result<()> {
        Ok(())
    }
    async fn add_channel_to_sockets(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) {
    }
    async fn get_channel_socket_count(&mut self, _app_id: &str, _channel: &str) -> usize {
        0
    }
    async fn add_to_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<bool> {
        Ok(false)
    }
    async fn remove_from_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Result<bool> {
        Ok(false)
    }
    async fn get_presence_member(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &SocketId,
    ) -> Option<PresenceMemberInfo> {
        None
    }
    async fn terminate_user_connections(&mut self, _app_id: &str, _user_id: &str) -> Result<()> {
        Ok(())
    }
    async fn add_user(&mut self, _ws: WebSocketRef) -> Result<()> {
        Ok(())
    }
    async fn remove_user(&mut self, _ws: WebSocketRef) -> Result<()> {
        Ok(())
    }
    async fn get_channels_with_socket_count(
        &mut self,
        _app_id: &str,
    ) -> Result<DashMap<String, usize>> {
        Ok(DashMap::new())
    }
    async fn get_sockets_count(&self, _app_id: &str) -> Result<usize> {
        Ok(0)
    }
    async fn get_namespaces(&mut self) -> Result<DashMap<String, Arc<Namespace>>> {
        Ok(DashMap::new())
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    async fn check_health(&self) -> Result<()> {
        // Mock adapter is always healthy for testing
        Ok(())
    }
}

#[derive(Clone)]
pub struct MockAppManager {
    expected_key: Option<String>,
    expected_id: Option<String>,
    app_to_return: Option<App>,
}

impl Default for MockAppManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MockAppManager {
    pub fn new() -> Self {
        Self {
            expected_key: None,
            expected_id: None,
            app_to_return: None,
        }
    }

    pub fn expect_find_by_key(&mut self, key: String, app: App) {
        self.expected_key = Some(key);
        self.app_to_return = Some(app);
    }

    pub fn expect_find_by_id(&mut self, id: String, app: App) {
        self.expected_id = Some(id);
        self.app_to_return = Some(app);
    }
}

#[async_trait]
impl AppManager for MockAppManager {
    async fn init(&self) -> Result<()> {
        Ok(())
    }
    async fn create_app(&self, _app: App) -> Result<()> {
        Ok(())
    }
    async fn update_app(&self, _app: App) -> Result<()> {
        Ok(())
    }
    async fn delete_app(&self, _app_id: &str) -> Result<()> {
        Ok(())
    }
    async fn get_apps(&self) -> Result<Vec<App>> {
        Ok(Vec::new())
    }
    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        if let Some(expected_key) = &self.expected_key {
            assert_eq!(key, expected_key, "Unexpected app key in find_by_key");
        }
        Ok(self.app_to_return.clone())
    }
    async fn find_by_id(&self, id: &str) -> Result<Option<App>> {
        if let Some(expected_id) = &self.expected_id {
            assert_eq!(id, expected_id, "Unexpected app id in find_by_id");
        }
        Ok(self.app_to_return.clone())
    }
    async fn check_health(&self) -> Result<()> {
        Ok(())
    }
}

pub struct MockChannelManager {
    signature_valid: AtomicBool,
    expected_signature: Option<String>,
    expected_message: Option<PusherMessage>,
}

impl Clone for MockChannelManager {
    fn clone(&self) -> Self {
        Self {
            signature_valid: AtomicBool::new(self.signature_valid.load(Ordering::SeqCst)),
            expected_signature: self.expected_signature.clone(),
            expected_message: self.expected_message.clone(),
        }
    }
}

impl Default for MockChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MockChannelManager {
    pub fn new() -> Self {
        Self {
            signature_valid: AtomicBool::new(true),
            expected_signature: None,
            expected_message: None,
        }
    }

    pub fn expect_signature_validation(
        &mut self,
        signature: String,
        message: PusherMessage,
        valid: bool,
    ) {
        self.expected_signature = Some(signature);
        self.expected_message = Some(message);
        self.signature_valid.store(valid, Ordering::SeqCst);
    }

    pub fn signature_is_valid(
        &self,
        _app: App,
        _socket_id: &SocketId,
        signature: &str,
        message: PusherMessage,
    ) -> bool {
        if let Some(expected_signature) = &self.expected_signature {
            assert_eq!(
                signature, expected_signature,
                "Unexpected signature in validation"
            );
        }
        if let Some(expected_message) = &self.expected_message {
            assert_eq!(
                message.channel, expected_message.channel,
                "Unexpected channel in message"
            );
            assert_eq!(
                message.event, expected_message.event,
                "Unexpected event in message"
            );
        }
        self.signature_valid.load(Ordering::SeqCst)
    }
}

pub struct MockCacheManager;
impl Default for MockCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MockCacheManager {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CacheManager for MockCacheManager {
    async fn has(&mut self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn get(&mut self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn set(&mut self, _key: &str, _value: &str, _ttl_seconds: u64) -> Result<()> {
        Ok(())
    }
    async fn remove(&mut self, _key: &str) -> Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
    async fn ttl(&mut self, _key: &str) -> Result<Option<Duration>> {
        Ok(None)
    }
    async fn check_health(&self) -> Result<()> {
        Ok(())
    }
}

pub struct MockMetricsInterface;
impl Default for MockMetricsInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl MockMetricsInterface {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MetricsInterface for MockMetricsInterface {
    async fn init(&self) -> Result<()> {
        Ok(())
    }
    fn mark_new_connection(&self, _app_id: &str, _socket_id: &SocketId) {}
    fn mark_disconnection(&self, _app_id: &str, _socket_id: &SocketId) {}
    fn mark_connection_error(&self, _app_id: &str, _error_type: &str) {}
    fn mark_rate_limit_check(&self, _app_id: &str, _limiter_type: &str) {}
    fn mark_rate_limit_check_with_context(
        &self,
        _app_id: &str,
        _limiter_type: &str,
        _request_context: &str,
    ) {
    }
    fn mark_rate_limit_triggered(&self, _app_id: &str, _limiter_type: &str) {}
    fn mark_rate_limit_triggered_with_context(
        &self,
        _app_id: &str,
        _limiter_type: &str,
        _request_context: &str,
    ) {
    }
    fn mark_channel_subscription(&self, _app_id: &str, _channel_type: &str) {}
    fn mark_channel_unsubscription(&self, _app_id: &str, _channel_type: &str) {}
    fn update_active_channels(&self, _app_id: &str, _channel_type: &str, _count: i64) {}
    fn mark_api_message(
        &self,
        _app_id: &str,
        _incoming_message_size: usize,
        _sent_message_size: usize,
    ) {
    }
    fn mark_ws_message_sent(&self, _app_id: &str, _sent_message_size: usize) {}
    fn mark_ws_messages_sent_batch(&self, _app_id: &str, _sent_message_size: usize, _count: usize) {
    }
    fn mark_ws_message_received(&self, _app_id: &str, _message_size: usize) {}
    fn track_horizontal_adapter_resolve_time(&self, _app_id: &str, _time_ms: f64) {}
    fn track_horizontal_adapter_resolved_promises(&self, _app_id: &str, _resolved: bool) {}
    fn mark_horizontal_adapter_request_sent(&self, _app_id: &str) {}
    fn mark_horizontal_adapter_request_received(&self, _app_id: &str) {}
    fn mark_horizontal_adapter_response_received(&self, _app_id: &str) {}
    fn track_broadcast_latency(
        &self,
        _app_id: &str,
        _channel_name: &str,
        _recipient_count: usize,
        _latency_ms: f64,
    ) {
    }
    async fn get_metrics_as_plaintext(&self) -> String {
        String::new()
    }
    async fn get_metrics_as_json(&self) -> Value {
        serde_json::json!({})
    }
    async fn clear(&self) {}
}

pub struct MockRateLimiter;
impl Default for MockRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRateLimiter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RateLimiter for MockRateLimiter {
    async fn check(&self, _key: &str) -> Result<RateLimitResult> {
        Ok(RateLimitResult {
            allowed: true,
            remaining: u32::MAX,
            reset_after: 0,
            limit: u32::MAX,
        })
    }
    async fn increment(&self, _key: &str) -> Result<RateLimitResult> {
        Ok(RateLimitResult {
            allowed: true,
            remaining: u32::MAX,
            reset_after: 0,
            limit: u32::MAX,
        })
    }
    async fn reset(&self, _key: &str) -> Result<()> {
        Ok(())
    }
    async fn get_remaining(&self, _key: &str) -> Result<u32> {
        Ok(u32::MAX)
    }
}

// Helper function to create a test ConnectionHandler with configurable mocks
pub fn create_test_connection_handler() -> (ConnectionHandler, MockAppManager, MockChannelManager) {
    let app_manager = MockAppManager::new();
    let channel_manager = MockChannelManager::new();

    let handler = ConnectionHandler::new(
        Arc::new(app_manager.clone()) as Arc<dyn AppManager + Send + Sync>,
        Arc::new(RwLock::new(ChannelManager::new(Arc::new(Mutex::new(
            MockAdapter::new(),
        ))))),
        Arc::new(Mutex::new(MockAdapter::new())),
        Arc::new(Mutex::new(MockCacheManager::new())),
        Some(Arc::new(Mutex::new(MockMetricsInterface::new()))),
        None,
        ServerOptions::default(),
        None,
    );

    (handler, app_manager, channel_manager)
}

pub fn create_test_connection_handler_with_app_manager(
    app_manager: MockAppManager,
) -> ConnectionHandler {
    ConnectionHandler::new(
        Arc::new(app_manager.clone()) as Arc<dyn AppManager + Send + Sync>,
        Arc::new(RwLock::new(ChannelManager::new(Arc::new(Mutex::new(
            MockAdapter::new(),
        ))))),
        Arc::new(Mutex::new(MockAdapter::new())),
        Arc::new(Mutex::new(MockCacheManager::new())),
        Some(Arc::new(Mutex::new(MockMetricsInterface::new()))),
        None,
        ServerOptions::default(),
        None,
    )
}
