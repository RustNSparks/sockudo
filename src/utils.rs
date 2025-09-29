use std::env;
use std::sync::LazyLock;

use crate::app::config::App;
use crate::error::Error;
use regex::Regex;
use tracing::warn;

// Compile regexes once using lazy_static
static CACHING_CHANNEL_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let patterns = vec![
        "cache-*",
        "private-cache-*",
        "private-encrypted-cache-*",
        "presence-cache-*",
    ];
    patterns
        .into_iter()
        .map(|pattern| Regex::new(&pattern.replace("*", ".*")).unwrap())
        .collect()
});

pub fn is_cache_channel(channel: &str) -> bool {
    // Use the pre-compiled regexes
    for regex in CACHING_CHANNEL_REGEXES.iter() {
        if regex.is_match(channel) {
            return true;
        }
    }
    false
}

pub fn data_to_bytes<T: AsRef<str> + serde::Serialize>(data: &[T]) -> usize {
    data.iter()
        .map(|element| {
            // Convert element to string representation
            let string_data = element.as_ref().to_string();

            // Calculate byte length of string
            string_data.len()
        })
        .sum()
}

pub fn data_to_bytes_flexible(data: Vec<serde_json::Value>) -> usize {
    data.iter().fold(0, |total_bytes, element| {
        let element_str = if element.is_string() {
            // Use string value directly if it's a string
            element.as_str().unwrap_or_default().to_string()
        } else {
            // Convert to JSON string for other types
            serde_json::to_string(element).unwrap_or_default()
        };

        // Add byte length, handling potential encoding errors
        // No specific error handling for as_bytes().len() as it's inherent to string length.
        total_bytes + element_str.len()
    })
}

pub async fn validate_channel_name(app: &App, channel: &str) -> crate::error::Result<()> {
    if channel.len() > app.max_channel_name_length.unwrap_or(200) as usize {
        return Err(Error::Channel(format!(
            "Channel name too long. Max length is {}",
            app.max_channel_name_length.unwrap_or(200)
        )));
    }
    if !channel.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || c == '-'
            || c == '_'
            || c == '='
            || c == '@'
            || c == '.'
            || c == ':'
            || c == '#'
    }) {
        return Err(Error::Channel(
            "Channel name contains invalid characters".to_string(),
        ));
    }

    Ok(())
}

/// Parse a boolean value from an environment variable with flexible format support.
///
/// Supports the following formats (case-insensitive):
/// - true: "true", "1", "yes", "on"
/// - false: "false", "0", "no", "off"
///
/// Returns the default value if the environment variable is not set.
/// Logs a warning and returns the default value for unrecognized formats.
pub fn parse_bool_env(var_name: &str, default: bool) -> bool {
    match env::var(var_name) {
        Ok(s) => {
            let lowercased_s = s.to_lowercase();
            match lowercased_s.as_str() {
                "true" | "1" | "yes" | "on" => true,
                "false" | "0" | "no" | "off" => false,
                _ => {
                    warn!(
                        "Unrecognized value for {}: '{}'. Defaulting to {}.",
                        var_name, s, default
                    );
                    default
                }
            }
        }
        Err(_) => default, // Variable not set, use default
    }
}

/// Parse an environment variable as any numeric type that implements FromStr + Display.
///
/// Returns the default value if the environment variable is not set.
/// Logs a warning and returns the default value if parsing fails.
pub fn parse_env<T>(var_name: &str, default: T) -> T
where
    T: std::str::FromStr + std::fmt::Display + Clone,
    T::Err: std::fmt::Display,
{
    match env::var(var_name) {
        Ok(s) => match s.parse::<T>() {
            Ok(value) => value,
            Err(e) => {
                warn!(
                    "Failed to parse {} as {}: '{}'. Error: {}. Defaulting to {}.",
                    var_name,
                    std::any::type_name::<T>(),
                    s,
                    e,
                    default
                );
                default
            }
        },
        Err(_) => default, // Variable not set, use default
    }
}

/// Parse an environment variable as any numeric type, returning None if not set.
///
/// Returns Some(value) if the environment variable is set and parsing succeeds.
/// Returns None if the environment variable is not set.
/// Logs a warning and returns None if parsing fails.
pub fn parse_env_optional<T>(var_name: &str) -> Option<T>
where
    T: std::str::FromStr + std::fmt::Display + Clone,
    T::Err: std::fmt::Display,
{
    match env::var(var_name) {
        Ok(s) => match s.parse::<T>() {
            Ok(value) => Some(value),
            Err(e) => {
                warn!(
                    "Failed to parse {} as {}: '{}'. Error: {}.",
                    var_name,
                    std::any::type_name::<T>(),
                    s,
                    e
                );
                None
            }
        },
        Err(_) => None, // Variable not set
    }
}

/// Resolve a hostname and port to a SocketAddr with proper DNS resolution.
///
/// Features:
/// - Prefers IPv4 addresses over IPv6 for better compatibility
/// - Handles hostname resolution (localhost, domain names, etc.)
/// - Falls back to 127.0.0.1 with the specified port on resolution failure
/// - Logs warnings for IPv6-only resolution or failures with context
///
/// # Parameters
/// - `host`: The hostname or IP address to resolve
/// - `port`: The port number
/// - `context`: A description of what this address is for (e.g., "HTTP server", "metrics server")
pub async fn resolve_socket_addr(host: &str, port: u16, context: &str) -> std::net::SocketAddr {
    use std::net::SocketAddr;
    use tokio::net::lookup_host;

    let host_port = format!("{host}:{port}");

    match lookup_host(&host_port).await {
        Ok(addrs) => {
            // Prefer IPv4 addresses to avoid potential IPv6 issues
            let mut ipv4_addr = None;
            let mut ipv6_addr = None;

            for addr in addrs {
                if addr.is_ipv4() && ipv4_addr.is_none() {
                    ipv4_addr = Some(addr);
                } else if addr.is_ipv6() && ipv6_addr.is_none() {
                    ipv6_addr = Some(addr);
                }
            }

            if let Some(addr) = ipv4_addr {
                addr
            } else if let Some(addr) = ipv6_addr {
                warn!(
                    "[{}] Only IPv6 address found for {}, using: {}",
                    context, host_port, addr
                );
                addr
            } else {
                let fallback = SocketAddr::new("127.0.0.1".parse().unwrap(), port);
                warn!(
                    "[{}] No addresses found for {}. Using fallback {}",
                    context, host_port, fallback
                );
                fallback
            }
        }
        Err(e) => {
            let fallback = SocketAddr::new("127.0.0.1".parse().unwrap(), port);
            warn!(
                "[{}] Failed to resolve {}: {}. Using fallback {}",
                context, host_port, e, fallback
            );
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_data_to_bytes_flexible() {
        let test_data = vec![
            json!("simple string"),
            json!(123),
            json!({"key": "value"}),
            json!([1, 2, 3]),
        ];

        let bytes = data_to_bytes_flexible(test_data);
        assert!(bytes > 0);
    }

    #[tokio::test]
    async fn test_validate_channel_name() {
        let app: App = App {
            id: "test-app".to_string(),
            key: "test-key".to_string(),
            secret: "test-secret".to_string(),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_client_events_per_second: 100,
            max_channel_name_length: Some(50),
            ..Default::default()
        };

        // Test valid channel names
        assert!(validate_channel_name(&app, "test-123").await.is_ok());
        assert!(validate_channel_name(&app, "user@domain").await.is_ok());
        assert!(validate_channel_name(&app, "channel.name").await.is_ok());
        assert!(
            validate_channel_name(&app, "#server-to-user-123")
                .await
                .is_ok()
        );

        // Test invalid channel names
        assert!(
            validate_channel_name(&app, "invalid$channel")
                .await
                .is_err()
        );
        assert!(
            validate_channel_name(
                &app,
                "too_long_channel_name_that_exceeds_fifty_characters_and_should_fail"
            )
            .await
            .is_err()
        );
        assert!(validate_channel_name(&app, "space in name").await.is_err());
    }
}
