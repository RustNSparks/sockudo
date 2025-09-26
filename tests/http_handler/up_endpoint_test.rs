use crate::mocks::connection_handler_mock::{
    MockAppManager, create_test_connection_handler_with_app_manager,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sockudo::app::config::App;
use sockudo::http_handler::up;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

// Helper to create a test app
fn create_test_app(app_id: &str, enabled: bool) -> App {
    App {
        id: app_id.to_string(),
        key: format!("{app_id}_key"),
        secret: format!("{app_id}_secret"),
        enabled,
        max_connections: 100,
        enable_client_messages: true,
        max_client_events_per_second: 100,
        max_presence_members_per_channel: Some(100),
        max_event_payload_in_kb: Some(10),
        ..Default::default()
    }
}

// Custom mock app manager that returns apps (non-empty list)
struct AppsAvailableMockAppManager;

#[async_trait::async_trait]
impl sockudo::app::manager::AppManager for AppsAvailableMockAppManager {
    async fn init(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn create_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn update_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn delete_app(&self, _app_id: &str) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_apps(&self) -> sockudo::error::Result<Vec<App>> {
        // Return at least one app to pass the "apps exist" check
        Ok(vec![create_test_app("default_app", true)])
    }
    async fn find_by_key(&self, _key: &str) -> sockudo::error::Result<Option<App>> {
        Ok(None)
    }
    async fn find_by_id(&self, _id: &str) -> sockudo::error::Result<Option<App>> {
        Ok(None)
    }
    async fn check_health(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_up_general_health_check_with_apps() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(AppsAvailableMockAppManager)
            as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockCacheManager::new(),
        )),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint without app_id
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "OK");
}

#[tokio::test]
async fn test_up_general_health_check_no_apps() {
    // Create app manager that returns empty apps list
    let app_manager = MockAppManager::new(); // Default returns empty Vec
    let handler = create_test_connection_handler_with_app_manager(app_manager);
    let handler_arc = Arc::new(handler);

    // Call the up endpoint without app_id
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

#[tokio::test]
async fn test_up_specific_app_exists_and_enabled() {
    let mut app_manager = MockAppManager::new();
    let test_app = create_test_app("test_app", true);
    app_manager.expect_find_by_id("test_app".to_string(), test_app);

    let handler = create_test_connection_handler_with_app_manager(app_manager);
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with specific app_id
    let result = up(Some(Path("test_app".to_string())), State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "OK");
}

#[tokio::test]
async fn test_up_specific_app_exists_but_disabled() {
    let mut app_manager = MockAppManager::new();
    let test_app = create_test_app("test_app", false); // disabled
    app_manager.expect_find_by_id("test_app".to_string(), test_app);

    let handler = create_test_connection_handler_with_app_manager(app_manager);
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with specific app_id
    let result = up(Some(Path("test_app".to_string())), State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

#[tokio::test]
async fn test_up_specific_app_not_found() {
    let app_manager = MockAppManager::new();
    // Don't set up any expectations, so find_by_id will return None

    let handler = create_test_connection_handler_with_app_manager(app_manager);
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with non-existent app_id
    let result = up(
        Some(Path("nonexistent_app".to_string())),
        State(handler_arc),
    )
    .await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers().get("X-Health-Check").unwrap(),
        "NOT_FOUND"
    );
}

// Custom mock app manager that simulates errors
struct ErrorMockAppManager;

#[async_trait::async_trait]
impl sockudo::app::manager::AppManager for ErrorMockAppManager {
    async fn init(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn create_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn update_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn delete_app(&self, _app_id: &str) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_apps(&self) -> sockudo::error::Result<Vec<App>> {
        Err(sockudo::error::Error::ApplicationNotFound)
    }
    async fn find_by_key(&self, _key: &str) -> sockudo::error::Result<Option<App>> {
        Ok(None)
    }
    async fn find_by_id(&self, _id: &str) -> sockudo::error::Result<Option<App>> {
        Err(sockudo::error::Error::ApplicationNotFound)
    }
    async fn check_health(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_up_general_health_check_app_manager_error() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(ErrorMockAppManager) as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockCacheManager::new(),
        )),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint without app_id (should hit app manager error)
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

#[tokio::test]
async fn test_up_specific_app_manager_error() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(ErrorMockAppManager) as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockCacheManager::new(),
        )),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with app_id (should hit app manager error in find_by_id)
    let result = up(Some(Path("test_app".to_string())), State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

// Mock app manager that simulates timeout by sleeping longer than the timeout
struct TimeoutMockAppManager;

#[async_trait::async_trait]
impl sockudo::app::manager::AppManager for TimeoutMockAppManager {
    async fn init(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn create_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn update_app(&self, _app: App) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn delete_app(&self, _app_id: &str) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_apps(&self) -> sockudo::error::Result<Vec<App>> {
        // Sleep longer than HEALTH_CHECK_TIMEOUT_MS (400ms)
        sleep(Duration::from_millis(500)).await;
        Ok(vec![])
    }
    async fn find_by_key(&self, _key: &str) -> sockudo::error::Result<Option<App>> {
        Ok(None)
    }
    async fn find_by_id(&self, _id: &str) -> sockudo::error::Result<Option<App>> {
        // Sleep longer than HEALTH_CHECK_TIMEOUT_MS (400ms)
        sleep(Duration::from_millis(500)).await;
        Ok(None)
    }
    async fn check_health(&self) -> sockudo::error::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_up_general_health_check_timeout() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(TimeoutMockAppManager) as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockCacheManager::new(),
        )),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint without app_id (should timeout on get_apps)
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

#[tokio::test]
async fn test_up_specific_app_timeout() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(TimeoutMockAppManager) as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockCacheManager::new(),
        )),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with app_id (should timeout on find_by_id)
    let result = up(Some(Path("test_app".to_string())), State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

// Mock adapter that fails health check
struct FailingAdapter;

#[async_trait::async_trait]
impl sockudo::adapter::ConnectionManager for FailingAdapter {
    async fn init(&mut self) {}
    async fn get_namespace(&mut self, _app_id: &str) -> Option<Arc<sockudo::namespace::Namespace>> {
        None
    }
    async fn add_socket(
        &mut self,
        _socket_id: sockudo::websocket::SocketId,
        _socket: fastwebsockets::WebSocketWrite<
            tokio::io::WriteHalf<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>,
        >,
        _app_id: &str,
        _app_manager: &Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_connection(
        &mut self,
        _socket_id: &sockudo::websocket::SocketId,
        _app_id: &str,
    ) -> Option<sockudo::websocket::WebSocketRef> {
        None
    }
    async fn remove_connection(
        &mut self,
        _socket_id: &sockudo::websocket::SocketId,
        _app_id: &str,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn send_message(
        &mut self,
        _app_id: &str,
        _socket_id: &sockudo::websocket::SocketId,
        _message: sockudo::protocol::messages::PusherMessage,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn send(
        &mut self,
        _channel: &str,
        _message: sockudo::protocol::messages::PusherMessage,
        _except: Option<&sockudo::websocket::SocketId>,
        _app_id: &str,
        _start_time_ms: Option<f64>,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_channel_members(
        &mut self,
        _app_id: &str,
        _channel: &str,
    ) -> sockudo::error::Result<
        std::collections::HashMap<String, sockudo::channel::PresenceMemberInfo>,
    > {
        Ok(std::collections::HashMap::new())
    }
    async fn get_channel_sockets(
        &mut self,
        _app_id: &str,
        _channel: &str,
    ) -> sockudo::error::Result<dashmap::DashSet<sockudo::websocket::SocketId>> {
        Ok(dashmap::DashSet::new())
    }
    async fn remove_channel(&mut self, _app_id: &str, _channel: &str) {}
    async fn is_in_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &sockudo::websocket::SocketId,
    ) -> sockudo::error::Result<bool> {
        Ok(false)
    }
    async fn get_user_sockets(
        &mut self,
        _user_id: &str,
        _app_id: &str,
    ) -> sockudo::error::Result<dashmap::DashSet<sockudo::websocket::WebSocketRef>> {
        Ok(dashmap::DashSet::new())
    }
    async fn cleanup_connection(&mut self, _app_id: &str, _ws: sockudo::websocket::WebSocketRef) {}
    async fn terminate_connection(
        &mut self,
        _app_id: &str,
        _user_id: &str,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn add_channel_to_sockets(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &sockudo::websocket::SocketId,
    ) {
    }
    async fn get_channel_socket_count(&mut self, _app_id: &str, _channel: &str) -> usize {
        0
    }
    async fn add_to_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &sockudo::websocket::SocketId,
    ) -> sockudo::error::Result<bool> {
        Ok(false)
    }
    async fn remove_from_channel(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &sockudo::websocket::SocketId,
    ) -> sockudo::error::Result<bool> {
        Ok(false)
    }
    async fn get_presence_member(
        &mut self,
        _app_id: &str,
        _channel: &str,
        _socket_id: &sockudo::websocket::SocketId,
    ) -> Option<sockudo::channel::PresenceMemberInfo> {
        None
    }
    async fn terminate_user_connections(
        &mut self,
        _app_id: &str,
        _user_id: &str,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn add_user(
        &mut self,
        _ws: sockudo::websocket::WebSocketRef,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn remove_user(
        &mut self,
        _ws: sockudo::websocket::WebSocketRef,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn get_channels_with_socket_count(
        &mut self,
        _app_id: &str,
    ) -> sockudo::error::Result<dashmap::DashMap<String, usize>> {
        Ok(dashmap::DashMap::new())
    }
    async fn get_sockets_count(&self, _app_id: &str) -> sockudo::error::Result<usize> {
        Ok(0)
    }
    async fn get_namespaces(
        &mut self,
    ) -> sockudo::error::Result<dashmap::DashMap<String, Arc<sockudo::namespace::Namespace>>> {
        Ok(dashmap::DashMap::new())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    async fn remove_user_socket(
        &mut self,
        _user_id: &str,
        _socket_id: &sockudo::websocket::SocketId,
        _app_id: &str,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }

    async fn count_user_connections_in_channel(
        &mut self,
        _user_id: &str,
        _app_id: &str,
        _channel: &str,
        _excluding_socket: Option<&sockudo::websocket::SocketId>,
    ) -> sockudo::error::Result<usize> {
        Ok(0)
    }

    // This is the key - make health check fail
    async fn check_health(&self) -> sockudo::error::Result<()> {
        Err(sockudo::error::Error::ApplicationNotFound)
    }

    fn get_node_id(&self) -> String {
        "test-node".to_string()
    }

    fn as_horizontal_adapter(
        &self,
    ) -> Option<&dyn sockudo::adapter::connection_manager::HorizontalAdapterInterface> {
        None
    }
}

// Mock cache manager that fails health check
struct FailingCacheManager;

#[async_trait::async_trait]
impl sockudo::cache::manager::CacheManager for FailingCacheManager {
    async fn has(&mut self, _key: &str) -> sockudo::error::Result<bool> {
        Ok(false)
    }
    async fn get(&mut self, _key: &str) -> sockudo::error::Result<Option<String>> {
        Ok(None)
    }
    async fn set(
        &mut self,
        _key: &str,
        _value: &str,
        _ttl_seconds: u64,
    ) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn remove(&mut self, _key: &str) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn disconnect(&mut self) -> sockudo::error::Result<()> {
        Ok(())
    }
    async fn ttl(&mut self, _key: &str) -> sockudo::error::Result<Option<Duration>> {
        Ok(None)
    }

    // This is the key - make health check fail
    async fn check_health(&self) -> sockudo::error::Result<()> {
        Err(sockudo::error::Error::ApplicationNotFound)
    }
}

#[tokio::test]
async fn test_up_adapter_health_check_failure() {
    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(AppsAvailableMockAppManager)
            as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(FailingAdapter)),
        Arc::new(tokio::sync::Mutex::new(FailingCacheManager)),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        sockudo::options::ServerOptions::default(),
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint - adapter failure should return ERROR
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

#[tokio::test]
async fn test_up_cache_health_check_failure() {
    // Create server options with cache enabled (not None)
    let mut server_options = sockudo::options::ServerOptions::default();
    server_options.cache.driver = sockudo::options::CacheDriver::Memory;

    let handler = sockudo::adapter::handler::ConnectionHandler::new(
        Arc::new(AppsAvailableMockAppManager)
            as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        )),
        Arc::new(tokio::sync::Mutex::new(FailingCacheManager)),
        Some(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockMetricsInterface::new(),
        ))),
        None,
        server_options,
        None,
    );
    let handler_arc = Arc::new(handler);

    // Call the up endpoint - cache failure should return ERROR (critical component)
    let result = up(None, State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "ERROR");
}

// Test that metrics are recorded (non-blocking)
#[tokio::test]
async fn test_up_records_metrics() {
    let mut app_manager = MockAppManager::new();
    let test_app = create_test_app("test_app", true);
    app_manager.expect_find_by_id("test_app".to_string(), test_app);

    let handler = create_test_connection_handler_with_app_manager(app_manager);
    let handler_arc = Arc::new(handler);

    // Call the up endpoint with specific app_id
    let result = up(Some(Path("test_app".to_string())), State(handler_arc)).await;

    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);

    // The test passes if metrics recording doesn't cause any issues
    // Actual metrics verification would require a more sophisticated mock
}
