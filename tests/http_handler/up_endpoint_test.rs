use crate::mocks::connection_handler_mock::{create_test_connection_handler_with_app_manager, MockAppManager};
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
        Arc::new(AppsAvailableMockAppManager) as Arc<dyn sockudo::app::manager::AppManager + Send + Sync>,
        Arc::new(tokio::sync::RwLock::new(sockudo::channel::ChannelManager::new(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        ))))),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockAdapter::new())),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockCacheManager::new())),
        Some(Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockMetricsInterface::new()))),
        None,
        sockudo::options::ServerOptions::default(),
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
    let result = up(
        Some(Path("test_app".to_string())), 
        State(handler_arc)
    ).await;
    
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
    let result = up(
        Some(Path("test_app".to_string())), 
        State(handler_arc)
    ).await;
    
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
        State(handler_arc)
    ).await;
    
    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers().get("X-Health-Check").unwrap(), "NOT_FOUND");
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
        Arc::new(tokio::sync::RwLock::new(sockudo::channel::ChannelManager::new(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        ))))),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockAdapter::new())),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockCacheManager::new())),
        Some(Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockMetricsInterface::new()))),
        None,
        sockudo::options::ServerOptions::default(),
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
        Arc::new(tokio::sync::RwLock::new(sockudo::channel::ChannelManager::new(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        ))))),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockAdapter::new())),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockCacheManager::new())),
        Some(Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockMetricsInterface::new()))),
        None,
        sockudo::options::ServerOptions::default(),
    );
    let handler_arc = Arc::new(handler);
    
    // Call the up endpoint with app_id (should hit app manager error in find_by_id)
    let result = up(
        Some(Path("test_app".to_string())), 
        State(handler_arc)
    ).await;
    
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
        Arc::new(tokio::sync::RwLock::new(sockudo::channel::ChannelManager::new(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        ))))),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockAdapter::new())),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockCacheManager::new())),
        Some(Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockMetricsInterface::new()))),
        None,
        sockudo::options::ServerOptions::default(),
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
        Arc::new(tokio::sync::RwLock::new(sockudo::channel::ChannelManager::new(Arc::new(tokio::sync::Mutex::new(
            crate::mocks::connection_handler_mock::MockAdapter::new(),
        ))))),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockAdapter::new())),
        Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockCacheManager::new())),
        Some(Arc::new(tokio::sync::Mutex::new(crate::mocks::connection_handler_mock::MockMetricsInterface::new()))),
        None,
        sockudo::options::ServerOptions::default(),
    );
    let handler_arc = Arc::new(handler);
    
    // Call the up endpoint with app_id (should timeout on find_by_id)
    let result = up(
        Some(Path("test_app".to_string())), 
        State(handler_arc)
    ).await;
    
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
    let result = up(
        Some(Path("test_app".to_string())), 
        State(handler_arc)
    ).await;
    
    assert!(result.is_ok());
    let response = result.unwrap().into_response();
    assert_eq!(response.status(), StatusCode::OK);
    
    // The test passes if metrics recording doesn't cause any issues
    // Actual metrics verification would require a more sophisticated mock
}