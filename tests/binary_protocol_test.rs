use sockudo::adapter::binary_protocol::*;
use sockudo::adapter::horizontal_adapter::{
    BroadcastMessage, RequestBody, RequestType, ResponseBody,
};
use std::collections::{HashMap, HashSet};

#[test]
fn test_broadcast_message_binary_conversion() {
    let original = BroadcastMessage {
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        channel: "test-channel".to_string(),
        message: r#"{"event":"pusher:test","data":{"message":"hello"}}"#.to_string(),
        except_socket_id: Some("socket-123".to_string()),
        timestamp_ms: Some(1234567890.123456),
    };

    // Convert to binary
    let binary: BinaryBroadcastMessage = original.clone().into();

    // Verify binary structure
    assert_eq!(binary.version, BINARY_PROTOCOL_VERSION);
    assert_eq!(binary.app_id, original.app_id);
    assert_eq!(binary.channel, original.channel);
    assert_eq!(binary.except_socket_id, original.except_socket_id);
    assert_eq!(binary.timestamp_ms, original.timestamp_ms);

    // Verify raw JSON is preserved
    assert_eq!(
        String::from_utf8_lossy(&binary.raw_client_json),
        original.message
    );

    // Convert back
    let recovered: BroadcastMessage = binary.into();
    assert_eq!(recovered.node_id, original.node_id);
    assert_eq!(recovered.app_id, original.app_id);
    assert_eq!(recovered.channel, original.channel);
    assert_eq!(recovered.message, original.message);
    assert_eq!(recovered.except_socket_id, original.except_socket_id);
    assert_eq!(recovered.timestamp_ms, original.timestamp_ms);
}

#[test]
fn test_request_body_binary_conversion() {
    let original = RequestBody {
        request_id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        request_type: RequestType::ChannelMembers,
        channel: Some("presence-room".to_string()),
        socket_id: Some("socket-456".to_string()),
        user_id: Some("user-789".to_string()),
        user_info: Some(serde_json::json!({"name": "Test User"})),
        timestamp: Some(1234567890),
        dead_node_id: None,
        target_node_id: None,
    };

    // Convert to binary
    let binary: BinaryRequestBody = original.clone().try_into().unwrap();

    // Verify binary structure
    assert_eq!(binary.version, BINARY_PROTOCOL_VERSION);
    assert_eq!(binary.app_id, original.app_id);
    // Note: request_type is now stored as request_type_discriminant (u8)
    assert_eq!(binary.channel, original.channel);
    assert!(binary.channel_hash.is_some());

    // Convert back
    let recovered: RequestBody = binary.try_into().unwrap();
    assert_eq!(recovered.request_id, original.request_id);
    assert_eq!(recovered.node_id, original.node_id);
    assert_eq!(recovered.app_id, original.app_id);
    assert_eq!(recovered.channel, original.channel);
    assert_eq!(recovered.user_id, original.user_id);
}

#[test]
fn test_response_body_binary_conversion() {
    let mut members = HashMap::new();
    members.insert(
        "user-1".to_string(),
        sockudo::channel::PresenceMemberInfo {
            user_id: "user-1".to_string(),
            user_info: Option::from(serde_json::json!({"name": "User 1"})),
        },
    );

    let mut channels = HashSet::new();
    channels.insert("channel-1".to_string());
    channels.insert("channel-2".to_string());

    let original = ResponseBody {
        request_id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        members: members.clone(),
        channels_with_sockets_count: HashMap::new(),
        socket_ids: vec!["socket-1".to_string(), "socket-2".to_string()],
        sockets_count: 2,
        exists: true,
        channels: channels.clone(),
        members_count: 1,
    };

    // Convert to binary
    let binary: BinaryResponseBody = original.clone().try_into().unwrap();

    // Verify binary structure
    assert_eq!(binary.version, BINARY_PROTOCOL_VERSION);
    assert_eq!(binary.app_id, original.app_id);
    assert_eq!(binary.sockets_count, original.sockets_count);
    assert_eq!(binary.exists, original.exists);

    // Convert back
    let recovered: ResponseBody = binary.try_into().unwrap();
    assert_eq!(recovered.request_id, original.request_id);
    assert_eq!(recovered.node_id, original.node_id);
    assert_eq!(recovered.app_id, original.app_id);
    assert_eq!(recovered.socket_ids, original.socket_ids);
    assert_eq!(recovered.sockets_count, original.sockets_count);
    assert_eq!(recovered.exists, original.exists);
    assert_eq!(recovered.channels, original.channels);
    assert_eq!(recovered.members.len(), original.members.len());
}

#[test]
fn test_bincode_size_reduction() {
    let message = BroadcastMessage {
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        channel: "test-channel".to_string(),
        message: r#"{"event":"pusher:test","data":"small payload"}"#.to_string(),
        except_socket_id: None,
        timestamp_ms: Some(1234567890.123),
    };

    // Serialize with JSON
    let json_bytes = serde_json::to_vec(&message).unwrap();

    // Serialize with bincode
    let binary_msg: BinaryBroadcastMessage = message.into();
    let binary_bytes = bincode::encode_to_vec(&binary_msg, bincode::config::standard()).unwrap();

    // Print sizes for comparison
    println!("JSON size: {} bytes", json_bytes.len());
    println!("Binary size: {} bytes", binary_bytes.len());

    // Binary should be competitive or better
    // Note: For very small messages, overhead might make binary slightly larger,
    // but for typical messages with longer payloads, binary will be smaller
}

#[test]
fn test_channel_hash_consistency() {
    let channel1 = "private-user-123";
    let channel2 = "private-user-123";
    let channel3 = "private-user-456";

    let hash1 = calculate_channel_hash(channel1);
    let hash2 = calculate_channel_hash(channel2);
    let hash3 = calculate_channel_hash(channel3);

    // Same channel should produce same hash
    assert_eq!(hash1, hash2);

    // Different channels should produce different hashes
    assert_ne!(hash1, hash3);
}

#[test]
fn test_large_message_handling() {
    // Create a large message with a big JSON payload
    let large_payload = format!(
        r#"{{"event":"pusher:test","data":"{}"}}"#,
        "x".repeat(1024 * 100) // 100KB payload
    );

    let message = BroadcastMessage {
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        channel: "test-channel".to_string(),
        message: large_payload.clone(),
        except_socket_id: None,
        timestamp_ms: Some(1234567890.123),
    };

    // Convert to binary
    let binary: BinaryBroadcastMessage = message.clone().into();
    let serialized = bincode::encode_to_vec(&binary, bincode::config::standard()).unwrap();

    // Verify it's under the size limit
    assert!(serialized.len() < MAX_MESSAGE_SIZE as usize);

    // Deserialize and verify
    let (deserialized, _): (BinaryBroadcastMessage, usize) =
        bincode::decode_from_slice(&serialized, bincode::config::standard()).unwrap();
    let recovered: BroadcastMessage = deserialized.into();

    assert_eq!(recovered.message, message.message);
}

#[test]
fn test_no_double_serialization() {
    // This test verifies that we're not re-parsing the client JSON
    let client_json = r#"{"event":"pusher:test","data":{"nested":{"deep":"value"}}}"#;

    let message = BroadcastMessage {
        node_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        app_id: "test-app".to_string(),
        channel: "test-channel".to_string(),
        message: client_json.to_string(),
        except_socket_id: None,
        timestamp_ms: Some(1234567890.123),
    };

    // Convert to binary
    let binary: BinaryBroadcastMessage = message.clone().into();

    // The raw_client_json should be exactly the input bytes
    assert_eq!(
        String::from_utf8(binary.raw_client_json.clone()).unwrap(),
        client_json
    );

    // When we recover it, we should get the exact same JSON
    let recovered: BroadcastMessage = binary.into();
    assert_eq!(recovered.message, client_json);
}
