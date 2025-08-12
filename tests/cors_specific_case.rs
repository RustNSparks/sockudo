#[cfg(test)]
mod cors_specific_case_tests {
    use sockudo::adapter::handler::origin_validation::OriginValidator;

    #[test]
    fn test_protocol_agnostic_with_custom_port() {
        // Test protocol-agnostic pattern with custom domain and port
        let allowed_origins = vec!["node1.ghslocal.com:444".to_string()];

        // Should work with HTTPS
        assert!(OriginValidator::validate_origin(
            "https://node1.ghslocal.com:444",
            &allowed_origins
        ));

        // Should work with HTTP
        assert!(OriginValidator::validate_origin(
            "http://node1.ghslocal.com:444",
            &allowed_origins
        ));

        // Wrong port should not work
        assert!(!OriginValidator::validate_origin(
            "https://node1.ghslocal.com:443",
            &allowed_origins
        ));

        // Wrong domain should not work
        assert!(!OriginValidator::validate_origin(
            "https://node2.ghslocal.com:444",
            &allowed_origins
        ));
    }

    #[test]
    fn test_common_cors_patterns() {
        let allowed_origins = vec![
            "localhost:3000".to_string(),
            "api.example.com".to_string(),
            "*.staging.example.com".to_string(),
            "https://secure.example.com".to_string(), // Protocol-specific
        ];

        // Localhost should work with any protocol
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed_origins
        ));
        assert!(OriginValidator::validate_origin(
            "https://localhost:3000",
            &allowed_origins
        ));

        // API subdomain should work with any protocol
        assert!(OriginValidator::validate_origin(
            "https://api.example.com",
            &allowed_origins
        ));
        assert!(OriginValidator::validate_origin(
            "http://api.example.com",
            &allowed_origins
        ));

        // Wildcard staging should work with any protocol
        assert!(OriginValidator::validate_origin(
            "https://test.staging.example.com",
            &allowed_origins
        ));
        assert!(OriginValidator::validate_origin(
            "http://app.staging.example.com",
            &allowed_origins
        ));

        // Protocol-specific should only work with HTTPS
        assert!(OriginValidator::validate_origin(
            "https://secure.example.com",
            &allowed_origins
        ));
        assert!(!OriginValidator::validate_origin(
            "http://secure.example.com", // Wrong protocol
            &allowed_origins
        ));
    }
}
