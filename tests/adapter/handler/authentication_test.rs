use crate::mocks::connection_handler_mock::create_test_connection_handler;
use sockudo::adapter::handler::types::SubscriptionRequest;
use sockudo::app::config::App;
use sockudo::error::Error;
use sockudo::protocol::messages::{MessageData, PusherMessage};
use sockudo::websocket::SocketId;
use sonic_rs::json;

#[tokio::test]
async fn test_verify_channel_authentication_public_channel() {
    let (handler, _app_manager) = create_test_connection_handler();
    let app_config = App::default();
    let socket_id = SocketId::new();

    let request = SubscriptionRequest {
        channel: "public-channel".to_string(),
        auth: None,
        channel_data: None,
    };

    let result = handler
        .verify_channel_authentication(&app_config, &socket_id, &request)
        .await;
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_verify_channel_authentication_private_channel_no_auth() {
    let (handler, _app_manager) = create_test_connection_handler();
    let app_config = App::default();
    let socket_id = SocketId::new();

    let request = SubscriptionRequest {
        channel: "private-channel".to_string(),
        auth: None,
        channel_data: None,
    };

    let result = handler
        .verify_channel_authentication(&app_config, &socket_id, &request)
        .await;
    assert!(result.is_err());
    match result {
        Err(Error::Auth(_)) => (),
        _ => panic!("Expected AuthError"),
    }
}

#[tokio::test]
async fn test_verify_channel_authentication_private_channel_with_auth() {
    let (handler, _app_manager) = create_test_connection_handler();
    let app_config = App::default();
    let socket_id = SocketId::new();
    let channel = "private-channel".to_string();
    let auth = "test-signature".to_string();

    let _expected_message = PusherMessage {
        channel: Some(channel.clone()),
        event: Some("pusher:subscribe".to_string()),
        data: Some(MessageData::Json(json!({
            "channel": channel,
            "auth": auth.clone(),
            "channel_data": null
        }))),
        name: None,
        user_id: None,
    };

    let request = SubscriptionRequest {
        channel,
        auth: Some(auth),
        channel_data: None,
    };

    let result = handler
        .verify_channel_authentication(&app_config, &socket_id, &request)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_verify_channel_authentication_presence_channel() {
    let (handler, _app_manager) = create_test_connection_handler();
    let app_config = App::default();
    let socket_id = SocketId::new();
    let channel = "presence-channel".to_string();
    let auth = "test-signature".to_string();
    let channel_data = json!({
        "user_id": "123",
        "user_info": {
            "name": "Test User"
        }
    })
    .to_string();

    let _expected_message = PusherMessage {
        channel: Some(channel.clone()),
        event: Some("pusher:subscribe".to_string()),
        data: Some(MessageData::Json(json!({
            "channel": channel,
            "auth": auth.clone(),
            "channel_data": channel_data
        }))),
        name: None,
        user_id: None,
    };

    let request = SubscriptionRequest {
        channel,
        auth: Some(auth),
        channel_data: Some(channel_data),
    };

    let result = handler
        .verify_channel_authentication(&app_config, &socket_id, &request)
        .await;
    assert!(result.is_ok());
}
