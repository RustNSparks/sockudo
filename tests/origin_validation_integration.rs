#[cfg(test)]
mod origin_validation_integration_tests {
    use sockudo::adapter::handler::origin_validation::OriginValidator;

    #[test]
    fn test_origin_validation_integration() {
        // Test with app that has no origin restrictions
        let no_restrictions: Vec<String> = vec![];
        assert!(OriginValidator::validate_origin(
            "https://any-site.com",
            &no_restrictions
        ));

        // Test with specific allowed origins
        let allowed_origins = vec![
            "https://app.example.com".to_string(),
            "https://*.staging.example.com".to_string(),
            "http://localhost:3000".to_string(),
        ];

        // Should allow exact match
        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &allowed_origins
        ));

        // Should allow wildcard subdomain
        assert!(OriginValidator::validate_origin(
            "https://test.staging.example.com",
            &allowed_origins
        ));

        // Should allow localhost
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed_origins
        ));

        // Should reject non-allowed origin
        assert!(!OriginValidator::validate_origin(
            "https://evil.com",
            &allowed_origins
        ));

        // Test with wildcard allow all
        let allow_all = vec!["*".to_string()];
        assert!(OriginValidator::validate_origin(
            "https://any-origin.com",
            &allow_all
        ));
    }

    #[test]
    fn test_complex_wildcard_patterns() {
        // Test protocol-specific patterns only
        let secure_patterns = vec![
            "https://*.secure.example.com".to_string(),
            "http://localhost:*".to_string(),
        ];

        // Test protocol-specific wildcard
        assert!(OriginValidator::validate_origin(
            "https://app.secure.example.com",
            &secure_patterns
        ));

        assert!(!OriginValidator::validate_origin(
            "http://app.secure.example.com",
            &secure_patterns
        ));

        // Test general subdomain wildcard (matches any protocol)
        let general_patterns = vec![
            "*.example.com".to_string(),
        ];

        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &general_patterns
        ));
        
        assert!(OriginValidator::validate_origin(
            "http://app.example.com",
            &general_patterns
        ));
        
        assert!(OriginValidator::validate_origin(
            "https://deep.nested.example.com",
            &general_patterns
        ));

        // Test localhost with any port
        let localhost_patterns = vec!["http://localhost:*".to_string()];
        
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &localhost_patterns
        ));

        assert!(OriginValidator::validate_origin(
            "http://localhost:8080",
            &localhost_patterns
        ));
    }

    #[test]
    fn test_edge_cases() {
        let origins = vec!["https://example.com".to_string()];

        // Empty origin should not match
        assert!(!OriginValidator::validate_origin("", &origins));

        // Different protocol should not match
        assert!(!OriginValidator::validate_origin(
            "http://example.com",
            &origins
        ));

        // Different port should not match exact
        assert!(!OriginValidator::validate_origin(
            "https://example.com:8080",
            &origins
        ));

        // Test case sensitivity
        assert!(!OriginValidator::validate_origin(
            "https://EXAMPLE.COM",
            &origins
        ));
    }
}