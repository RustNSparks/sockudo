use serde_json::json;
use sockudo::adapter::handler::types::SignInRequest;
use sockudo::app::auth::AuthValidator;
use sockudo::app::config::App;
use sockudo::token::Token;
use sockudo::websocket::SocketId;
use std::sync::Arc;

use crate::mocks::connection_handler_mock::{
    MockAppManager, create_test_connection_handler_with_app_manager,
};

#[tokio::test]
async fn test_signin_request_parsing_with_app_key_prefix() {
    // Test that SignInRequest correctly parses user_data from the Structured variant
    let user_data = json!({
        "id": "test-user-123",
        "user_info": {
            "name": "Test User",
            "email": "test@example.com"
        }
    })
    .to_string();

    let auth_string = "app-key:signature_here".to_string();

    let request = SignInRequest {
        user_data: user_data.clone(),
        auth: auth_string.clone(),
    };

    // Verify the request holds the correct data
    assert_eq!(request.user_data, user_data);
    assert_eq!(request.auth, auth_string);
    assert!(
        request.auth.contains(':'),
        "Auth should contain app-key:signature format"
    );
}

#[tokio::test]
async fn test_verify_signin_authentication_with_prefix() {
    let socket_id = SocketId::new();
    let app = create_test_app();

    let mut mock_app_manager = MockAppManager::new();
    mock_app_manager.expect_find_by_key("test-app-key".to_string(), app.clone());
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({"id": "user-123"}).to_string();
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new("test-app-key".to_string(), "test-app-secret".to_string());
    let signature = token.sign(&string_to_sign);

    let auth_with_prefix = format!("test-app-key:{}", signature);
    let request = SignInRequest {
        user_data,
        auth: auth_with_prefix,
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app, &request)
        .await;
    assert!(
        result.is_ok(),
        "Should succeed with app-key:signature format"
    );
}

#[tokio::test]
async fn test_verify_signin_authentication_without_prefix() {
    let socket_id = SocketId::new();
    let app = create_test_app();

    let mut mock_app_manager = MockAppManager::new();
    mock_app_manager.expect_find_by_key("test-app-key".to_string(), app.clone());
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({"id": "user-123"}).to_string();
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new("test-app-key".to_string(), "test-app-secret".to_string());
    let signature = token.sign(&string_to_sign);

    let request = SignInRequest {
        user_data,
        auth: signature, // No prefix
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app, &request)
        .await;
    assert!(result.is_ok(), "Should succeed with signature-only format");
}

#[tokio::test]
async fn test_auth_validator_sign_in_token_generation() {
    let app = create_test_app();
    let auth_validator = AuthValidator::new(Arc::new(MockAppManager::new()));
    let socket_id = "123.456";
    let user_data = json!({"id": "user-1", "user_info": {"name": "Alice"}}).to_string();

    let generated_signature =
        auth_validator.sign_in_token_for_user_data(socket_id, &user_data, app.clone());

    // Test that the same inputs produce the same signature
    let second_signature = auth_validator.sign_in_token_for_user_data(socket_id, &user_data, app);
    assert_eq!(
        generated_signature, second_signature,
        "Signatures should be deterministic"
    );
    assert!(
        !generated_signature.is_empty(),
        "Signature should not be empty"
    );
}

#[tokio::test]
async fn test_verify_signin_authentication_with_invalid_signature() {
    let socket_id = SocketId::new();
    let app = create_test_app();

    let mut mock_app_manager = MockAppManager::new();
    mock_app_manager.expect_find_by_key("test-app-key".to_string(), app.clone());
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({"id": "user-123"}).to_string();
    let invalid_signature = "invalid_signature_here";

    let request = SignInRequest {
        user_data,
        auth: format!("test-app-key:{}", invalid_signature),
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app, &request)
        .await;
    assert!(result.is_err(), "Should fail with invalid signature");
}

#[tokio::test]
async fn test_auth_validator_with_different_user_data() {
    let socket_id = SocketId::new();
    let app = create_test_app();

    let mut mock_app_manager = MockAppManager::new();
    mock_app_manager.expect_find_by_key("test-app-key".to_string(), app.clone());
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data1 = json!({"id": "user-1"}).to_string();
    let user_data2 = json!({"id": "user-2"}).to_string();

    // Generate valid signature for user_data1
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data1);
    let token = Token::new("test-app-key".to_string(), "test-app-secret".to_string());
    let signature = token.sign(&string_to_sign);

    // Try to use signature for user_data1 with user_data2 (should fail)
    let request = SignInRequest {
        user_data: user_data2,
        auth: format!("test-app-key:{}", signature),
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app, &request)
        .await;
    assert!(
        result.is_err(),
        "Should fail when user_data doesn't match signature"
    );
}

fn create_test_app() -> App {
    App {
        id: "test-app-id".to_string(),
        key: "test-app-key".to_string(),
        secret: "test-app-secret".to_string(),
        max_connections: 1000,
        enable_client_messages: true,
        enabled: true,
        max_backend_events_per_second: Some(1000),
        max_client_events_per_second: 100,
        max_read_requests_per_second: Some(1000),
        max_presence_members_per_channel: None,
        max_presence_member_size_in_kb: None,
        max_channel_name_length: None,
        max_event_channels_at_once: None,
        max_event_name_length: None,
        max_event_payload_in_kb: None,
        max_event_batch_size: None,
        enable_user_authentication: None,
        webhooks: Some(vec![]),
        enable_watchlist_events: None,
        allowed_origins: None,
    }
}
