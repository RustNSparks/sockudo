#[cfg(test)]
mod unix_socket_config_tests {

    use sockudo::options::{ServerOptions, UnixSocketConfig};
    use std::env;

    #[test]
    fn test_unix_socket_config_defaults() {
        let config = UnixSocketConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.path, "/var/run/sockudo/sockudo.sock");
        assert_eq!(config.permission_mode, 0o660); // rw-rw----
    }

    #[test]
    fn test_unix_socket_config_json_deserialization() {
        // Test string format (recommended)
        let json = r#"{
            "enabled": true,
            "path": "/tmp/test.sock",
            "permission_mode": "755"
        }"#;

        let config: UnixSocketConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.path, "/tmp/test.sock");
        assert_eq!(config.permission_mode, 0o755);
    }

    #[test]
    fn test_unix_socket_config_invalid_octal_permission() {
        // Test invalid octal digits
        let json = r#"{
            "enabled": true,
            "path": "/tmp/test.sock",
            "permission_mode": "789"
        }"#;

        let result: Result<UnixSocketConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must contain only digits 0-7")
        );
    }

    #[test]
    fn test_unix_socket_config_permission_too_high() {
        // Test permission above 777 (but valid octal digits)
        let json = r#"{
            "enabled": true,
            "path": "/tmp/test.sock",
            "permission_mode": "1000"
        }"#;

        let result: Result<UnixSocketConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum value 777")
        );
    }

    #[test]
    fn test_unix_socket_config_permission_invalid_octal_digits() {
        // Test invalid octal digits (8 and 9 are not valid in octal)
        let json = r#"{
            "enabled": true,
            "path": "/tmp/test.sock",
            "permission_mode": "888"
        }"#;

        let result: Result<UnixSocketConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must contain only digits 0-7")
        );
    }

    #[test]
    fn test_unix_socket_config_various_valid_permissions() {
        let test_cases = vec![
            ("600", 0o600), // rw-------
            ("644", 0o644), // rw-r--r--
            ("660", 0o660), // rw-rw----
            ("755", 0o755), // rwxr-xr-x
            ("777", 0o777), // rwxrwxrwx
            ("000", 0o000), // ---------
        ];

        for (perm_str, expected) in test_cases {
            let json = format!(
                r#"{{
                "enabled": true,
                "path": "/tmp/test.sock",
                "permission_mode": "{}"
            }}"#,
                perm_str
            );

            let config: UnixSocketConfig = serde_json::from_str(&json).unwrap();
            assert_eq!(
                config.permission_mode, expected,
                "Failed for permission {}",
                perm_str
            );
        }
    }

    #[tokio::test]
    async fn test_env_override_unix_socket_enabled() {
        unsafe {
            env::set_var("UNIX_SOCKET_ENABLED", "true");
        }

        let mut config = ServerOptions::default();
        assert!(!config.unix_socket.enabled); // Default is false

        config.override_from_env().await.unwrap();
        assert!(config.unix_socket.enabled);

        unsafe {
            env::remove_var("UNIX_SOCKET_ENABLED");
        }
    }

    #[tokio::test]
    async fn test_env_override_unix_socket_path() {
        unsafe {
            env::set_var("UNIX_SOCKET_PATH", "/custom/path.sock");
        }

        let mut config = ServerOptions::default();
        assert_eq!(config.unix_socket.path, "/var/run/sockudo/sockudo.sock"); // Default

        config.override_from_env().await.unwrap();
        assert_eq!(config.unix_socket.path, "/custom/path.sock");

        unsafe {
            env::remove_var("UNIX_SOCKET_PATH");
        }
    }


    #[tokio::test]
    async fn test_env_override_unix_socket_permission_invalid() {
        unsafe {
            env::set_var("UNIX_SOCKET_PERMISSION_MODE", "999"); // Invalid: exceeds 777
        }

        let mut config = ServerOptions::default();
        let original_mode = config.unix_socket.permission_mode;

        config.override_from_env().await.unwrap();
        // Should keep original value when invalid
        assert_eq!(config.unix_socket.permission_mode, original_mode);

        unsafe {
            env::remove_var("UNIX_SOCKET_PERMISSION_MODE");
        }
    }

    #[tokio::test]
    async fn test_env_override_unix_socket_permission_non_octal() {
        unsafe {
            env::set_var("UNIX_SOCKET_PERMISSION_MODE", "abc"); // Invalid: non-octal
        }

        let mut config = ServerOptions::default();
        let original_mode = config.unix_socket.permission_mode;

        config.override_from_env().await.unwrap();
        // Should keep original value when invalid
        assert_eq!(config.unix_socket.permission_mode, original_mode);

        unsafe {
            env::remove_var("UNIX_SOCKET_PERMISSION_MODE");
        }
    }

    #[test]
    fn test_unix_socket_validation_enabled_empty_path() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;
        config.unix_socket.path = "".to_string(); // Empty path

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Unix socket path cannot be empty")
        );
    }

    #[test]
    fn test_unix_socket_validation_permission_too_high() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;
        config.unix_socket.permission_mode = 0o1000; // Invalid: exceeds 777

        let result = config.validate();
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("permission_mode"));
        assert!(error_msg.contains("invalid"));
    }

    #[test]
    fn test_unix_socket_security_validation_directory_traversal() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;

        let dangerous_paths = vec![
            "/tmp/../etc/passwd.sock",
            "/var/run/../../../root/evil.sock",
            "..\\Windows\\System32\\evil.sock",
        ];

        for path in dangerous_paths {
            config.unix_socket.path = path.to_string();
            let result = config.validate();
            assert!(result.is_err(), "Should reject dangerous path: {}", path);
            assert!(
                result.unwrap_err().contains("directory traversal"),
                "Path: {}",
                path
            );
        }
    }

    #[test]
    fn test_unix_socket_security_validation_relative_path() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;

        let relative_paths = vec![
            "socket.sock",
            "tmp/socket.sock",
            "./socket.sock",
            "run/socket.sock",
        ];

        for path in relative_paths {
            config.unix_socket.path = path.to_string();
            let result = config.validate();
            assert!(result.is_err(), "Should reject relative path: {}", path);
            assert!(
                result.unwrap_err().contains("must be absolute"),
                "Path: {}",
                path
            );
        }
    }

    #[test]
    fn test_unix_socket_security_validation_safe_paths() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;

        let safe_paths = vec![
            "/tmp/sockudo.sock",
            "/var/run/sockudo/sockudo.sock",
            "/home/user/app/socket.sock",
            "/opt/sockudo/socket.sock",
            "/run/sockudo.sock",
        ];

        for path in safe_paths {
            config.unix_socket.path = path.to_string();
            let result = config.validate();
            assert!(
                result.is_ok(),
                "Should accept safe path: {} - Error: {:?}",
                path,
                result
            );
        }
    }

    #[test]
    fn test_unix_socket_validation_disabled_ignores_path() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = false; // Disabled
        config.unix_socket.path = "/etc/passwd.sock".to_string(); // Dangerous path

        // Should pass validation when disabled
        let result = config.validate();
        assert!(
            result.is_ok(),
            "Disabled Unix socket should ignore dangerous paths"
        );
    }

    #[test]
    fn test_full_server_options_with_unix_socket() {
        let json = r#"{
            "debug": false,
            "host": "127.0.0.1",
            "port": 6001,
            "unix_socket": {
                "enabled": true,
                "path": "/tmp/test-sockudo.sock",
                "permission_mode": "664"
            }
        }"#;

        let config: ServerOptions = serde_json::from_str(json).unwrap();
        assert!(config.unix_socket.enabled);
        assert_eq!(config.unix_socket.path, "/tmp/test-sockudo.sock");
        assert_eq!(config.unix_socket.permission_mode, 0o664);

        // Should pass validation
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_unix_socket_ssl_warning_scenario() {
        let mut config = ServerOptions::default();
        config.unix_socket.enabled = true;
        config.ssl.enabled = true; // Both Unix socket and SSL enabled

        // Should pass validation but would log a warning (we can't easily test logging in unit tests)
        let result = config.validate();
        assert!(
            result.is_ok(),
            "Unix socket + SSL should be valid but unusual"
        );
    }
}
