// Test for GitHub issues: user authentication and server-to-user channels
use serde_json::{json, Value};
use sockudo::adapter::handler::types::SignInRequest;
use sockudo::app::config::App;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use sockudo::utils::validate_channel_name;

#[tokio::test]
async fn test_server_to_user_channel_validation() {
    let app = App {
        id: "test-app".to_string(),
        key: "test-key".to_string(),
        secret: "test-secret".to_string(),
        max_connections: 100,
        enable_client_messages: true,
        enabled: true,
        max_client_events_per_second: 100,
        max_channel_name_length: Some(200),
        ..Default::default()
    };

    // Test that #server-to-user- channels are now valid
    assert!(
        validate_channel_name(&app, "#server-to-user-userid123")
            .await
            .is_ok(),
        "Server-to-user channels should be valid"
    );

    // Test that other # channels are also allowed
    assert!(
        validate_channel_name(&app, "#internal-channel")
            .await
            .is_ok(),
        "Internal channels with # should be valid"
    );
}

#[test]
fn test_user_data_as_json_string() {
    // Test the traditional format where user_data is a JSON string
    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Json(json!({
            "user_data": r#"{"id":"userid123","name":"John Doe"}"#,
            "auth": "test-app-key:signature"
        }))),
        channel: None,
        name: None,
        user_id: None,
    };

    let request = SignInRequest::from_message(&message);
    assert!(request.is_ok(), "Should parse JSON string user_data");

    let request = request.unwrap();
    assert_eq!(request.auth, "test-app-key:signature");
    assert_eq!(request.user_data, r#"{"id":"userid123","name":"John Doe"}"#);
}

#[test]
fn test_user_data_as_object() {
    // Test the format where user_data is sent as an object (Pusher SDK behavior)
    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Json(json!({
            "user_data": {
                "id": "userid123",
                "name": "John Doe"
            },
            "auth": "test-app-key:signature"
        }))),
        channel: None,
        name: None,
        user_id: None,
    };

    let request = SignInRequest::from_message(&message);
    assert!(request.is_ok(), "Should parse object user_data");

    let request = request.unwrap();
    assert_eq!(request.auth, "test-app-key:signature");

    // Verify the user_data was serialized to JSON string
    let parsed: Value = serde_json::from_str(&request.user_data).unwrap();
    assert_eq!(parsed["id"], "userid123");
    assert_eq!(parsed["name"], "John Doe");
}

#[test]
fn test_structured_message_format() {
    // Test the structured message format
    let mut extra = serde_json::Map::new();
    extra.insert("user_data".to_string(), json!({
        "id": "userid123",
        "user_info": {
            "name": "John Doe"
        }
    }));
    extra.insert("auth".to_string(), json!("test-app-key:signature"));

    let message = PusherMessage {
        event: Some("pusher:signin".to_string()),
        data: Some(MessageData::Structured {
            channel: None,
            extra: Value::Object(extra),
            channel_data: None,
        }),
        channel: None,
        name: None,
        user_id: None,
    };

    let request = SignInRequest::from_message(&message);
    assert!(request.is_ok(), "Should parse structured message format");

    let request = request.unwrap();
    assert_eq!(request.auth, "test-app-key:signature");

    // Verify the user_data was serialized to JSON string
    let parsed: Value = serde_json::from_str(&request.user_data).unwrap();
    assert_eq!(parsed["id"], "userid123");
    assert_eq!(parsed["user_info"]["name"], "John Doe");
}