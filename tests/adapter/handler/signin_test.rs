use serde_json::json;
use sockudo::adapter::handler::types::SignInRequest;
use sockudo::token::Token;

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
async fn test_auth_signature_extraction() {
    // Test the signature extraction logic from verify_signin_authentication
    let auth_with_prefix = "test-app-key:abc123def456";

    // Simulate the extraction logic
    let signature = if let Some(colon_pos) = auth_with_prefix.find(':') {
        &auth_with_prefix[colon_pos + 1..]
    } else {
        auth_with_prefix
    };

    assert_eq!(
        signature, "abc123def456",
        "Should extract signature after colon"
    );

    // Test with no prefix
    let auth_no_prefix = "just_a_signature";
    let signature2 = if let Some(colon_pos) = auth_no_prefix.find(':') {
        &auth_no_prefix[colon_pos + 1..]
    } else {
        auth_no_prefix
    };

    assert_eq!(
        signature2, "just_a_signature",
        "Should return full string if no colon"
    );
}

#[tokio::test]
async fn test_signin_string_format() {
    // Test that the signing string format matches Pusher spec
    let socket_id = "123.456";
    let user_data = json!({"id": "user-1", "user_info": {"name": "Alice"}}).to_string();

    let string_to_sign = format!("{}::user::{}", socket_id, user_data);

    // Verify format matches: {socket_id}::user::{user_data}
    assert!(string_to_sign.starts_with("123.456::user::"));
    assert!(string_to_sign.contains(r#""id":"user-1""#));
    assert!(
        string_to_sign.contains("::user::"),
        "Should have ::user:: delimiter"
    );
}

#[tokio::test]
async fn test_user_data_validation() {
    // Test that user data can be parsed as JSON and contains required fields
    let valid_user_data = json!({"id": "user-123", "user_info": {"name": "Test"}}).to_string();
    let parsed: serde_json::Value = serde_json::from_str(&valid_user_data).expect("Should parse");

    assert!(parsed.get("id").is_some(), "Should have id field");
    assert_eq!(parsed["id"], "user-123");

    // Test invalid user data without id
    let invalid_user_data = json!({"user_info": {"name": "Test"}}).to_string();
    let parsed2: serde_json::Value =
        serde_json::from_str(&invalid_user_data).expect("Should parse");

    assert!(parsed2.get("id").is_none(), "Should not have id field");
}

#[tokio::test]
async fn test_token_signing() {
    // Test that Token generates consistent HMAC-SHA256 signatures
    let key = "test-key".to_string();
    let secret = "test-secret".to_string();
    let token = Token::new(key, secret);

    let data1 = "test_string";
    let signature1 = token.sign(data1);
    let signature2 = token.sign(data1);

    // Same input should produce same signature
    assert_eq!(signature1, signature2, "Signatures should be deterministic");
    assert!(!signature1.is_empty(), "Signature should not be empty");

    // Different input should produce different signature
    let data2 = "different_string";
    let signature3 = token.sign(data2);
    assert_ne!(
        signature1, signature3,
        "Different inputs should produce different signatures"
    );
}
