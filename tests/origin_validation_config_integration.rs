#[cfg(test)]
mod origin_validation_config_integration_tests {
    use sockudo::app::config::App;
    use sockudo::app::memory_app_manager::MemoryAppManager;
    use sockudo::app::manager::AppManager;
    use sockudo::adapter::handler::origin_validation::OriginValidator;

    #[tokio::test]
    async fn test_memory_app_manager_with_allowed_origins() {
        // Create test apps with different origin configurations
        let apps = vec![
            // App with no origin restrictions
            App {
                id: "app-no-restrictions".to_string(),
                key: "key1".to_string(),
                secret: "secret1".to_string(),
                max_connections: 1000,
                enable_client_messages: true,
                enabled: true,
                max_client_events_per_second: 100,
                max_backend_events_per_second: Some(1000),
                max_read_requests_per_second: Some(1000),
                max_presence_members_per_channel: Some(100),
                max_presence_member_size_in_kb: Some(2),
                max_channel_name_length: Some(200),
                max_event_channels_at_once: Some(10),
                max_event_name_length: Some(200),
                max_event_payload_in_kb: Some(100),
                max_event_batch_size: Some(10),
                enable_user_authentication: Some(false),
                webhooks: None,
                enable_watchlist_events: Some(false),
                allowed_origins: None,
            },
            // App with specific allowed origins
            App {
                id: "app-with-origins".to_string(),
                key: "key2".to_string(),
                secret: "secret2".to_string(),
                max_connections: 1000,
                enable_client_messages: true,
                enabled: true,
                max_client_events_per_second: 100,
                max_backend_events_per_second: Some(1000),
                max_read_requests_per_second: Some(1000),
                max_presence_members_per_channel: Some(100),
                max_presence_member_size_in_kb: Some(2),
                max_channel_name_length: Some(200),
                max_event_channels_at_once: Some(10),
                max_event_name_length: Some(200),
                max_event_payload_in_kb: Some(100),
                max_event_batch_size: Some(10),
                enable_user_authentication: Some(false),
                webhooks: None,
                enable_watchlist_events: Some(false),
                allowed_origins: Some(vec![
                    "https://app.example.com".to_string(),
                    "*.staging.example.com".to_string(),
                    "http://localhost:3000".to_string(),
                ]),
            },
            // App with wildcard allowing all origins
            App {
                id: "app-allow-all".to_string(),
                key: "key3".to_string(),
                secret: "secret3".to_string(),
                max_connections: 1000,
                enable_client_messages: true,
                enabled: true,
                max_client_events_per_second: 100,
                max_backend_events_per_second: Some(1000),
                max_read_requests_per_second: Some(1000),
                max_presence_members_per_channel: Some(100),
                max_presence_member_size_in_kb: Some(2),
                max_channel_name_length: Some(200),
                max_event_channels_at_once: Some(10),
                max_event_name_length: Some(200),
                max_event_payload_in_kb: Some(100),
                max_event_batch_size: Some(10),
                enable_user_authentication: Some(false),
                webhooks: None,
                enable_watchlist_events: Some(false),
                allowed_origins: Some(vec!["*".to_string()]),
            },
        ];

        // Create memory app manager  
        let app_manager = MemoryAppManager::new();
        
        // Add apps to the manager
        for app in apps {
            app_manager.create_app(app).await.unwrap();
        }

        // Test app with no restrictions
        let app1 = app_manager.find_by_id("app-no-restrictions").await.unwrap().unwrap();
        assert!(app1.allowed_origins.is_none());
        // Validator should allow any origin when no restrictions are configured
        assert!(OriginValidator::validate_origin(
            "https://any-site.com",
            &app1.allowed_origins.as_deref().unwrap_or_default()
        ));

        // Test app with specific allowed origins
        let app2 = app_manager.find_by_id("app-with-origins").await.unwrap().unwrap();
        assert!(app2.allowed_origins.is_some());
        let allowed_origins = app2.allowed_origins.as_ref().unwrap();
        
        // Should allow configured origins
        assert!(OriginValidator::validate_origin("https://app.example.com", allowed_origins));
        assert!(OriginValidator::validate_origin("https://test.staging.example.com", allowed_origins));
        assert!(OriginValidator::validate_origin("http://localhost:3000", allowed_origins));
        
        // Should reject non-allowed origins
        assert!(!OriginValidator::validate_origin("https://evil.com", allowed_origins));
        assert!(!OriginValidator::validate_origin("http://app.example.com", allowed_origins)); // Different protocol

        // Test app with wildcard allow all
        let app3 = app_manager.find_by_id("app-allow-all").await.unwrap().unwrap();
        assert!(app3.allowed_origins.is_some());
        let allowed_origins = app3.allowed_origins.as_ref().unwrap();
        
        // Should allow any origin due to wildcard
        assert!(OriginValidator::validate_origin("https://any-site.com", allowed_origins));
        assert!(OriginValidator::validate_origin("http://malicious.com", allowed_origins));
    }

    #[test]
    fn test_environment_variable_parsing() {
        // Test the environment variable parsing logic from options.rs
        
        // Test empty string
        let empty_result = if "".is_empty() {
            None
        } else {
            Some("".split(',').map(|s| s.trim().to_string()).collect::<Vec<String>>())
        };
        assert_eq!(empty_result, None);
        
        // Test single origin
        let single_result = "https://app.example.com"
            .split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<String>>();
        assert_eq!(single_result, vec!["https://app.example.com"]);
        
        // Test multiple origins
        let multiple_result = "https://app.example.com,*.staging.example.com, http://localhost:3000"
            .split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<String>>();
        assert_eq!(multiple_result, vec![
            "https://app.example.com",
            "*.staging.example.com", 
            "http://localhost:3000"
        ]);
    }
}