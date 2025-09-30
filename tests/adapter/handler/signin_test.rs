use sockudo::adapter::handler::types::SignInRequest;
use sockudo::app::auth::AuthValidator;
use sockudo::app::config::App;
use sockudo::token::Token;
use sockudo::websocket::SocketId;
use sonic_rs::json;
use std::str::FromStr;
use std::sync::Arc;

use crate::mocks::connection_handler_mock::{
    MockAppManager, create_test_connection_handler_with_app_manager,
};

#[tokio::test]
async fn test_signin_request_from_message_json_format() {
    use sockudo::protocol::messages::{MessageData, PusherMessage};

    let user_data = json!({
        "id": "test-user-123",
        "user_info": {
            "name": "Test User",
            "email": "test@example.com"
        }
    });

    let message_data = json!({
        "user_data": user_data.to_string(),
        "auth": "app-key:signature_here"
    });

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Json(message_data)),
        channel: None,
        name: None,
        user_id: None,
    };

    let request = SignInRequest::from_message(&message).unwrap();

    assert_eq!(request.user_data, user_data.to_string());
    assert_eq!(request.auth, "app-key:signature_here");
}

#[tokio::test]
async fn test_signin_request_from_message_structured_format() {
    use sockudo::protocol::messages::{MessageData, PusherMessage};
    use sonic_rs::Value;
    use std::collections::HashMap;

    let user_data = json!({
        "id": "test-user-456",
        "user_info": {
            "name": "Another User"
        }
    })
    .to_string();

    let mut extra = HashMap::new();
    extra.insert(
        "auth".to_string(),
        Value::from_str("app-key:another_signature").unwrap(),
    );

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Structured {
            channel: None,
            channel_data: None,
            user_data: Some(user_data.clone()),
            extra,
        }),
        channel: None,
        name: None,
        user_id: None,
    };

    let request = SignInRequest::from_message(&message).unwrap();

    assert_eq!(request.user_data, user_data);
    assert_eq!(request.auth, "app-key:another_signature");
}

#[tokio::test]
async fn test_signin_request_from_message_missing_user_data_json() {
    use sockudo::protocol::messages::{MessageData, PusherMessage};

    let message_data = json!({
        "auth": "app-key:signature_here"
        // Missing user_data
    });

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Json(message_data)),
        channel: None,
        name: None,
        user_id: None,
    };

    let result = SignInRequest::from_message(&message);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing 'user_data' field")
    );
}

#[tokio::test]
async fn test_signin_request_from_message_missing_auth_structured() {
    use sockudo::protocol::messages::{MessageData, PusherMessage};
    use std::collections::HashMap;

    let user_data = json!({"id": "test-user"}).to_string();
    let extra = HashMap::new(); // Empty extra, missing auth

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Structured {
            channel: None,
            channel_data: None,
            user_data: Some(user_data),
            extra,
        }),
        channel: None,
        name: None,
        user_id: None,
    };

    let result = SignInRequest::from_message(&message);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing 'auth' field")
    );
}

#[tokio::test]
async fn test_signin_request_from_message_invalid_format() {
    use sockudo::protocol::messages::{MessageData, PusherMessage};

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::String("invalid string data".to_string())),
        channel: None,
        name: None,
        user_id: None,
    };

    let result = SignInRequest::from_message(&message);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid signin data format")
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

    let auth_with_prefix = format!("test-app-key:{signature}");
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
        auth: format!("test-app-key:{invalid_signature}"),
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
        auth: format!("test-app-key:{signature}"),
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app, &request)
        .await;
    assert!(
        result.is_err(),
        "Should fail when user_data doesn't match signature"
    );
}

// Integration tests for handle_signin_request function

#[tokio::test]
async fn test_handle_signin_request_parse_and_validate_user_data() {
    // Test the parse_and_validate_user_data method which is called by handle_signin_request
    let mock_app_manager = MockAppManager::new();
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({
        "id": "user-123",
        "user_info": {
            "name": "Test User"
        },
        "watchlist": ["channel1", "channel2"]
    })
    .to_string();

    let result = handler.parse_and_validate_user_data(&user_data);
    assert!(result.is_ok(), "Should parse valid user data");

    let user_info = result.unwrap();
    assert_eq!(user_info.id, "user-123");
    assert!(user_info.watchlist.is_some());
    assert_eq!(user_info.watchlist.unwrap(), vec!["channel1", "channel2"]);
}

#[tokio::test]
async fn test_handle_signin_request_authentication_disabled() {
    let socket_id = SocketId::new();
    let mut app = create_test_app();
    app.enable_user_authentication = Some(false); // Disable signin

    let mock_app_manager = MockAppManager::new();
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let request = SignInRequest {
        user_data: json!({"id": "user-123"}).to_string(),
        auth: "test-app-key:signature".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app, request)
        .await;
    assert!(
        result.is_err(),
        "Should fail when authentication is disabled"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("User authentication is disabled")
    );
}

#[tokio::test]
async fn test_handle_signin_request_invalid_user_data() {
    let socket_id = SocketId::new();
    let mut app = create_test_app();
    app.enable_user_authentication = Some(true);

    let mock_app_manager = MockAppManager::new();
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let request = SignInRequest {
        user_data: "invalid json".to_string(), // Invalid JSON
        auth: "test-app-key:signature".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app, request)
        .await;
    assert!(result.is_err(), "Should fail with invalid JSON user_data");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid user_data JSON")
    );
}

#[tokio::test]
async fn test_handle_signin_request_missing_user_id() {
    let socket_id = SocketId::new();
    let mut app = create_test_app();
    app.enable_user_authentication = Some(true);

    let mock_app_manager = MockAppManager::new();
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({
        "user_info": {
            "name": "Test User"
        }
        // Missing 'id' field
    })
    .to_string();

    let request = SignInRequest {
        user_data,
        auth: "test-app-key:signature".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app, request)
        .await;
    assert!(
        result.is_err(),
        "Should fail when user_data lacks 'id' field"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing 'id' field")
    );
}

#[tokio::test]
async fn test_handle_signin_request_invalid_signature() {
    let socket_id = SocketId::new();
    let mut app = create_test_app();
    app.enable_user_authentication = Some(true);

    let mut mock_app_manager = MockAppManager::new();
    mock_app_manager.expect_find_by_key("test-app-key".to_string(), app.clone());
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let user_data = json!({"id": "user-123"}).to_string();

    let request = SignInRequest {
        user_data,
        auth: "test-app-key:invalid_signature_here".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app, request)
        .await;
    assert!(result.is_err(), "Should fail with invalid signature");
}

#[tokio::test]
async fn test_handle_signin_request_early_validation_flow() {
    // Test the early validation parts of handle_signin_request that don't require connection state
    let socket_id = SocketId::new();
    let mut app = create_test_app();

    // Test 1: Authentication disabled
    app.enable_user_authentication = Some(false);
    let mock_app_manager = MockAppManager::new();
    let handler = create_test_connection_handler_with_app_manager(mock_app_manager);

    let request = SignInRequest {
        user_data: json!({"id": "user-123"}).to_string(),
        auth: "test-app-key:signature".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app, request)
        .await;
    assert!(
        result.is_err(),
        "Should fail when authentication is disabled"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("User authentication is disabled")
    );

    // Test 2: Invalid user data
    app.enable_user_authentication = Some(true);
    let mock_app_manager2 = MockAppManager::new();
    let handler2 = create_test_connection_handler_with_app_manager(mock_app_manager2);

    let invalid_request = SignInRequest {
        user_data: "invalid json".to_string(),
        auth: "test-app-key:signature".to_string(),
    };

    let result2 = handler2
        .handle_signin_request(&socket_id, &app, invalid_request)
        .await;
    assert!(result2.is_err(), "Should fail with invalid JSON");
    assert!(
        result2
            .unwrap_err()
            .to_string()
            .contains("Invalid user_data JSON")
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
