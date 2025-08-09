use tracing::debug;

pub struct OriginValidator;

impl OriginValidator {
    pub fn validate_origin(origin: &str, allowed_origins: &[String]) -> bool {
        if allowed_origins.is_empty() {
            debug!("No origin restrictions configured, allowing all origins");
            return true;
        }

        for allowed in allowed_origins {
            if allowed == "*" {
                debug!("Wildcard origin configured, allowing all origins");
                return true;
            }

            if Self::matches_pattern(origin, allowed) {
                debug!("Origin {} matches allowed pattern {}", origin, allowed);
                return true;
            }
        }

        debug!("Origin {} not in allowed list", origin);
        false
    }

    fn matches_pattern(origin: &str, pattern: &str) -> bool {
        // Exact match first
        if pattern == origin {
            return true;
        }

        // Handle wildcard patterns
        if pattern.contains('*') {
            return Self::matches_wildcard(origin, pattern);
        }

        // CORS-like protocol-less matching
        // If pattern has no protocol, match against origin without protocol
        if !pattern.contains("://")
            && let Some(origin_without_protocol) = origin.split("://").nth(1)
        {
            return pattern == origin_without_protocol;
        }

        false
    }

    fn matches_wildcard(origin: &str, pattern: &str) -> bool {
        let pattern_parts: Vec<&str> = pattern.split('*').collect();

        if pattern_parts.len() != 2 {
            return false;
        }

        let prefix = pattern_parts[0];
        let suffix = pattern_parts[1];

        // Special case for subdomain wildcards without protocol (e.g., "*.example.com")
        if pattern.starts_with("*.") && !pattern.contains("://") {
            let domain_suffix = &pattern[2..];

            // Allow exact domain match
            if origin == domain_suffix {
                return true;
            }

            // Check if origin (with or without protocol) ends with the domain pattern
            if let Some(origin_without_protocol) = origin.split("://").nth(1) {
                if origin_without_protocol == domain_suffix {
                    return true;
                }

                if origin_without_protocol.ends_with(&format!(".{}", domain_suffix)) {
                    return true;
                }
            }

            return origin.ends_with(&format!(".{}", domain_suffix));
        }

        // General wildcard matching with prefix and suffix
        origin.starts_with(prefix) && origin.ends_with(suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_allowed_origins() {
        assert!(OriginValidator::validate_origin("https://example.com", &[]));
    }

    #[test]
    fn test_wildcard_allows_all() {
        let allowed = vec!["*".to_string()];
        assert!(OriginValidator::validate_origin(
            "https://example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed
        ));
    }

    #[test]
    fn test_exact_match() {
        let allowed = vec!["https://example.com".to_string()];
        assert!(OriginValidator::validate_origin(
            "https://example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "https://other.com",
            &allowed
        ));
    }

    #[test]
    fn test_subdomain_wildcard() {
        let allowed = vec!["*.example.com".to_string()];
        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://staging.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://deep.nested.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin("example.com", &allowed));
        assert!(!OriginValidator::validate_origin(
            "https://example.org",
            &allowed
        ));
    }

    #[test]
    fn test_protocol_wildcard() {
        let allowed = vec!["https://*.example.com".to_string()];
        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://app.example.com",
            &allowed
        ));
    }

    #[test]
    fn test_multiple_allowed_origins() {
        let allowed = vec![
            "https://app.example.com".to_string(),
            "http://localhost:3000".to_string(),
            "*.staging.example.com".to_string(),
        ];
        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://test.staging.example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "https://other.com",
            &allowed
        ));
    }

    #[test]
    fn test_port_handling() {
        let allowed = vec!["http://localhost:3000".to_string()];
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://localhost:3001",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://localhost",
            &allowed
        ));
    }

    #[test]
    fn test_cors_like_protocol_less_matching() {
        let allowed = vec!["example.com".to_string()];

        // Protocol-less pattern should match any protocol
        assert!(OriginValidator::validate_origin(
            "https://example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://example.com",
            &allowed
        ));

        // Should not match different domains
        assert!(!OriginValidator::validate_origin(
            "https://other.com",
            &allowed
        ));
    }

    #[test]
    fn test_cors_like_with_ports() {
        let allowed = vec!["node1.ghslocal.com:444".to_string()];

        // Should match both protocols with same host:port
        assert!(OriginValidator::validate_origin(
            "https://node1.ghslocal.com:444",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://node1.ghslocal.com:444",
            &allowed
        ));

        // Should not match different ports
        assert!(!OriginValidator::validate_origin(
            "https://node1.ghslocal.com:443",
            &allowed
        ));

        // Should not match without port when pattern has port
        assert!(!OriginValidator::validate_origin(
            "https://node1.ghslocal.com",
            &allowed
        ));
    }

    #[test]
    fn test_mixed_protocol_patterns() {
        let allowed = vec![
            "https://secure.example.com".to_string(), // Protocol-specific
            "flexible.example.com".to_string(),       // Protocol-agnostic
            "http://insecure.example.com".to_string(), // HTTP only
        ];

        // Protocol-specific should only match exact protocol
        assert!(OriginValidator::validate_origin(
            "https://secure.example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://secure.example.com", // Different protocol
            &allowed
        ));

        // Protocol-agnostic should match any protocol
        assert!(OriginValidator::validate_origin(
            "https://flexible.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://flexible.example.com",
            &allowed
        ));

        // HTTP-only should only match HTTP
        assert!(OriginValidator::validate_origin(
            "http://insecure.example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "https://insecure.example.com", // Different protocol
            &allowed
        ));
    }

    #[test]
    fn test_protocol_less_with_subdomains() {
        let allowed = vec!["api.example.com".to_string()];

        // Should match exact subdomain with any protocol
        assert!(OriginValidator::validate_origin(
            "https://api.example.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://api.example.com",
            &allowed
        ));

        // Should not match different subdomains
        assert!(!OriginValidator::validate_origin(
            "https://app.example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "https://example.com", // Missing subdomain
            &allowed
        ));
    }

    #[test]
    fn test_backwards_compatibility() {
        let allowed = vec![
            "https://old-style.com".to_string(), // Full URLs should still work
            "new-style.com".to_string(),         // Protocol-less should work
        ];

        // Old behavior: exact match with protocol
        assert!(OriginValidator::validate_origin(
            "https://old-style.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "http://old-style.com", // Different protocol should not match
            &allowed
        ));

        // New behavior: protocol-less matches any protocol
        assert!(OriginValidator::validate_origin(
            "https://new-style.com",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "http://new-style.com",
            &allowed
        ));
    }

    #[test]
    fn test_edge_cases_with_protocols() {
        let allowed = vec![
            "localhost:3000".to_string(),
            "127.0.0.1:8080".to_string(),
            "custom-protocol://example.com".to_string(), // Unusual protocol
        ];

        // Localhost with port
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://localhost:3000",
            &allowed
        ));

        // IP with port
        assert!(OriginValidator::validate_origin(
            "http://127.0.0.1:8080",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://127.0.0.1:8080",
            &allowed
        ));

        // Custom protocol should match exactly
        assert!(OriginValidator::validate_origin(
            "custom-protocol://example.com",
            &allowed
        ));
        assert!(!OriginValidator::validate_origin(
            "https://example.com", // Different protocol
            &allowed
        ));
    }
}
