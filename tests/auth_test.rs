use chrono::Utc;
use sockudo::app::auth::AuthValidator;
use sockudo::app::config::App;
use sockudo::app::manager::AppManager;
use sockudo::app::memory_app_manager::MemoryAppManager;
use sockudo::error::Error;
use sockudo::http_handler::EventQuery;
use sockudo::token::Token;
use sockudo::websocket::SocketId;
use std::collections::BTreeMap;
use std::sync::Arc;

async fn create_test_app_manager() -> Arc<dyn AppManager> {
    let manager = MemoryAppManager::new();
    let app = App {
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
    };
    manager.create_app(app).await.unwrap();
    Arc::new(manager)
}

fn generate_valid_signature(
    app_key: &str,
    app_secret: &str,
    http_method: &str,
    request_path: &str,
    query_params: &BTreeMap<String, String>,
) -> String {
    // Convert keys to lowercase and sort (same as the fixed implementation)
    let mut params_for_signing: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in query_params {
        params_for_signing.insert(key.to_lowercase(), value.clone());
    }

    // Build query string
    let mut sorted_params_kv_pairs: Vec<String> = Vec::new();
    for (key, value) in &params_for_signing {
        sorted_params_kv_pairs.push(format!("{}={}", key, value));
    }
    let query_string = sorted_params_kv_pairs.join("&");

    let string_to_sign = format!(
        "{}\n{}\n{}",
        http_method.to_uppercase(),
        request_path,
        query_string
    );
    let token = Token::new(app_key.to_string(), app_secret.to_string());
    token.sign(&string_to_sign)
}

#[tokio::test]
async fn test_validate_channel_auth_valid() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);
    let socket_id = SocketId::new();

    // Generate a valid signature
    let user_data = "private-channel";
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new("test-app-key".to_string(), "test-app-secret".to_string());
    let valid_auth = token.sign(&string_to_sign);

    let result = auth_validator
        .validate_channel_auth(socket_id, "test-app-key", user_data, &valid_auth)
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_validate_channel_auth_with_app_key_prefix() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);
    let socket_id = SocketId::new();

    // Generate a valid signature with app-key prefix (as sent by real clients)
    let user_data = r#"{"id":"test-user","user_info":{"name":"Test User"}}"#;
    let string_to_sign = format!("{}::user::{}", socket_id.0, user_data);
    let token = Token::new("test-app-key".to_string(), "test-app-secret".to_string());
    let signature = token.sign(&string_to_sign);

    // Client sends auth in format: "app-key:signature"
    let auth_with_prefix = format!("test-app-key:{}", signature);

    // Extract just the signature (simulating what verify_signin_authentication does)
    let extracted_signature = if let Some(colon_pos) = auth_with_prefix.find(':') {
        &auth_with_prefix[colon_pos + 1..]
    } else {
        &auth_with_prefix
    };

    let result = auth_validator
        .validate_channel_auth(socket_id, "test-app-key", user_data, extracted_signature)
        .await;

    assert!(
        result.is_ok(),
        "Authentication should succeed with extracted signature"
    );
    assert!(result.unwrap(), "Signature should be valid");
}

#[tokio::test]
async fn test_validate_channel_auth_invalid_key() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);
    let socket_id = SocketId::new();

    let result = auth_validator
        .validate_channel_auth(socket_id, "invalid-key", "user-data", "invalid-auth")
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::InvalidAppKey => (),
        _ => panic!("Expected InvalidAppKey error"),
    }
}

#[tokio::test]
async fn test_validate_channel_auth_invalid_signature() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);
    let socket_id = SocketId::new();

    let result = auth_validator
        .validate_channel_auth(socket_id, "test-app-key", "user-data", "invalid-signature")
        .await;

    assert!(result.is_ok());
    assert!(!result.unwrap()); // Should return false for invalid signature
}

#[tokio::test]
async fn test_api_auth_valid_signature() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("auth_version".to_string(), "1.0".to_string());

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_case_insensitive_query_keys() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    // Create query params with mixed case keys
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_KEY".to_string(), "test-app-key".to_string()); // Uppercase KEY
    query_params.insert("auth_TIMESTAMP".to_string(), current_timestamp.clone()); // Uppercase TIMESTAMP
    query_params.insert("auth_VERSION".to_string(), "1.0".to_string()); // Uppercase VERSION
    query_params.insert("Some_Mixed_Case_Param".to_string(), "value".to_string());

    // Generate signature with the same mixed-case params
    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(
        result.is_ok(),
        "Mixed case query keys should be handled correctly"
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_case_insensitive_sorting_order() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    // Create params that would sort differently with case-sensitive vs case-insensitive sorting
    let mut query_params = BTreeMap::new();
    query_params.insert("Z_param".to_string(), "z_value".to_string()); // Would come last in case-sensitive sort
    query_params.insert("a_param".to_string(), "a_value".to_string()); // Would come first in case-insensitive sort
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("auth_version".to_string(), "1.0".to_string());
    query_params.insert("B_param".to_string(), "b_value".to_string()); // Would interfere with correct sorting

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(
        result.is_ok(),
        "Case-insensitive sorting should work correctly"
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_expired_timestamp() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let expired_timestamp = (Utc::now().timestamp() - 700).to_string(); // 700 seconds ago (> 600 limit)
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), expired_timestamp.clone());

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: expired_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: "any-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("Timestamp expired") || msg.contains("too far in the future"));
        }
        _ => panic!("Expected Auth error for expired timestamp"),
    }
}

#[tokio::test]
async fn test_api_auth_invalid_signature() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: "invalid-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("Invalid API signature"));
        }
        _ => panic!("Expected Auth error for invalid signature"),
    }
}

#[tokio::test]
async fn test_api_auth_post_with_body_md5() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let body = b"test body content";
    let body_md5 = format!("{:x}", md5::compute(body));

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), body_md5.clone());

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "POST",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5,
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "POST",
            "/apps/test-app-id/events",
            &query_params,
            Some(body),
        )
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_post_with_wrong_body_md5() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let body = b"test body content";
    let wrong_body_md5 = "wrong_md5_hash";

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), wrong_body_md5.to_string());

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: wrong_body_md5.to_string(),
        auth_signature: "any-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "POST",
            "/apps/test-app-id/events",
            &query_params,
            Some(body),
        )
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("body_md5 mismatch"));
        }
        _ => panic!("Expected Auth error for body_md5 mismatch"),
    }
}

#[tokio::test]
async fn test_sign_in_token_generation() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let socket_id = "12345.67890";
    let user_data = "test-user-data";
    let app_config = App {
        id: "test-app-id".to_string(),
        key: "test-key".to_string(),
        secret: "test-secret".to_string(),
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
    };

    let signature =
        auth_validator.sign_in_token_for_user_data(socket_id, user_data, app_config.clone());

    // Verify the signature is valid
    let is_valid =
        auth_validator.sign_in_token_is_valid(socket_id, user_data, &signature, app_config.clone());
    assert!(is_valid);

    // Verify invalid signature fails
    let is_invalid =
        auth_validator.sign_in_token_is_valid(socket_id, user_data, "wrong-signature", app_config);
    assert!(!is_invalid);
}

#[tokio::test]
async fn test_api_auth_empty_parameter_values() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    // Test with empty parameter values
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("empty_param".to_string(), "".to_string()); // Empty value
    query_params.insert("another_empty".to_string(), "".to_string()); // Another empty value

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(
        result.is_ok(),
        "Empty parameter values should be handled correctly"
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_special_characters_in_keys() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    // Test with special characters in parameter keys (already URL decoded)
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("param_with-dash".to_string(), "value1".to_string());
    query_params.insert("param.with.dots".to_string(), "value2".to_string());
    query_params.insert("param_with_underscores".to_string(), "value3".to_string());

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(
        result.is_ok(),
        "Special characters in parameter keys should be handled correctly"
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_large_number_of_parameters() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    // Test with a large number of parameters to ensure no performance regression
    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());

    // Add 100 parameters with mixed case keys
    for i in 0..100 {
        let key = if i % 2 == 0 {
            format!("PARAM_{}", i) // Uppercase
        } else {
            format!("param_{}", i) // Lowercase
        };
        query_params.insert(key, format!("value_{}", i));
    }

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "GET",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "".to_string(),
        auth_signature: signature,
    };

    let start = std::time::Instant::now();
    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;
    let duration = start.elapsed();

    assert!(
        result.is_ok(),
        "Large number of parameters should be handled correctly"
    );
    assert!(result.unwrap());
    assert!(
        duration.as_millis() < 100,
        "Performance should remain reasonable with many parameters"
    );
}

#[tokio::test]
async fn test_api_auth_post_empty_body_with_body_md5() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let empty_body = b"";
    let empty_body_md5 = "d41d8cd98f00b204e9800998ecf8427e"; // MD5 of empty bytes

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), empty_body_md5.to_string()); // Include body_md5 for empty body

    let signature = generate_valid_signature(
        "test-app-key",
        "test-app-secret",
        "POST",
        "/apps/test-app-id/events",
        &query_params,
    );

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: empty_body_md5.to_string(),
        auth_signature: signature,
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "POST",
            "/apps/test-app-id/events",
            &query_params,
            Some(empty_body),
        )
        .await;

    assert!(
        result.is_ok(),
        "POST with empty body and correct body_md5 should be valid"
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_api_auth_post_empty_body_with_wrong_body_md5_should_fail() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();
    let empty_body = b"";
    let wrong_body_md5 = "wrong_hash_value"; // Wrong MD5 for empty body

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), wrong_body_md5.to_string());

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: wrong_body_md5.to_string(),
        auth_signature: "any-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "POST",
            "/apps/test-app-id/events",
            &query_params,
            Some(empty_body),
        )
        .await;

    assert!(
        result.is_err(),
        "POST with empty body and wrong body_md5 should fail"
    );
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("body_md5 mismatch"));
        }
        _ => panic!("Expected Auth error for body_md5 mismatch"),
    }
}

#[tokio::test]
async fn test_api_auth_get_with_body_md5_should_fail() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), "some_hash".to_string()); // Should not be present for GET

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "some_hash".to_string(),
        auth_signature: "any-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "GET",
            "/apps/test-app-id/events",
            &query_params,
            None,
        )
        .await;

    assert!(result.is_err(), "GET requests should not contain body_md5");
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("body_md5 must not be present"));
        }
        _ => panic!("Expected Auth error for body_md5 with GET request"),
    }
}

#[tokio::test]
async fn test_api_auth_post_no_body_with_body_md5_should_fail() {
    let app_manager = create_test_app_manager().await;
    let auth_validator = AuthValidator::new(app_manager);

    let current_timestamp = Utc::now().timestamp().to_string();

    let mut query_params = BTreeMap::new();
    query_params.insert("auth_key".to_string(), "test-app-key".to_string());
    query_params.insert("auth_timestamp".to_string(), current_timestamp.clone());
    query_params.insert("body_md5".to_string(), "some_hash".to_string()); // Should not be present when no body

    let auth_query = EventQuery {
        auth_key: "test-app-key".to_string(),
        auth_timestamp: current_timestamp,
        auth_version: "1.0".to_string(),
        body_md5: "some_hash".to_string(),
        auth_signature: "any-signature".to_string(),
    };

    let result = auth_validator
        .validate_pusher_api_request(
            &auth_query,
            "POST",
            "/apps/test-app-id/events",
            &query_params,
            None, // No body provided
        )
        .await;

    assert!(
        result.is_err(),
        "POST with no body should not contain body_md5"
    );
    match result.unwrap_err() {
        Error::Auth(msg) => {
            assert!(msg.contains("body_md5 must not be present"));
        }
        _ => panic!("Expected Auth error for body_md5 with no body"),
    }
}
