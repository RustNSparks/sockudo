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
        if pattern == origin {
            return true;
        }

        if pattern.contains('*') {
            Self::matches_wildcard(origin, pattern)
        } else {
            false
        }
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
}
