#[cfg(test)]
mod client_event_validation_tests {
    use crate::mocks::connection_handler_mock::create_test_connection_handler;
    use sockudo::adapter::handler::types::ClientEventRequest;
    use sockudo::app::config::App;
    use sonic_rs::json;

    fn setup_test_app() -> App {
        App {
            id: "test-app".to_string(),
            key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            enabled: true,
            enable_client_messages: true,
            max_event_name_length: Some(200),
            max_channel_name_length: Some(200),
            max_event_payload_in_kb: Some(10),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_client_event_must_start_with_client_prefix() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "my-event".to_string(), // Missing 'client-' prefix
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Client events must start with 'client-'")
        );
    }

    #[tokio::test]
    async fn test_client_event_cannot_use_pusher_prefix() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "pusher:fake-event".to_string(), // Reserved prefix
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Client events cannot use reserved prefixes")
        );
    }

    #[tokio::test]
    async fn test_client_event_cannot_use_pusher_internal_prefix() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "pusher_internal:fake-event".to_string(), // Reserved prefix
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Client events cannot use reserved prefixes")
        );
    }

    #[tokio::test]
    async fn test_valid_client_event_passes_validation() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "client-typing".to_string(), // Valid client event
            channel: "private-channel".to_string(),
            data: json!({"user": "alice", "status": "typing"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_events_disabled_for_app() {
        let (handler, _app_manager) = create_test_connection_handler();
        let mut app = setup_test_app();
        app.enable_client_messages = false;

        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("Client events are not enabled"));
    }

    #[tokio::test]
    async fn test_client_event_rejected_on_public_channel() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "public-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Client events can only be sent to private or presence channels")
        );
    }

    #[tokio::test]
    async fn test_client_event_allowed_on_private_channel() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_event_allowed_on_presence_channel() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "presence-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_event_name_length_limit() {
        let (handler, _app_manager) = create_test_connection_handler();
        let mut app = setup_test_app();
        app.max_event_name_length = Some(20);

        let request = ClientEventRequest {
            event: "client-this-is-a-very-long-event-name-that-exceeds-limit".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[tokio::test]
    async fn test_client_event_payload_size_limit() {
        let (handler, _app_manager) = create_test_connection_handler();
        let mut app = setup_test_app();
        app.max_event_payload_in_kb = Some(1); // 1KB limit

        // Create a large payload over 1KB
        let large_data = "x".repeat(2000); // 2000 bytes > 1KB
        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": large_data}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds limit"));
    }

    #[tokio::test]
    async fn test_client_event_channel_name_length_limit() {
        let (handler, _app_manager) = create_test_connection_handler();
        let mut app = setup_test_app();
        app.max_channel_name_length = Some(20);

        let request = ClientEventRequest {
            event: "client-typing".to_string(),
            channel: "private-this-is-a-very-long-channel-name-that-exceeds-limit".to_string(),
            data: json!({"message": "test"}),
        };

        let result = handler.validate_client_event(&app, &request).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum length"));
    }

    #[tokio::test]
    async fn test_client_event_with_special_prefixes_variations() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        // Test variations of reserved prefixes that should be blocked
        let reserved_prefixes = vec![
            "pusher:",
            "pusher:ping",
            "pusher:pong",
            "pusher:error",
            "pusher:connection_established",
            "pusher:subscribe",
            "pusher:unsubscribe",
            "pusher:cache_miss",
            "pusher:signin_success",
            "pusher_internal:",
            "pusher_internal:subscription_succeeded",
            "pusher_internal:member_added",
            "pusher_internal:member_removed",
            "pusher_internal:subscription_count",
        ];

        for event_name in reserved_prefixes {
            let request = ClientEventRequest {
                event: event_name.to_string(),
                channel: "private-channel".to_string(),
                data: json!({"message": "test"}),
            };

            let result = handler.validate_client_event(&app, &request).await;
            assert!(
                result.is_err(),
                "Event '{}' should have been rejected but wasn't",
                event_name
            );

            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("reserved prefix"),
                "Error message for '{}' should mention reserved prefix",
                event_name
            );
        }
    }

    #[tokio::test]
    async fn test_client_event_validation_edge_cases() {
        let (handler, _app_manager) = create_test_connection_handler();
        let app = setup_test_app();

        // Test event that starts with 'client-pusher:' (should pass after prefix check)
        let request = ClientEventRequest {
            event: "client-pusher:custom".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };
        assert!(handler.validate_client_event(&app, &request).await.is_ok());

        // Test event that starts with 'client-pusher_internal:' (should pass after prefix check)
        let request = ClientEventRequest {
            event: "client-pusher_internal:custom".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };
        assert!(handler.validate_client_event(&app, &request).await.is_ok());

        // Test empty event name
        let request = ClientEventRequest {
            event: "".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };
        assert!(handler.validate_client_event(&app, &request).await.is_err());

        // Test with only 'client-' prefix (should pass)
        let request = ClientEventRequest {
            event: "client-".to_string(),
            channel: "private-channel".to_string(),
            data: json!({"message": "test"}),
        };
        assert!(handler.validate_client_event(&app, &request).await.is_ok());
    }
}
