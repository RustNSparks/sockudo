use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};
use crate::options::DatabaseConnection;
use crate::token::Token;
use crate::websocket::SocketId;
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use moka::future::Cache;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// PostgreSQL-based implementation of the AppManager
pub struct PgSQLAppManager {
    config: DatabaseConnection,
    pool: PgPool,
    app_cache: Cache<String, App>, // App ID -> App
}

impl PgSQLAppManager {
    /// Create a new PostgreSQL-based AppManager with the provided configuration
    pub async fn new(config: DatabaseConnection) -> Result<Self> {
        info!(
            "Initializing PostgreSQL AppManager with database {}",
            config.database
        );

        // Build connection string with proper URL encoding for password
        let password = urlencoding::encode(&config.password);
        let connection_string = format!(
            "postgresql://{}:{}@{}:{}/{}",
            config.username, password, config.host, config.port, config.database
        );

        // Create connection pool with improved options
        let pool = PgPoolOptions::new()
            .max_connections(config.connection_pool_size)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(180))
            .connect(&connection_string)
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to PostgreSQL: {e}")))?;

        // Initialize cache
        let app_cache = Cache::builder()
            .time_to_live(Duration::from_secs(config.cache_ttl))
            .max_capacity(config.cache_max_capacity)
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
        // PostgreSQL table creation query
        let create_table_query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                id VARCHAR(255) PRIMARY KEY,
                key VARCHAR(255) UNIQUE NOT NULL,
                secret VARCHAR(255) NOT NULL,
                max_connections INTEGER NOT NULL,
                enable_client_messages BOOLEAN NOT NULL DEFAULT FALSE,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                max_backend_events_per_second INTEGER,
                max_client_events_per_second INTEGER NOT NULL,
                max_read_requests_per_second INTEGER,
                max_presence_members_per_channel INTEGER,
                max_presence_member_size_in_kb INTEGER,
                max_channel_name_length INTEGER,
                max_event_channels_at_once INTEGER,
                max_event_name_length INTEGER,
                max_event_payload_in_kb INTEGER,
                max_event_batch_size INTEGER,
                enable_user_authentication BOOLEAN,
                enable_watchlist_events BOOLEAN,
                allowed_origins JSONB,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        "#,
            self.config.table_name
        );

        sqlx::query(&create_table_query)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create PostgreSQL table: {e}")))?;

        // Add migration for allowed_origins column if it doesn't exist
        let add_column_query = format!(
            r#"ALTER TABLE {} ADD COLUMN IF NOT EXISTS allowed_origins JSONB"#,
            self.config.table_name
        );

        if let Err(e) = sqlx::query(&add_column_query).execute(&self.pool).await {
            warn!(
                "Could not add allowed_origins column (may already exist): {}",
                e
            );
        }

        info!("Ensured table '{}' exists", self.config.table_name);
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

        // Create the query with the correct table name
        let query = format!(
            r#"SELECT
                id, key, secret, max_connections,
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
                enable_watchlist_events,
                allowed_origins
            FROM {} WHERE id = $1"#,
            self.config.table_name
        );

        let app_result = sqlx::query_as::<_, AppRow>(&query)
            .bind(app_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error fetching app {}: {}", app_id, e);
                Error::Internal(format!("Failed to fetch app from PostgreSQL: {e}"))
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
        // For PostgreSQL, we need to query by key since cache is by ID
        debug!("Fetching app by key {} from database", key);

        let query = format!(
            r#"SELECT
                id, key, secret, max_connections,
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
                enable_watchlist_events,
                allowed_origins
            FROM {} WHERE key = $1"#,
            self.config.table_name
        );

        let app_result = sqlx::query_as::<_, AppRow>(&query)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error fetching app by key {}: {}", key, e);
                Error::Internal(format!("Failed to fetch app from PostgreSQL: {e}"))
            })?;

        if let Some(app_row) = app_result {
            let app = app_row.into_app();

            // Update cache with this app using ID as key
            self.app_cache.insert(app.id.clone(), app.clone()).await;

            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    /// Register a new app in the database
    pub async fn create_app(&self, app: App) -> Result<()> {
        info!("Registering new app: {}", app.id);

        // Prepare the query with proper table name and PostgreSQL syntax
        let query = format!(
            r#"INSERT INTO {} (
                id, key, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size, enable_user_authentication,
                enable_watchlist_events
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
            self.config.table_name
        );

        sqlx::query(&query)
            .bind(&app.id)
            .bind(&app.key)
            .bind(&app.secret)
            .bind(app.max_connections as i32)
            .bind(app.enable_client_messages)
            .bind(app.enabled)
            .bind(app.max_backend_events_per_second.map(|v| v as i32))
            .bind(app.max_client_events_per_second as i32)
            .bind(app.max_read_requests_per_second.map(|v| v as i32))
            .bind(app.max_presence_members_per_channel.map(|v| v as i32))
            .bind(app.max_presence_member_size_in_kb.map(|v| v as i32))
            .bind(app.max_channel_name_length.map(|v| v as i32))
            .bind(app.max_event_channels_at_once.map(|v| v as i32))
            .bind(app.max_event_name_length.map(|v| v as i32))
            .bind(app.max_event_payload_in_kb.map(|v| v as i32))
            .bind(app.max_event_batch_size.map(|v| v as i32))
            .bind(app.enable_user_authentication)
            .bind(app.enable_watchlist_events)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error registering app {}: {}", app.id, e);
                Error::Internal(format!("Failed to insert app into PostgreSQL: {e}"))
            })?;

        // Update cache
        self.app_cache.insert(app.id.clone(), app).await;

        Ok(())
    }

    /// Update an existing app in the database
    pub async fn update_app(&self, app: App) -> Result<()> {
        info!("Updating app: {}", app.id);

        // Prepare the query with proper table name and PostgreSQL syntax
        let query = format!(
            r#"UPDATE {} SET
                key = $1, secret = $2, max_connections = $3, enable_client_messages = $4, enabled = $5,
                max_backend_events_per_second = $6, max_client_events_per_second = $7,
                max_read_requests_per_second = $8, max_presence_members_per_channel = $9,
                max_presence_member_size_in_kb = $10, max_channel_name_length = $11,
                max_event_channels_at_once = $12, max_event_name_length = $13,
                max_event_payload_in_kb = $14, max_event_batch_size = $15, 
                enable_user_authentication = $16, enable_watchlist_events = $17,
                updated_at = CURRENT_TIMESTAMP
                WHERE id = $18"#,
            self.config.table_name
        );

        let result = sqlx::query(&query)
            .bind(&app.key)
            .bind(&app.secret)
            .bind(app.max_connections as i32)
            .bind(app.enable_client_messages)
            .bind(app.enabled)
            .bind(app.max_backend_events_per_second.map(|v| v as i32))
            .bind(app.max_client_events_per_second as i32)
            .bind(app.max_read_requests_per_second.map(|v| v as i32))
            .bind(app.max_presence_members_per_channel.map(|v| v as i32))
            .bind(app.max_presence_member_size_in_kb.map(|v| v as i32))
            .bind(app.max_channel_name_length.map(|v| v as i32))
            .bind(app.max_event_channels_at_once.map(|v| v as i32))
            .bind(app.max_event_name_length.map(|v| v as i32))
            .bind(app.max_event_payload_in_kb.map(|v| v as i32))
            .bind(app.max_event_batch_size.map(|v| v as i32))
            .bind(app.enable_user_authentication)
            .bind(app.enable_watchlist_events)
            .bind(&app.id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error updating app {}: {}", app.id, e);
                Error::Internal(format!("Failed to update app in PostgreSQL: {e}"))
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
        info!("Removing app: {}", app_id);

        // Prepare the query with proper table name and PostgreSQL syntax
        let query = format!("DELETE FROM {} WHERE id = $1", self.config.table_name);

        let result = sqlx::query(&query)
            .bind(app_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error removing app {}: {}", app_id, e);
                Error::Internal(format!("Failed to delete app from PostgreSQL: {e}"))
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
        info!("Fetching all apps from database");

        let query = format!(
            r#"SELECT
            id, key, secret, max_connections,
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
            enable_watchlist_events,
            allowed_origins
        FROM {}"#,
            self.config.table_name
        );

        // Fetch all rows from the database
        let app_rows = sqlx::query_as::<_, AppRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error fetching all apps: {}", e);
                Error::Internal(format!("Failed to fetch apps from PostgreSQL: {e}"))
            })?;

        warn!("Fetched {} app rows from database.", app_rows.len());

        // Process rows concurrently using streams
        let apps = stream::iter(app_rows)
            .map(|row| async {
                let app = row.into_app();
                // Insert into cache
                self.app_cache.insert(app.id.clone(), app.clone()).await;
                app
            })
            .buffer_unordered(self.config.connection_pool_size as usize)
            .collect::<Vec<App>>()
            .await;

        info!("Finished processing and caching {} apps.", apps.len());

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
    max_connections: i32,
    enable_client_messages: bool,
    enabled: bool,
    max_backend_events_per_second: Option<i32>,
    max_client_events_per_second: i32,
    max_read_requests_per_second: Option<i32>,
    max_presence_members_per_channel: Option<i32>,
    max_presence_member_size_in_kb: Option<i32>,
    max_channel_name_length: Option<i32>,
    max_event_channels_at_once: Option<i32>,
    max_event_name_length: Option<i32>,
    max_event_payload_in_kb: Option<i32>,
    max_event_batch_size: Option<i32>,
    enable_user_authentication: Option<bool>,
    enable_watchlist_events: Option<bool>,
    allowed_origins: Option<String>, // Stored as JSON string
}

impl AppRow {
    /// Convert database row to App struct
    fn into_app(self) -> App {
        App {
            id: self.id,
            key: self.key,
            secret: self.secret,
            max_connections: self.max_connections as u32,
            enable_client_messages: self.enable_client_messages,
            enabled: self.enabled,
            max_backend_events_per_second: self.max_backend_events_per_second.map(|v| v as u32),
            max_client_events_per_second: self.max_client_events_per_second as u32,
            max_read_requests_per_second: self.max_read_requests_per_second.map(|v| v as u32),
            max_presence_members_per_channel: self
                .max_presence_members_per_channel
                .map(|v| v as u32),
            max_presence_member_size_in_kb: self.max_presence_member_size_in_kb.map(|v| v as u32),
            max_channel_name_length: self.max_channel_name_length.map(|v| v as u32),
            max_event_channels_at_once: self.max_event_channels_at_once.map(|v| v as u32),
            max_event_name_length: self.max_event_name_length.map(|v| v as u32),
            max_event_payload_in_kb: self.max_event_payload_in_kb.map(|v| v as u32),
            max_event_batch_size: self.max_event_batch_size.map(|v| v as u32),
            enable_user_authentication: self.enable_user_authentication,
            webhooks: None,
            enable_watchlist_events: self.enable_watchlist_events,
            allowed_origins: self
                .allowed_origins
                .and_then(|json_str| sonic_rs::from_str::<Vec<String>>(&json_str).ok()),
        }
    }
}

#[async_trait]
impl AppManager for PgSQLAppManager {
    async fn init(&self) -> Result<()> {
        // Initialization is done in the constructor
        Ok(())
    }

    async fn create_app(&self, config: App) -> Result<()> {
        self.create_app(config).await
    }

    async fn update_app(&self, config: App) -> Result<()> {
        self.update_app(config).await
    }

    async fn delete_app(&self, app_id: &str) -> Result<()> {
        self.delete_app(app_id).await
    }

    async fn get_apps(&self) -> Result<Vec<App>> {
        self.get_apps().await
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
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
                crate::error::Error::Internal(format!(
                    "App manager PostgreSQL connection failed: {e}"
                ))
            })?;
        Ok(())
    }
}

impl Clone for PgSQLAppManager {
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
            allowed_origins: None,
        }
    }

    #[tokio::test]
    #[ignore] // Requires PostgreSQL database
    async fn test_pgsql_app_manager() {
        // Setup test database
        let config = DatabaseConnection {
            database: "sockudo_test".to_string(),
            table_name: "apps_test".to_string(),
            cache_ttl: 5, // Short TTL for testing
            ..Default::default()
        };

        // Create manager
        let manager = PgSQLAppManager::new(config).await.unwrap();

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
    }
}
