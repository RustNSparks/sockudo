use sockudo::protocol::messages::PusherMessage;

#[test]
fn test_subscribe_message_parsing() {
    // Test case 1: Simple subscription
    let json1 = r#"{"event":"pusher:subscribe","data":{"channel":"test-channel"}}"#;
    let result1 = serde_json::from_str::<PusherMessage>(json1);
    assert!(
        result1.is_ok(),
        "Failed to parse simple subscription: {:?}",
        result1.err()
    );

    // Test case 2: Subscription with auth
    let json2 = r#"{"event":"pusher:subscribe","data":{"channel":"private-channel","auth":"app-key:signature"}}"#;
    let result2 = serde_json::from_str::<PusherMessage>(json2);
    assert!(
        result2.is_ok(),
        "Failed to parse subscription with auth: {:?}",
        result2.err()
    );

    // Test case 3: Subscription with auth and channel_data
    let json3 = r#"{"event":"pusher:subscribe","data":{"channel":"presence-channel","auth":"app-key:signature","channel_data":"{\"user_id\":\"123\"}"}}"#;
    let result3 = serde_json::from_str::<PusherMessage>(json3);
    assert!(
        result3.is_ok(),
        "Failed to parse subscription with channel_data: {:?}",
        result3.err()
    );

    // Test case 4: Long auth string (similar to error message)
    let json4 = r#"{"event":"pusher:subscribe","data":{"channel":"private-channel","auth":"app-key:very-long-hmac-signature-string-to-reach-approximately-column-146-position-with-secret"}}"#;
    let result4 = serde_json::from_str::<PusherMessage>(json4);
    assert!(
        result4.is_ok(),
        "Failed to parse subscription with long auth: {:?}",
        result4.err()
    );
}

#[test]
fn test_message_data_variants() {
    // Test string data
    let json1 = r#"{"event":"test","data":"simple string"}"#;
    let result1 = serde_json::from_str::<PusherMessage>(json1);
    assert!(
        result1.is_ok(),
        "Failed to parse string data: {:?}",
        result1.err()
    );

    // Test JSON object data
    let json2 = r#"{"event":"test","data":{"key":"value"}}"#;
    let result2 = serde_json::from_str::<PusherMessage>(json2);
    assert!(
        result2.is_ok(),
        "Failed to parse JSON object data: {:?}",
        result2.err()
    );

    // Test nested JSON data
    let json3 = r#"{"event":"test","data":{"nested":{"deep":"value"}}}"#;
    let result3 = serde_json::from_str::<PusherMessage>(json3);
    assert!(
        result3.is_ok(),
        "Failed to parse nested JSON data: {:?}",
        result3.err()
    );
}
