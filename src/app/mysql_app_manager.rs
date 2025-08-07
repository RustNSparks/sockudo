use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};

use crate::options::DatabaseConnection;
use crate::token::Token;
use crate::websocket::SocketId;
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use moka::future::Cache;
use sqlx::{MySqlPool, mysql::MySqlPoolOptions};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Configuration for MySQL App Manager
/// MySQL-based implementation of the AppManager
pub struct MySQLAppManager {
    config: DatabaseConnection,
    pool: MySqlPool,
    app_cache: Cache<String, App>, // App ID -> App
}

impl MySQLAppManager {
    /// Create a new MySQL-based AppManager with the provided configuration
    pub async fn new(config: DatabaseConnection) -> Result<Self> {
        info!(
            "{}",
            format!(
                "Initializing MySQL AppManager with database {}",
                config.database
            )
        );

        // Build connection string with proper URL encoding for password
        let password = urlencoding::encode(&config.password);
        let connection_string = format!(
            "mysql://{}:{}@{}:{}/{}",
            config.username, password, config.host, config.port, config.database
        );

        // Create connection pool with improved options
        let pool = MySqlPoolOptions::new()
            .max_connections(config.connection_pool_size)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(180))
            .connect(&connection_string)
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to MySQL: {e}")))?;

        // Initialize cache
        let app_cache = Cache::builder()
            .time_to_live(Duration::from_secs(config.cache_ttl))
            .max_capacity(config.cache_max_capacity)
            // Add other options like time_to_idle if needed
            .build();

        let manager = Self {
            config,
            pool,
            app_cache,
        };

        manager.ensure_table_exists().await?;

        Ok(manager)
    }

    /// Create the applications table if it doesn't exist
    async fn ensure_table_exists(&self) -> Result<()> {
        // Use a constant query (avoid format!) for security
        const CREATE_TABLE_QUERY: &str = r#"
            CREATE TABLE IF NOT EXISTS `applications` (
                id VARCHAR(255) PRIMARY KEY,
                `key` VARCHAR(255) UNIQUE NOT NULL,
                secret VARCHAR(255) NOT NULL,
                max_connections INT UNSIGNED NOT NULL,
                enable_client_messages BOOLEAN NOT NULL DEFAULT FALSE,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                max_backend_events_per_second INT UNSIGNED NULL,
                max_client_events_per_second INT UNSIGNED NOT NULL,
                max_read_requests_per_second INT UNSIGNED NULL,
                max_presence_members_per_channel INT UNSIGNED NULL,
                max_presence_member_size_in_kb INT UNSIGNED NULL,
                max_channel_name_length INT UNSIGNED NULL,
                max_event_channels_at_once INT UNSIGNED NULL,
                max_event_name_length INT UNSIGNED NULL,
                max_event_payload_in_kb INT UNSIGNED NULL,
                max_event_batch_size INT UNSIGNED NULL,
                enable_user_authentication BOOLEAN NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
        "#;

        // Replace table name with the configured one if different from default
        let query = if self.config.table_name != "applications" {
            CREATE_TABLE_QUERY.replace("applications", &self.config.table_name)
        } else {
            CREATE_TABLE_QUERY.to_string()
        };

        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create MySQL table: {e}")))?;

        info!(
            "{}",
            format!("Ensured table '{}' exists", self.config.table_name)
        );
        Ok(())
    }

    /// Get an app by ID from cache or database
    pub async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        // Try to get from cache first
        if let Some(app) = self.app_cache.get(app_id).await {
            return Ok(Some(app));
        }

        // Not in cache or expired, fetch from database
        debug!("Cache miss for app {}, fetching from database", app_id);

        // Use a query_as that matches your App struct
        // Create the query with the correct table name
        let query = format!(
            r#"SELECT
                id, `key`, secret, max_connections,
                enable_client_messages, enabled,
                max_backend_events_per_second,
                max_client_events_per_second,
                max_read_requests_per_second,
                max_presence_members_per_channel,
                max_presence_member_size_in_kb,
                max_channel_name_length,
                max_event_channels_at_once,
                max_event_name_length,
                max_event_payload_in_kb,
                max_event_batch_size,
                enable_user_authentication
            FROM `{}` WHERE id = ?"#,
            self.config.table_name
        );

        let app_result = sqlx::query_as::<_, AppRow>(&query)
            .bind(app_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                error!(
                    "{}",
                    format!("Database error fetching app {}: {}", app_id, e)
                );
                Error::Internal(format!("Failed to fetch app from MySQL: {e}"))
            })?;

        if let Some(app_row) = app_result {
            // Convert to App
            let app = app_row.into_app();

            // Update cache
            self.app_cache.insert(app_id.to_string(), app.clone()).await;

            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    /// Get an app by key from cache or database
    pub async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        // Check cache first
        if let Some(app) = self.app_cache.get(key).await {
            return Ok(Some(app));
        }

        // Not found in cache, query database
        debug!("Cache miss for app key {}, fetching from database", key);

        let query = format!(
            r#"SELECT
                id, `key`, secret, max_connections,
                enable_client_messages, enabled,
                max_backend_events_per_second,
                max_client_events_per_second,
                max_read_requests_per_second,
                max_presence_members_per_channel,
                max_presence_member_size_in_kb,
                max_channel_name_length,
                max_event_channels_at_once,
                max_event_name_length,
                max_event_payload_in_kb,
                max_event_batch_size,
                enable_user_authentication
            FROM `{}` WHERE `key` = ?"#,
            self.config.table_name
        );

        let app_result = sqlx::query_as::<_, AppRow>(&query)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                error!(
                    "{}",
                    format!("Database error fetching app by key {}: {}", key, e)
                );
                Error::Internal(format!("Failed to fetch app from MySQL: {e}"))
            })?;

        if let Some(app_row) = app_result {
            let app = app_row.into_app();

            // Update cache with this app
            let app_id = app.id.clone();
            self.app_cache.insert(app_id, app.clone()).await;

            Ok(Some(app.clone()))
        } else {
            Ok(None)
        }
    }

    /// Register a new app in the database
    pub async fn create_app(&self, app: App) -> Result<()> {
        info!("{}", format!("Registering new app: {}", app.id));

        // Prepare the query with proper table name
        let query = format!(
            r#"INSERT INTO `{}` (
                id, `key`, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size, enable_user_authentication
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            self.config.table_name
        );

        sqlx::query(&query)
            .bind(&app.id)
            .bind(&app.key)
            .bind(&app.secret)
            .bind(app.max_connections)
            .bind(app.enable_client_messages)
            .bind(app.enabled)
            .bind(app.max_backend_events_per_second)
            .bind(app.max_client_events_per_second)
            .bind(app.max_read_requests_per_second)
            .bind(app.max_presence_members_per_channel)
            .bind(app.max_presence_member_size_in_kb)
            .bind(app.max_channel_name_length)
            .bind(app.max_event_channels_at_once)
            .bind(app.max_event_name_length)
            .bind(app.max_event_payload_in_kb)
            .bind(app.max_event_batch_size)
            .bind(app.enable_user_authentication)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(
                    "{}",
                    format!("Database error registering app {}: {}", app.id, e)
                );
                Error::Internal(format!("Failed to insert app into MySQL: {e}"))
            })?;

        // Update cach
        self.app_cache.insert(app.id.clone(), app).await;

        Ok(())
    }

    /// Update an existing app in the database
    pub async fn update_app(&self, app: App) -> Result<()> {
        info!("{}", format!("Updating app: {}", app.id));

        // Prepare the query with proper table name
        let query = format!(
            r#"UPDATE `{}` SET
                `key` = ?, secret = ?, max_connections = ?, enable_client_messages = ?, enabled = ?,
                max_backend_events_per_second = ?, max_client_events_per_second = ?,
                max_read_requests_per_second = ?, max_presence_members_per_channel = ?,
                max_presence_member_size_in_kb = ?, max_channel_name_length = ?,
                max_event_channels_at_once = ?, max_event_name_length = ?,
                max_event_payload_in_kb = ?, max_event_batch_size = ?, enable_user_authentication = ?
                WHERE id = ?"#,
            self.config.table_name
        );

        let result = sqlx::query(&query)
            .bind(&app.key)
            .bind(&app.secret)
            .bind(app.max_connections)
            .bind(app.enable_client_messages)
            .bind(app.enabled)
            .bind(app.max_backend_events_per_second)
            .bind(app.max_client_events_per_second)
            .bind(app.max_read_requests_per_second)
            .bind(app.max_presence_members_per_channel)
            .bind(app.max_presence_member_size_in_kb)
            .bind(app.max_channel_name_length)
            .bind(app.max_event_channels_at_once)
            .bind(app.max_event_name_length)
            .bind(app.max_event_payload_in_kb)
            .bind(app.max_event_batch_size)
            .bind(app.enable_user_authentication)
            .bind(&app.id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(
                    "{}",
                    format!("Database error updating app {}: {}", app.id, e)
                );
                Error::Internal(format!("Failed to update app in MySQL: {e}"))
            })?;

        if result.rows_affected() == 0 {
            return Err(Error::InvalidAppKey);
        }

        // Update cache
        self.app_cache.insert(app.id.clone(), app).await;

        Ok(())
    }

    /// Remove an app from the database
    pub async fn delete_app(&self, app_id: &str) -> Result<()> {
        info!("{}", format!("Removing app: {}", app_id));

        // Prepare the query with proper table name
        let query = format!(r#"DELETE FROM `{}` WHERE id = ?"#, self.config.table_name);

        let result = sqlx::query(&query)
            .bind(app_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!(
                    "{}",
                    format!("Database error removing app {}: {}", app_id, e)
                );
                Error::Internal(format!("Failed to delete app from MySQL: {e}"))
            })?;

        if result.rows_affected() == 0 {
            return Err(Error::InvalidAppKey);
        }

        // Remove from cache
        self.app_cache.remove(app_id).await;

        Ok(())
    }

    /// Get all apps from the database
    pub async fn get_apps(&self) -> Result<Vec<App>> {
        info!("{}", "Fetching all apps from database");

        let query = format!(
            r#"SELECT
            id, `key`, secret, max_connections,
            enable_client_messages, enabled,
            max_backend_events_per_second,
            max_client_events_per_second,
            max_read_requests_per_second,
            max_presence_members_per_channel,
            max_presence_member_size_in_kb,
            max_channel_name_length,
            max_event_channels_at_once,
            max_event_name_length,
            max_event_payload_in_kb,
            max_event_batch_size,
            enable_user_authentication,
            enable_watchlist_events
        FROM `{}`"#,
            self.config.table_name // Ensure config.table_name is safely handled
        );

        // Fetch all rows from the database
        let app_rows = sqlx::query_as::<_, AppRow>(&query) // Ensure AppRow derives FromRow
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("{}", format!("Database error fetching all apps: {}", e));
                // Consider a more specific error type if possible
                Error::Internal(format!("Failed to fetch apps from MySQL: {e}"))
            })?;

        warn!(
            "{}",
            format!("Fetched {} app rows from database.", app_rows.len())
        );

        // Process rows concurrently using streams:
        // 1. Convert iterator to stream
        // 2. Map each row to an async block that converts, caches, and returns the App
        // 3. Buffer the async operations for concurrency
        // 4. Collect the results (Apps) into a Vec
        let apps = stream::iter(app_rows)
            .map(|row| async {
                let app = row.into_app(); // Convert row to App struct
                let app_arc = Arc::new(app.clone()); // Create Arc for caching

                // Insert the Arc<App> into the cache
                // Note: insert takes key by value, so clone app_arc.id
                self.app_cache.insert(app_arc.id.clone(), app.clone()).await;

                // Return the owned App for the final Vec<App>
                app
            })
            // Execute up to N futures concurrently (e.g., based on pool size)
            // Adjust buffer size as needed. Using connection_pool_size might be reasonable.
            .buffer_unordered(self.config.connection_pool_size as usize)
            .collect::<Vec<App>>() // Collect the resulting Apps
            .await; // Await the stream processing

        info!(
            "{}",
            format!("Finished processing and caching {} apps.", apps.len())
        );

        Ok(apps)
    }

    /// Validate if an app ID exists
    pub async fn validate_key(&self, app_id: &str) -> Result<bool> {
        Ok(self.find_by_id(app_id).await?.is_some())
    }

    /// Validate a signature against an app's secret
    pub async fn validate_signature(
        &self,
        app_id: &str,
        signature: &str,
        body: &str,
    ) -> Result<bool> {
        let app = self
            .find_by_id(app_id)
            .await?
            .ok_or_else(|| Error::InvalidAppKey)?;

        let token = Token::new(app.key.clone(), app.secret.clone());
        let expected = token.sign(body);

        Ok(signature == expected)
    }

    /// Validate if a channel name is valid for an app
    pub async fn validate_channel_name(&self, app_id: &str, channel: &str) -> Result<()> {
        let app = self
            .find_by_id(app_id)
            .await?
            .ok_or_else(|| Error::InvalidAppKey)?;

        let max_length = app.max_channel_name_length.unwrap_or(200);
        if channel.len() > max_length as usize {
            return Err(Error::InvalidChannelName(format!(
                "Channel name too long. Max length is {max_length}"
            )));
        }

        // Validate channel name format using regex
        let valid_chars = regex::Regex::new(r"^[a-zA-Z0-9_\-=@,.;]+$").unwrap();
        if !valid_chars.is_match(channel) {
            return Err(Error::InvalidChannelName(
                "Channel name contains invalid characters".to_string(),
            ));
        }

        Ok(())
    }

    /// Check if client events are enabled for an app
    pub async fn can_handle_client_events(&self, app_key: &str) -> Result<bool> {
        Ok(self
            .find_by_key(app_key)
            .await?
            .map(|app| app.enable_client_messages)
            .unwrap_or(false))
    }

    /// Validate user authentication
    pub async fn validate_user_auth(&self, socket_id: &SocketId, auth: &str) -> Result<bool> {
        // Split auth string into key and signature (format: "app_key:signature")
        let parts: Vec<&str> = auth.split(':').collect();
        if parts.len() < 2 {
            return Err(Error::Auth("Invalid auth format".into()));
        }

        let app_key = parts[0];
        // Signature might contain colons (e.g., in user auth), so join the rest
        let signature = parts[1..].join(":");

        // Get app config
        let app = self
            .find_by_key(app_key)
            .await?
            .ok_or_else(|| Error::InvalidAppKey)?;

        // Create string to sign: socket_id
        let string_to_sign = format!("{socket_id}::user::{signature}");

        // Generate token
        let token = Token::new(app.key.clone(), app.secret.clone());

        // Verify
        Ok(token.verify(&string_to_sign, &signature))
    }
}

/// Row struct for SQLx query results
#[derive(sqlx::FromRow)]
struct AppRow {
    id: String,
    key: String,
    secret: String,
    max_connections: u32,
    enable_client_messages: bool,
    enabled: bool,
    max_backend_events_per_second: Option<u32>,
    max_client_events_per_second: u32,
    max_read_requests_per_second: Option<u32>,
    max_presence_members_per_channel: Option<u32>,
    max_presence_member_size_in_kb: Option<u32>,
    max_channel_name_length: Option<u32>,
    max_event_channels_at_once: Option<u32>,
    max_event_name_length: Option<u32>,
    max_event_payload_in_kb: Option<u32>,
    max_event_batch_size: Option<u32>,
    enable_user_authentication: Option<bool>,
}

impl AppRow {
    /// Convert database row to App struct
    fn into_app(self) -> App {
        App {
            id: self.id,
            key: self.key,
            secret: self.secret,
            max_connections: self.max_connections,
            enable_client_messages: self.enable_client_messages,
            enabled: self.enabled,
            max_backend_events_per_second: self.max_backend_events_per_second,
            max_client_events_per_second: self.max_client_events_per_second,
            max_read_requests_per_second: self.max_read_requests_per_second,
            max_presence_members_per_channel: self.max_presence_members_per_channel,
            max_presence_member_size_in_kb: self.max_presence_member_size_in_kb,
            max_channel_name_length: self.max_channel_name_length,
            max_event_channels_at_once: self.max_event_channels_at_once,
            max_event_name_length: self.max_event_name_length,
            max_event_payload_in_kb: self.max_event_payload_in_kb,
            max_event_batch_size: self.max_event_batch_size,
            enable_user_authentication: self.enable_user_authentication,
            webhooks: None, // Assuming webhooks are not part of the App struct
            enable_watchlist_events: None, // Assuming this is not part of the App struct
        }
    }
}

// Implement your App trait for MySQLAppManager
// This implementation will depend on how your current code is structured
#[async_trait]
impl AppManager for MySQLAppManager {
    // The basic implementation delegates to our improved methods above
    // You may need to adjust this based on your specific AppManager trait

    async fn init(&self) -> Result<()> {
        // Initialization is done in the constructor
        Ok(())
    }

    async fn create_app(&self, config: App) -> Result<()> {
        // Spawn a blocking task to avoid blocking the current thread
        let config_clone = config.clone();
        self.create_app(config_clone).await
    }

    async fn update_app(&self, config: App) -> Result<()> {
        // This calls our implementation method
        self.update_app(config).await
    }

    async fn delete_app(&self, app_id: &str) -> Result<()> {
        // This calls our implementation method
        self.delete_app(app_id).await
    }

    async fn get_apps(&self) -> Result<Vec<App>> {
        // This calls our implementation method
        self.get_apps().await
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        // For the sync interface, poll the async function in a blocking manner
        self.find_by_id(app_id).await
    }

    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        self.find_by_key(key).await
    }

    async fn check_health(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                crate::error::Error::Internal(format!("App manager MySQL connection failed: {e}"))
            })?;
        Ok(())
    }
}

// Make the MySQLAppManager clonable for use in async contexts
impl Clone for MySQLAppManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            pool: self.pool.clone(),
            app_cache: self.app_cache.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::runtime::Runtime;

    // Helper to create a test app
    fn create_test_app(id: &str) -> App {
        App {
            id: id.to_string(),
            key: format!("{id}_key"),
            secret: format!("{id}_secret"),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_backend_events_per_second: Some(1000),
            max_client_events_per_second: 100,
            max_read_requests_per_second: Some(1000),
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(10),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(10),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(100),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: None,
            enable_watchlist_events: None,
        }
    }

    #[test]
    fn test_mysql_app_manager() {
        // To run these tests, you need a MySQL database available
        // These tests are marked as ignored by default since they require a DB

        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            // Setup test database with proper credentials for Docker MySQL dev environment
            let config = DatabaseConnection {
                username: "sockudo".to_string(),
                password: "sockudo123".to_string(),
                database: "sockudo".to_string(),
                table_name: "applications".to_string(),
                cache_ttl: 5, // Short TTL for testing
                ..Default::default()
            };

            // Create manager
            let manager = MySQLAppManager::new(config).await.unwrap();

            // Test registering an app
            let test_app = create_test_app("test1");
            manager.create_app(test_app.clone()).await.unwrap();

            // Test getting an app
            let app = manager.find_by_id("test1").await.unwrap().unwrap();
            assert_eq!(app.id, "test1");
            assert_eq!(app.key, "test1_key");

            // Test getting an app by key
            let app = manager.find_by_key("test1_key").await.unwrap().unwrap();
            assert_eq!(app.id, "test1");

            // Test updating an app
            let mut updated_app = test_app.clone();
            updated_app.max_connections = 200;
            manager.update_app(updated_app).await.unwrap();

            let app = manager.find_by_id("test1").await.unwrap().unwrap();
            assert_eq!(app.max_connections, 200);

            // Test cache expiration
            tokio::time::sleep(Duration::from_secs(6)).await;

            // Add another app
            let test_app2 = create_test_app("test2");
            manager.create_app(test_app2).await.unwrap();

            // Get all apps
            let apps = manager.get_apps().await.unwrap();
            assert_eq!(apps.len(), 2);

            // Test removing an app
            manager.delete_app("test1").await.unwrap();
            assert!(manager.find_by_id("test1").await.unwrap().is_none());

            // Cleanup
            manager.delete_app("test2").await.unwrap();
        });
    }
}
