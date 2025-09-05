use serde_json::{Value, json};
use sockudo::protocol::messages::{MessageData, PusherMessage};

// Helper function to serialize message and parse as JSON for testing
fn message_to_json(message: &PusherMessage) -> Value {
    serde_json::to_value(message).expect("Failed to serialize message")
}

#[test]
fn test_connection_established_format() {
    // According to spec: data should be a String (JSON-encoded object)
    let message = PusherMessage::connection_established("test-socket-123".to_string());
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:connection_established");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains valid JSON
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data has correct structure
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert!(
        parsed_data.get("socket_id").is_some(),
        "Should have socket_id field"
    );
    assert!(
        parsed_data["socket_id"].is_string(),
        "socket_id should be a string"
    );
    assert_eq!(parsed_data["socket_id"], "test-socket-123");

    assert!(
        parsed_data.get("activity_timeout").is_some(),
        "Should have activity_timeout field"
    );
    assert!(
        parsed_data["activity_timeout"].is_number(),
        "activity_timeout should be a number"
    );
    assert_eq!(parsed_data["activity_timeout"], 120);
}

#[test]
fn test_error_format() {
    // According to spec: data should be an Object with message and code
    let message = PusherMessage::error(4001, "Application does not exist".to_string(), None);
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:error");

    // Assert data field exists and is an object (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_object(), "Data field should be an Object");

    let data = json["data"].as_object().expect("Data should be an object");

    // Assert data object has correct structure
    assert!(data.get("code").is_some(), "Should have 'code' field");
    assert!(data["code"].is_number(), "Code should be a number");
    assert_eq!(data["code"], 4001);

    assert!(data.get("message").is_some(), "Should have 'message' field");
    assert!(data["message"].is_string(), "Message should be a string");
    assert_eq!(data["message"], "Application does not exist");
}

#[test]
fn test_signin_success_format() {
    // According to spec: data should be an Object with only user_data field
    // We need to check our implementation
    use sockudo::adapter::handler::types::SignInRequest;

    let request = SignInRequest {
        user_data: r#"{"id":"123","name":"John"}"#.to_string(),
        auth: "app_key:signature".to_string(),
    };

    // Use the helper function to ensure consistency with production code
    let message = PusherMessage::signin_success(request.user_data);

    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:signin_success");

    // Assert data field exists and is an object (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_object(), "Data field should be an Object");

    let data = json["data"].as_object().expect("Data should be an object");

    // Assert data object has correct structure
    assert!(
        data.contains_key("user_data"),
        "Should have user_data field"
    );
    assert!(!data.contains_key("auth"), "Should NOT have auth field");
    assert!(
        data["user_data"].is_string(),
        "user_data should be a string"
    );
    assert_eq!(data["user_data"], r#"{"id":"123","name":"John"}"#);
}

#[test]
fn test_ping_format() {
    // According to spec: should have NO data field at all
    let message = PusherMessage::ping();
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:ping");

    // Assert no data field exists (per Pusher spec)
    assert!(
        json.get("data").is_none(),
        "Ping should not have 'data' field"
    );
}

#[test]
fn test_pong_format() {
    // According to spec: should have NO data field at all
    let message = PusherMessage::pong();
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:pong");

    // Assert no data field exists (per Pusher spec)
    assert!(
        json.get("data").is_none(),
        "Pong should not have 'data' field"
    );
}

#[test]
fn test_subscription_succeeded_format() {
    // According to spec: data should be a String (JSON-encoded object)
    // For presence channels, it contains presence data
    use sockudo::protocol::messages::PresenceData;
    use std::collections::HashMap;

    let mut hash = HashMap::new();
    hash.insert("user1".to_string(), Some(json!({"name": "Alice"})));
    hash.insert("user2".to_string(), Some(json!({"name": "Bob"})));

    let presence_data = PresenceData {
        ids: vec!["user1".to_string(), "user2".to_string()],
        hash: hash.clone(),
        count: 2,
    };

    let message =
        PusherMessage::subscription_succeeded("presence-room".to_string(), Some(presence_data));
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher_internal:subscription_succeeded");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "presence-room");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains valid JSON with presence data
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data has correct structure
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert!(
        parsed_data.get("presence").is_some(),
        "Should have 'presence' field"
    );

    // Verify the presence data structure (not double-wrapped)
    let presence = &parsed_data["presence"];
    assert_eq!(presence["count"], 2);
    assert_eq!(presence["ids"], json!(["user1", "user2"]));
    assert_eq!(presence["hash"]["user1"], json!({"name": "Alice"}));
    assert_eq!(presence["hash"]["user2"], json!({"name": "Bob"}));
}

#[test]
fn test_subscription_succeeded_non_presence_format() {
    // For non-presence channels, data should be empty object as string
    let message = PusherMessage::subscription_succeeded("private-channel".to_string(), None);
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher_internal:subscription_succeeded");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "private-channel");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains empty JSON object
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data is an empty object
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert_eq!(parsed_data, json!({}));
}

#[test]
fn test_member_added_format() {
    // According to spec: data should be a String (JSON-encoded object)
    let user_info = json!({"name": "Alice", "email": "alice@example.com"});

    let message = PusherMessage::member_added(
        "presence-room".to_string(),
        "user123".to_string(),
        Some(user_info.clone()),
    );
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher_internal:member_added");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "presence-room");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains valid JSON
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data has correct structure
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert!(
        parsed_data.get("user_id").is_some(),
        "Should have 'user_id' field"
    );
    assert!(
        parsed_data["user_id"].is_string(),
        "user_id should be a string"
    );
    assert_eq!(parsed_data["user_id"], "user123");

    assert!(
        parsed_data.get("user_info").is_some(),
        "Should have 'user_info' field"
    );
    assert_eq!(parsed_data["user_info"], user_info);
}

#[test]
fn test_member_removed_format() {
    // According to spec: data should be a String (JSON-encoded object)
    let message = PusherMessage::member_removed("presence-room".to_string(), "user123".to_string());
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher_internal:member_removed");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "presence-room");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains valid JSON
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data has correct structure
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert!(
        parsed_data.get("user_id").is_some(),
        "Should have 'user_id' field"
    );
    assert!(
        parsed_data["user_id"].is_string(),
        "user_id should be a string"
    );
    assert_eq!(parsed_data["user_id"], "user123");

    assert!(
        parsed_data.get("user_info").is_none(),
        "Should not have user_info"
    );
}

#[test]
fn test_channel_event_format() {
    // According to spec: data should be a String for channel events
    let event_data = json!({"message": "Hello", "timestamp": 1234567890});

    let message = PusherMessage::channel_event("my-event", "my-channel", event_data.clone());
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "my-event");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "my-channel");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_string(), "Data field should be a String");

    // Verify the string contains the JSON data
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data matches expected event data
    assert_eq!(parsed_data, event_data);
}

#[test]
fn test_channel_event_with_user_id() {
    // Test that channel events can include optional user_id field (per Pusher spec)
    let event_data = json!({"message": "Hello from Alice"});

    let message = PusherMessage {
        event: Some("my-event".to_string()),
        channel: Some("presence-room".to_string()),
        data: Some(MessageData::String(event_data.to_string())),
        name: None,
        user_id: Some("user123".to_string()),
    };
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "my-event");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "presence-room");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_string(), "Data field should be a String");

    // Assert user_id field exists and has correct value (per Pusher spec)
    assert!(json.get("user_id").is_some(), "Should have 'user_id' field");
    assert!(json["user_id"].is_string(), "user_id should be a string");
    assert_eq!(json["user_id"], "user123");
}

#[test]
fn test_encrypted_channel_event_format() {
    // According to spec: data should be a String containing JSON with ciphertext and nonce
    // This tests that our format can handle encrypted data properly
    let encrypted_data = json!({
        "ciphertext": "encrypted_content_here",
        "nonce": "random_nonce_value"
    });

    let message = PusherMessage::channel_event(
        "my-event",
        "private-encrypted-channel",
        encrypted_data.clone(),
    );
    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "my-event");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "private-encrypted-channel");

    // Assert data field exists and is a string (per Pusher spec)
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (JSON-encoded)"
    );

    // Verify the string contains the encrypted data JSON
    let data_str = json["data"].as_str().expect("Data should be a string");
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");

    // Assert parsed data has correct encrypted structure
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert!(
        parsed_data.get("ciphertext").is_some(),
        "Should have 'ciphertext' field"
    );
    assert!(
        parsed_data["ciphertext"].is_string(),
        "ciphertext should be a string"
    );
    assert_eq!(parsed_data["ciphertext"], "encrypted_content_here");

    assert!(
        parsed_data.get("nonce").is_some(),
        "Should have 'nonce' field"
    );
    assert!(parsed_data["nonce"].is_string(), "nonce should be a string");
    assert_eq!(parsed_data["nonce"], "random_nonce_value");
}

#[test]
fn test_client_event_accepts_string() {
    // Client events should accept String data
    let message = PusherMessage {
        channel: Some("private-channel".to_string()),
        event: Some("client-typing".to_string()),
        data: Some(MessageData::String("user is typing...".to_string())),
        name: None,
        user_id: None,
    };

    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "client-typing");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "private-channel");

    // Assert data field exists and is a string
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_string(), "Data should be a string");
    assert_eq!(json["data"], "user is typing...");
}

#[test]
fn test_client_event_accepts_json() {
    // Client events should also accept JSON object data
    let message = PusherMessage {
        channel: Some("private-channel".to_string()),
        event: Some("client-typing".to_string()),
        data: Some(MessageData::Json(
            json!({"user": "alice", "status": "typing"}),
        )),
        name: None,
        user_id: None,
    };

    let json = message_to_json(&message);

    // Assert event and channel fields exist and have correct values
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "client-typing");

    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "private-channel");

    // Assert data field exists and is an object
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_object(), "Data should be an object");

    // Assert data object has correct structure
    assert!(
        json["data"].get("user").is_some(),
        "Should have 'user' field in data"
    );
    assert!(json["data"]["user"].is_string(), "User should be a string");
    assert_eq!(json["data"]["user"], "alice");

    assert!(
        json["data"].get("status").is_some(),
        "Should have 'status' field in data"
    );
    assert!(
        json["data"]["status"].is_string(),
        "Status should be a string"
    );
    assert_eq!(json["data"]["status"], "typing");
}

#[test]
fn test_cache_miss_event_format() {
    // According to spec: cache_miss event should send an empty JSON object as a string
    let message = PusherMessage::cache_miss_event("cache-channel".to_string());
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "pusher:cache_miss");

    // Assert channel field exists and has correct value
    assert!(json.get("channel").is_some(), "Should have 'channel' field");
    assert!(
        json["channel"].is_string(),
        "Channel field should be a string"
    );
    assert_eq!(json["channel"], "cache-channel");

    // Assert data field exists and is a string containing empty JSON object
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(
        json["data"].is_string(),
        "Data field should be a String (empty JSON object)"
    );

    // Verify the string is exactly "{}"
    let data_str = json["data"].as_str().expect("Data should be a string");
    assert_eq!(data_str, "{}", "Data should be exactly '{{}}'");

    // Also verify it parses to an empty JSON object
    let parsed_data: Value =
        serde_json::from_str(data_str).expect("Data string should contain valid JSON");
    assert!(parsed_data.is_object(), "Parsed data should be an object");
    assert_eq!(
        parsed_data,
        json!({}),
        "Parsed data should be an empty object"
    );
}

#[test]
fn test_channel_info_minimal_format() {
    // Test channel_info with only occupied field (minimal case)
    let response = PusherMessage::channel_info(false, None, None, None);

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert occupied field exists and is correct type
    assert!(
        response.get("occupied").is_some(),
        "Should have 'occupied' field"
    );
    assert!(
        response["occupied"].is_boolean(),
        "'occupied' should be a boolean"
    );
    assert_eq!(response["occupied"], false);

    // Assert no optional fields are present
    assert!(
        response.get("subscription_count").is_none(),
        "Should not have 'subscription_count' when None"
    );
    assert!(
        response.get("user_count").is_none(),
        "Should not have 'user_count' when None"
    );
    assert!(
        response.get("cache").is_none(),
        "Should not have 'cache' when None"
    );

    // Assert exactly 1 field
    let obj = response.as_object().expect("Response should be an object");
    assert_eq!(obj.len(), 1, "Should have exactly 1 field when minimal");
}

#[test]
fn test_channel_info_with_subscription_count() {
    // Test channel_info with subscription_count
    let response = PusherMessage::channel_info(true, Some(42), None, None);

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert occupied field
    assert!(
        response.get("occupied").is_some(),
        "Should have 'occupied' field"
    );
    assert!(
        response["occupied"].is_boolean(),
        "'occupied' should be a boolean"
    );
    assert_eq!(response["occupied"], true);

    // Assert subscription_count field
    assert!(
        response.get("subscription_count").is_some(),
        "Should have 'subscription_count' field"
    );
    assert!(
        response["subscription_count"].is_u64(),
        "'subscription_count' should be a number"
    );
    assert_eq!(response["subscription_count"], 42);

    // Assert other optional fields are not present
    assert!(
        response.get("user_count").is_none(),
        "Should not have 'user_count' when None"
    );
    assert!(
        response.get("cache").is_none(),
        "Should not have 'cache' when None"
    );
}

#[test]
fn test_channel_info_with_user_count() {
    // Test channel_info with user_count (for presence channels)
    let response = PusherMessage::channel_info(true, Some(10), Some(8), None);

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert all fields
    assert_eq!(response["occupied"], true);
    assert_eq!(response["subscription_count"], 10);
    assert_eq!(response["user_count"], 8);

    // Verify types
    assert!(
        response["occupied"].is_boolean(),
        "'occupied' should be a boolean"
    );
    assert!(
        response["subscription_count"].is_u64(),
        "'subscription_count' should be a number"
    );
    assert!(
        response["user_count"].is_u64(),
        "'user_count' should be a number"
    );

    // Assert cache is not present
    assert!(
        response.get("cache").is_none(),
        "Should not have 'cache' when None"
    );
}

#[test]
fn test_channel_info_with_cache_data() {
    use std::time::Duration;

    // Test channel_info with cache data
    let cache_content = r#"{"event":"test","data":"cached"}"#.to_string();
    let ttl = Duration::from_secs(3600); // 1 hour

    let response =
        PusherMessage::channel_info(true, Some(5), None, Some((cache_content.clone(), ttl)));

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert basic fields
    assert_eq!(response["occupied"], true);
    assert_eq!(response["subscription_count"], 5);

    // Assert cache field exists and has correct structure
    assert!(response.get("cache").is_some(), "Should have 'cache' field");
    assert!(response["cache"].is_object(), "'cache' should be an object");

    // Assert cache subfields
    let cache_obj = response["cache"]
        .as_object()
        .expect("Cache should be an object");
    assert!(
        cache_obj.contains_key("data"),
        "Cache should have 'data' field"
    );
    assert!(
        cache_obj.contains_key("ttl"),
        "Cache should have 'ttl' field"
    );

    // Verify cache data field
    assert!(
        response["cache"]["data"].is_string(),
        "Cache data should be a string"
    );
    assert_eq!(response["cache"]["data"], cache_content);

    // Verify cache ttl field
    assert!(
        response["cache"]["ttl"].is_u64(),
        "Cache ttl should be a number"
    );
    assert_eq!(response["cache"]["ttl"], 3600);

    // Assert user_count is not present
    assert!(
        response.get("user_count").is_none(),
        "Should not have 'user_count' when None"
    );
}

#[test]
fn test_channel_info_full_format() {
    use std::time::Duration;

    // Test channel_info with all fields populated
    let cache_content = r#"{"message":"Hello from cache"}"#.to_string();
    let ttl = Duration::from_secs(7200); // 2 hours

    let response = PusherMessage::channel_info(
        true,
        Some(100),
        Some(75),
        Some((cache_content.clone(), ttl)),
    );

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert all fields are present and have correct values
    assert_eq!(response["occupied"], true);
    assert_eq!(response["subscription_count"], 100);
    assert_eq!(response["user_count"], 75);

    // Assert cache structure
    assert!(response["cache"].is_object(), "'cache' should be an object");
    assert_eq!(response["cache"]["data"], cache_content);
    assert_eq!(response["cache"]["ttl"], 7200);

    // Verify exact field count
    let obj = response.as_object().expect("Response should be an object");
    assert_eq!(
        obj.len(),
        4,
        "Should have exactly 4 fields when all are populated"
    );

    // Verify field types
    assert!(
        response["occupied"].is_boolean(),
        "'occupied' should be a boolean"
    );
    assert!(
        response["subscription_count"].is_u64(),
        "'subscription_count' should be a number"
    );
    assert!(
        response["user_count"].is_u64(),
        "'user_count' should be a number"
    );
    assert!(
        response["cache"]["data"].is_string(),
        "Cache data should be a string"
    );
    assert!(
        response["cache"]["ttl"].is_u64(),
        "Cache ttl should be a number"
    );
}

#[test]
fn test_channel_info_zero_values() {
    // Test that zero values are properly included (not filtered out)
    let response = PusherMessage::channel_info(false, Some(0), Some(0), None);

    // Assert response is a valid JSON object
    assert!(response.is_object(), "Response should be an object");

    // Assert all fields with zero values are present
    assert_eq!(response["occupied"], false);
    assert_eq!(response["subscription_count"], 0);
    assert_eq!(response["user_count"], 0);

    // Verify that zero counts are included (not filtered)
    assert!(
        response.get("subscription_count").is_some(),
        "Should include subscription_count even when 0"
    );
    assert!(
        response.get("user_count").is_some(),
        "Should include user_count even when 0"
    );
}

#[test]
fn test_watchlist_events_format() {
    // Watchlist events are custom Sockudo extensions, not in Pusher spec
    // They can keep their current format
    let message =
        PusherMessage::watchlist_online_event(vec!["user1".to_string(), "user2".to_string()]);
    let json = message_to_json(&message);

    // Assert event field exists and has correct value
    assert!(json.get("event").is_some(), "Should have 'event' field");
    assert!(json["event"].is_string(), "Event field should be a string");
    assert_eq!(json["event"], "online");

    // Assert channel field is omitted for watchlist events (due to skip_serializing_if)
    assert!(
        json.get("channel").is_none(),
        "Channel field should be omitted for watchlist events"
    );

    // Watchlist events can remain as JSON objects since they're custom
    assert!(json.get("data").is_some(), "Should have 'data' field");
    assert!(json["data"].is_object(), "Data should be an object");

    // Assert data object has correct structure
    assert!(
        json["data"].get("user_ids").is_some(),
        "Should have 'user_ids' field in data"
    );
    assert!(
        json["data"]["user_ids"].is_array(),
        "user_ids should be an array"
    );
    assert_eq!(json["data"]["user_ids"], json!(["user1", "user2"]));
}
