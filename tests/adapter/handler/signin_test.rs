use crate::mocks::connection_handler_mock::create_test_connection_handler;
use serde_json::json;
use sockudo::adapter::handler::types::SignInRequest;
use sockudo::app::config::App;
use sockudo::error::Error;
use sockudo::token::Token;
use sockudo::websocket::SocketId;

#[tokio::test]
async fn test_signin_with_app_key_prefix_format() {
    let (handler, _app_manager) = create_test_connection_handler();
    let mut app_config = App::default();
    app_config.enable_user_authentication = Some(true);
    let socket_id = SocketId::new();

    // Generate proper user data
    let user_data = json!({
        "id": "test-user-123",
        "user_info": {
            "name": "Test User",
            "email": "test@example.com"
        }
    })
    .to_string();

    // Generate signature the same way a real client would
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new(app_config.key.clone(), app_config.secret.clone());
    let signature = token.sign(&string_to_sign);

    // Client sends auth in format: "app-key:signature" (Pusher spec)
    let auth_with_prefix = format!("{}:{}", app_config.key, signature);

    let request = SignInRequest {
        user_data: user_data.clone(),
        auth: auth_with_prefix,
    };

    // This should succeed with the updated verify_signin_authentication logic
    let result = handler
        .verify_signin_authentication(&socket_id, &app_config, &request)
        .await;

    assert!(result.is_ok(), "Sign-in with app-key:signature format should succeed");
}

#[tokio::test]
async fn test_signin_disabled_for_app() {
    let (handler, _app_manager) = create_test_connection_handler();
    let mut app_config = App::default();
    app_config.enable_user_authentication = Some(false); // Disabled
    let socket_id = SocketId::new();

    let user_data = json!({"id": "test-user"}).to_string();
    let request = SignInRequest {
        user_data,
        auth: "app-key:signature".to_string(),
    };

    let result = handler
        .handle_signin_request(&socket_id, &app_config, request)
        .await;

    assert!(result.is_err(), "Sign-in should fail when disabled");
    match result {
        Err(Error::Auth(msg)) => {
            assert!(msg.contains("User authentication is disabled"));
        }
        _ => panic!("Expected Auth error about disabled authentication"),
    }
}

#[tokio::test]
async fn test_signin_invalid_signature() {
    let (handler, _app_manager) = create_test_connection_handler();
    let mut app_config = App::default();
    app_config.enable_user_authentication = Some(true);
    let socket_id = SocketId::new();

    let user_data = json!({
        "id": "test-user",
        "user_info": {"name": "Test"}
    })
    .to_string();

    // Use an invalid signature
    let request = SignInRequest {
        user_data,
        auth: "app-key:invalid_signature_here".to_string(),
    };

    let result = handler
        .verify_signin_authentication(&socket_id, &app_config, &request)
        .await;

    assert!(result.is_err(), "Sign-in should fail with invalid signature");
    match result {
        Err(Error::Auth(msg)) => {
            assert!(msg.contains("not authorized"));
        }
        _ => panic!("Expected Auth error for invalid signature"),
    }
}

#[tokio::test]
async fn test_signin_missing_user_id() {
    let (handler, _app_manager) = create_test_connection_handler();
    let mut app_config = App::default();
    app_config.enable_user_authentication = Some(true);
    let socket_id = SocketId::new();

    // User data without 'id' field
    let user_data = json!({
        "user_info": {"name": "Test"}
    })
    .to_string();

    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new(app_config.key.clone(), app_config.secret.clone());
    let signature = token.sign(&string_to_sign);
    let auth = format!("{}:{}", app_config.key, signature);

    let request = SignInRequest {
        user_data,
        auth,
    };

    let result = handler.handle_signin_request(&socket_id, &app_config, request).await;

    assert!(result.is_err(), "Sign-in should fail without user id");
}

#[tokio::test]
async fn test_signin_without_app_key_prefix() {
    let (handler, _app_manager) = create_test_connection_handler();
    let mut app_config = App::default();
    app_config.enable_user_authentication = Some(true);
    let socket_id = SocketId::new();

    let user_data = json!({
        "id": "test-user",
        "user_info": {"name": "Test"}
    })
    .to_string();

    // Generate signature and pass it without app-key prefix
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new(app_config.key.clone(), app_config.secret.clone());
    let signature = token.sign(&string_to_sign);

    let request = SignInRequest {
        user_data,
        auth: signature, // Just the signature, no prefix
    };

    // Should still work because verify_signin_authentication handles both formats
    let result = handler
        .verify_signin_authentication(&socket_id, &app_config, &request)
        .await;

    assert!(result.is_ok(), "Sign-in should work with just signature (backward compat)");
}