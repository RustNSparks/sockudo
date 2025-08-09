use tracing::debug;

pub struct OriginValidator;

/// Pre-processed origin pattern for efficient matching
#[derive(Debug, Clone)]
pub struct NormalizedOriginPattern {
    normalized: String,
    is_wildcard: bool,
    wildcard_prefix: String,
    wildcard_suffix: String,
}

impl NormalizedOriginPattern {
    pub fn new(pattern: &str) -> Self {
        let normalized = pattern.to_lowercase();
        let is_wildcard = pattern.contains('*');

        let (wildcard_prefix, wildcard_suffix) = if is_wildcard {
            let parts: Vec<&str> = normalized.split('*').collect();
            if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                (String::new(), String::new())
            }
        } else {
            (String::new(), String::new())
        };

        Self {
            normalized,
            is_wildcard,
            wildcard_prefix,
            wildcard_suffix,
        }
    }
}

impl OriginValidator {
    /// Validates origin patterns at configuration time
    /// Returns Ok(()) if all patterns are valid, Err(String) with error message if invalid
    pub fn validate_patterns(patterns: &[String]) -> Result<(), String> {
        for pattern in patterns {
            if let Err(e) = Self::validate_single_pattern(pattern) {
                return Err(format!("Invalid origin pattern '{}': {}", pattern, e));
            }
        }
        Ok(())
    }

    /// Validates a single origin pattern
    fn validate_single_pattern(pattern: &str) -> Result<(), String> {
        if pattern.is_empty() {
            return Err("pattern cannot be empty".to_string());
        }

        // Allow wildcard-only pattern
        if pattern == "*" {
            return Ok(());
        }

        // Check for invalid wildcard patterns
        if pattern.contains('*') {
            let wildcard_count = pattern.matches('*').count();
            if wildcard_count > 1 {
                return Err("multiple wildcards not supported".to_string());
            }

            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() != 2 {
                return Err("wildcard pattern must have exactly one '*'".to_string());
            }

            let prefix = parts[0];
            let suffix = parts[1];

            // Validate subdomain wildcard patterns
            if prefix.is_empty() && suffix.starts_with('.') {
                let domain = &suffix[1..];
                if domain.is_empty() {
                    return Err("domain part cannot be empty in '*.domain' pattern".to_string());
                }
                // Basic domain validation
                if domain.contains("..") || domain.starts_with('.') || domain.ends_with('.') {
                    return Err("invalid domain in wildcard pattern".to_string());
                }
            }
        }

        // Basic URL validation for patterns with protocols
        if pattern.contains("://") {
            if let Some(protocol_end) = pattern.find("://") {
                let protocol = &pattern[..protocol_end];
                if protocol.is_empty() {
                    return Err("protocol cannot be empty".to_string());
                }

                let host_part = &pattern[protocol_end + 3..];
                if host_part.is_empty() {
                    return Err("host part cannot be empty".to_string());
                }
            }
        }

        Ok(())
    }

    pub fn validate_origin(origin: &str, allowed_origins: &[String]) -> bool {
        if allowed_origins.is_empty() {
            debug!("No origin restrictions configured, allowing all origins");
            return true;
        }

        // Single allocation for origin normalization (RFC 6454 - case insensitive)
        let origin_lower = origin.to_lowercase();

        for allowed in allowed_origins {
            if allowed == "*" {
                debug!("Wildcard origin configured, allowing all origins");
                return true;
            }

            // Pre-compute normalized pattern to avoid repeated allocations in matching
            let normalized_pattern = NormalizedOriginPattern::new(allowed);

            if Self::matches_pattern(&origin_lower, &normalized_pattern) {
                debug!("Origin {} matches allowed pattern {}", origin, allowed);
                return true;
            }
        }

        debug!("Origin {} not in allowed list", origin);
        false
    }

    fn matches_pattern(origin: &str, pattern: &NormalizedOriginPattern) -> bool {
        // Exact match first
        if pattern.normalized == origin {
            return true;
        }

        // Handle wildcard patterns
        if pattern.is_wildcard {
            return Self::matches_wildcard(origin, pattern);
        }

        // CORS-like protocol-less matching
        // If pattern has no protocol, match against origin without protocol
        if !pattern.normalized.contains("://")
            && let Some(origin_without_protocol) = origin.split("://").nth(1)
        {
            return pattern.normalized == origin_without_protocol;
        }

        false
    }

    /// Matches wildcard patterns using pre-computed prefix/suffix.
    /// Currently supports single wildcard patterns only.
    /// Examples: "*.example.com", "https://*.example.com", "prefix*suffix"
    fn matches_wildcard(origin: &str, pattern: &NormalizedOriginPattern) -> bool {
        // Special case for protocol-less subdomain wildcards (e.g., "*.example.com")
        if pattern.wildcard_prefix.is_empty()
            && pattern.wildcard_suffix.starts_with('.')
            && !pattern.normalized.contains("://")
        {
            let domain = &pattern.wildcard_suffix[1..]; // Remove the leading dot

            // Extract host part from origin (with or without protocol)
            let host = if let Some(host_part) = origin.split("://").nth(1) {
                host_part
            } else {
                origin
            };

            // Match exact domain or any subdomain of the domain
            return host == domain || host.ends_with(&pattern.wildcard_suffix);
        }

        // General wildcard matching: origin must start with prefix and end with suffix
        origin.starts_with(&pattern.wildcard_prefix) && origin.ends_with(&pattern.wildcard_suffix)
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
    fn test_pattern_validation() {
        // Valid patterns
        assert!(OriginValidator::validate_patterns(&[]).is_ok());
        assert!(OriginValidator::validate_patterns(&["*".to_string()]).is_ok());
        assert!(OriginValidator::validate_patterns(&["https://example.com".to_string()]).is_ok());
        assert!(OriginValidator::validate_patterns(&["*.example.com".to_string()]).is_ok());
        assert!(OriginValidator::validate_patterns(&["https://*.example.com".to_string()]).is_ok());
        assert!(OriginValidator::validate_patterns(&["example.com".to_string()]).is_ok());
        assert!(OriginValidator::validate_patterns(&["localhost:3000".to_string()]).is_ok());

        // Invalid patterns
        assert!(OriginValidator::validate_patterns(&["".to_string()]).is_err());
        assert!(OriginValidator::validate_patterns(&["*.*example.com".to_string()]).is_err()); // Multiple wildcards
        assert!(OriginValidator::validate_patterns(&["*.".to_string()]).is_err()); // Empty domain
        assert!(OriginValidator::validate_patterns(&["*..example.com".to_string()]).is_err()); // Double dots
        assert!(OriginValidator::validate_patterns(&["://example.com".to_string()]).is_err()); // Empty protocol
        assert!(OriginValidator::validate_patterns(&["https://".to_string()]).is_err()); // Empty host

        // Mixed valid and invalid
        let mixed = vec![
            "https://example.com".to_string(),
            "invalid-*-*-pattern".to_string(),
        ];
        let result = OriginValidator::validate_patterns(&mixed);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("multiple wildcards not supported")
        );
    }

    #[test]
    fn test_validation_with_multiple_patterns() {
        let patterns = vec![
            "https://app.example.com".to_string(),
            "*.staging.example.com".to_string(),
            "http://localhost:3000".to_string(),
        ];

        // Test various origins against multiple patterns
        assert!(OriginValidator::validate_origin(
            "https://app.example.com",
            &patterns
        ));
        assert!(OriginValidator::validate_origin(
            "https://test.staging.example.com",
            &patterns
        ));
        assert!(OriginValidator::validate_origin(
            "http://localhost:3000",
            &patterns
        ));
        assert!(!OriginValidator::validate_origin(
            "https://unauthorized.com",
            &patterns
        ));
    }

    #[test]
    fn test_normalized_pattern_creation() {
        let pattern = NormalizedOriginPattern::new("*.Example.COM");
        assert_eq!(pattern.normalized, "*.example.com");
        assert!(pattern.is_wildcard);
        assert_eq!(pattern.wildcard_prefix, "");
        assert_eq!(pattern.wildcard_suffix, ".example.com");

        let pattern2 = NormalizedOriginPattern::new("HTTPS://Example.COM");
        assert_eq!(pattern2.normalized, "https://example.com");
        assert!(!pattern2.is_wildcard);
        assert_eq!(pattern2.wildcard_prefix, "");
        assert_eq!(pattern2.wildcard_suffix, "");
    }

    #[test]
    fn test_case_insensitive_validation() {
        let allowed = vec![
            "https://example.com".to_string(),
            "*.Example.ORG".to_string(),
        ];

        // Test exact match with different cases
        assert!(OriginValidator::validate_origin(
            "HTTPS://Example.COM",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "https://EXAMPLE.com",
            &allowed
        ));

        // Test wildcard match with different cases
        assert!(OriginValidator::validate_origin(
            "https://app.EXAMPLE.org",
            &allowed
        ));
        assert!(OriginValidator::validate_origin(
            "HTTPS://staging.example.ORG",
            &allowed
        ));

        // Should still reject non-matching origins regardless of case
        assert!(!OriginValidator::validate_origin(
            "https://OTHER.com",
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
