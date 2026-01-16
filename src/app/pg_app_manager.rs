use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};
use crate::options::{DatabaseConnection, DatabasePooling};
use crate::token::Token;
use crate::webhook::types::Webhook;
use crate::websocket::SocketId;
use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tracing::{debug, error, info};

/// PostgreSQL-based implementation of the AppManager
pub struct PgSQLAppManager {
    config: DatabaseConnection,
    pool: PgPool,
}

impl PgSQLAppManager {
    /// Create a new PostgreSQL-based AppManager with the provided configuration
    pub async fn new(config: DatabaseConnection, pooling: DatabasePooling) -> Result<Self> {
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

        // Create connection pool with options from config/env
        let mut opts = PgPoolOptions::new();
        opts = if pooling.enabled {
            let min = config.pool_min.unwrap_or(pooling.min);
            let max = config.pool_max.unwrap_or(pooling.max);
            opts.min_connections(min).max_connections(max)
        } else {
            // Backward-compat: only max via per-DB connection_pool_size
            opts.max_connections(config.connection_pool_size)
        };
        let pool = opts
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(180))
            .connect(&connection_string)
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to PostgreSQL: {e}")))?;

        let manager = Self { config, pool };

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
                webhooks JSONB,
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

        // Add migrations for columns that may not exist
        // PostgreSQL supports ADD COLUMN IF NOT EXISTS
        let columns_to_add = vec![("allowed_origins", "JSONB"), ("webhooks", "JSONB")];

        for (column_name, column_type) in columns_to_add {
            let add_column_query = format!(
                r#"ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {}"#,
                self.config.table_name, column_name, column_type
            );

            sqlx::query(&add_column_query)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    Error::Internal(format!(
                        "Failed to add column '{}' to table '{}': {}",
                        column_name, self.config.table_name, e
                    ))
                })?;
        }

        info!("Ensured table '{}' exists", self.config.table_name);
        Ok(())
    }

    /// Get an app by ID from database
    pub async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        debug!("Fetching app {} from database", app_id);

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
                webhooks,
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

        Ok(app_result.map(|row| row.into_app()))
    }

    /// Get an app by key from database
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
                webhooks,
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

        Ok(app_result.map(|row| row.into_app()))
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
                enable_watchlist_events, webhooks, allowed_origins
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)"#,
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
            .bind(sqlx::types::Json(&app.webhooks))
            .bind(sqlx::types::Json(&app.allowed_origins))
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Database error registering app {}: {}", app.id, e);
                Error::Internal(format!("Failed to insert app into PostgreSQL: {e}"))
            })?;

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
                webhooks = $18, allowed_origins = $19,
                updated_at = CURRENT_TIMESTAMP
                WHERE id = $20"#,
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
            .bind(sqlx::types::Json(&app.webhooks))
            .bind(sqlx::types::Json(&app.allowed_origins))
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
            webhooks,
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

        let apps: Vec<App> = app_rows.into_iter().map(|row| row.into_app()).collect();

        debug!("Fetched {} apps from database", apps.len());

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
    #[sqlx(json(nullable))]
    webhooks: Option<Vec<Webhook>>,
    #[sqlx(json(nullable))]
    allowed_origins: Option<Vec<String>>,
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
            webhooks: self.webhooks,
            enable_watchlist_events: self.enable_watchlist_events,
            allowed_origins: self.allowed_origins,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create test database config using centralized config system
    async fn get_test_db_config(table_name: &str) -> DatabaseConnection {
        let mut config = crate::options::ServerOptions::default();
        config
            .override_from_env()
            .await
            .expect("Failed to load config from env");

        let mut db_config = config.database.postgres.clone();
        db_config.table_name = table_name.to_string();
        db_config
    }

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
    async fn test_pgsql_app_manager() {
        // Setup test database
        let config = get_test_db_config("apps_test").await;

        // Create manager
        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_webhooks_serialization() {
        let config = get_test_db_config("apps_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with webhooks
        let webhook = Webhook {
            url: Some("https://example.com/webhook".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec![
                "channel_occupied".to_string(),
                "channel_vacated".to_string(),
            ],
            filter: None,
            headers: None,
        };

        let mut app = create_test_app("webhook_test");
        app.webhooks = Some(vec![webhook.clone()]);

        manager.create_app(app.clone()).await.unwrap();

        // Retrieve and verify webhooks
        let retrieved = manager.find_by_id("webhook_test").await.unwrap().unwrap();
        assert!(retrieved.webhooks.is_some());
        let webhooks = retrieved.webhooks.unwrap();
        assert_eq!(webhooks.len(), 1);
        assert_eq!(webhooks[0].event_types, webhook.event_types);
        assert_eq!(webhooks[0].url, webhook.url);

        // Cleanup
        manager.delete_app("webhook_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_multiple_webhooks() {
        let config = get_test_db_config("apps_multi_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with multiple webhooks
        let webhook1 = Webhook {
            url: Some("https://example.com/webhook1".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        };

        let webhook2 = Webhook {
            url: Some("https://example.com/webhook2".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["member_added".to_string(), "member_removed".to_string()],
            filter: None,
            headers: None,
        };

        let mut app = create_test_app("multi_webhook_test");
        app.webhooks = Some(vec![webhook1, webhook2]);

        manager.create_app(app.clone()).await.unwrap();

        // Retrieve and verify
        let retrieved = manager
            .find_by_id("multi_webhook_test")
            .await
            .unwrap()
            .unwrap();
        assert!(retrieved.webhooks.is_some());
        assert_eq!(retrieved.webhooks.unwrap().len(), 2);

        // Cleanup
        manager.delete_app("multi_webhook_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_watchlist_events() {
        let config = get_test_db_config("apps_watchlist_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Test with watchlist enabled
        let mut app1 = create_test_app("watchlist_enabled");
        app1.enable_watchlist_events = Some(true);
        manager.create_app(app1).await.unwrap();

        let retrieved1 = manager
            .find_by_id("watchlist_enabled")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved1.enable_watchlist_events, Some(true));

        // Test with watchlist disabled
        let mut app2 = create_test_app("watchlist_disabled");
        app2.enable_watchlist_events = Some(false);
        manager.create_app(app2).await.unwrap();

        let retrieved2 = manager
            .find_by_id("watchlist_disabled")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved2.enable_watchlist_events, Some(false));

        // Test with watchlist unset (None)
        let app3 = create_test_app("watchlist_none");
        manager.create_app(app3).await.unwrap();

        let retrieved3 = manager.find_by_id("watchlist_none").await.unwrap().unwrap();
        assert_eq!(retrieved3.enable_watchlist_events, None);

        // Cleanup
        manager.delete_app("watchlist_enabled").await.unwrap();
        manager.delete_app("watchlist_disabled").await.unwrap();
        manager.delete_app("watchlist_none").await.unwrap();
    }

    #[tokio::test]
    async fn test_allowed_origins() {
        let config = get_test_db_config("apps_origins_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Test with allowed origins
        let mut app = create_test_app("origins_test");
        app.allowed_origins = Some(vec![
            "https://example.com".to_string(),
            "https://*.example.com".to_string(),
            "http://localhost:3000".to_string(),
        ]);
        manager.create_app(app.clone()).await.unwrap();

        let retrieved = manager.find_by_id("origins_test").await.unwrap().unwrap();
        assert!(retrieved.allowed_origins.is_some());
        let origins = retrieved.allowed_origins.unwrap();
        assert_eq!(origins.len(), 3);
        assert!(origins.contains(&"https://example.com".to_string()));
        assert!(origins.contains(&"https://*.example.com".to_string()));
        assert!(origins.contains(&"http://localhost:3000".to_string()));

        // Cleanup
        manager.delete_app("origins_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_update_webhooks() {
        let config = get_test_db_config("apps_update_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app without webhooks
        let app = create_test_app("update_webhooks");
        manager.create_app(app).await.unwrap();

        // Update to add webhooks
        let webhook = Webhook {
            url: Some("https://example.com/new-webhook".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        };

        let mut updated_app = create_test_app("update_webhooks");
        updated_app.webhooks = Some(vec![webhook]);
        updated_app.enable_watchlist_events = Some(true);
        manager.update_app(updated_app).await.unwrap();

        // Verify update
        let retrieved = manager
            .find_by_id("update_webhooks")
            .await
            .unwrap()
            .unwrap();
        assert!(retrieved.webhooks.is_some());
        assert_eq!(retrieved.webhooks.unwrap().len(), 1);
        assert_eq!(retrieved.enable_watchlist_events, Some(true));

        // Cleanup
        manager.delete_app("update_webhooks").await.unwrap();
    }

    #[tokio::test]
    async fn test_find_by_key_with_webhooks() {
        let config = get_test_db_config("apps_key_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with webhooks
        let webhook = Webhook {
            url: Some("https://example.com/webhook".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        };

        let mut app = create_test_app("key_test");
        app.webhooks = Some(vec![webhook]);
        manager.create_app(app).await.unwrap();

        // Find by key and verify webhooks are included
        let retrieved = manager.find_by_key("key_test_key").await.unwrap().unwrap();
        assert_eq!(retrieved.id, "key_test");
        assert!(retrieved.webhooks.is_some());
        assert_eq!(retrieved.webhooks.unwrap().len(), 1);

        // Cleanup
        manager.delete_app("key_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_get_all_apps_with_webhooks() {
        let config = get_test_db_config("apps_all_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create multiple apps with different webhook configurations
        let webhook1 = Webhook {
            url: Some("https://example.com/webhook1".parse().unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        };

        let mut app1 = create_test_app("all_apps_1");
        app1.webhooks = Some(vec![webhook1]);
        manager.create_app(app1).await.unwrap();

        let mut app2 = create_test_app("all_apps_2");
        app2.enable_watchlist_events = Some(true);
        manager.create_app(app2).await.unwrap();

        let app3 = create_test_app("all_apps_3");
        manager.create_app(app3).await.unwrap();

        // Get all apps
        let all_apps = manager.get_apps().await.unwrap();
        assert!(all_apps.len() >= 3);

        // Verify each app has correct data
        let app1_found = all_apps.iter().find(|a| a.id == "all_apps_1");
        assert!(app1_found.is_some());
        assert!(app1_found.unwrap().webhooks.is_some());

        let app2_found = all_apps.iter().find(|a| a.id == "all_apps_2");
        assert!(app2_found.is_some());
        assert_eq!(app2_found.unwrap().enable_watchlist_events, Some(true));

        // Cleanup
        manager.delete_app("all_apps_1").await.unwrap();
        manager.delete_app("all_apps_2").await.unwrap();
        manager.delete_app("all_apps_3").await.unwrap();
    }

    #[tokio::test]
    async fn test_health_check() {
        let config = get_test_db_config("apps_health_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Health check should succeed
        let result = manager.check_health().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_app() {
        let config = get_test_db_config("apps_delete_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Try to delete non-existent app
        let result = manager.delete_app("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_nonexistent_app() {
        let config = get_test_db_config("apps_update_fail_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Try to update non-existent app
        let app = create_test_app("nonexistent");
        let result = manager.update_app(app).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_null_values() {
        let config = get_test_db_config("apps_null_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with all optional fields as None
        let mut app = create_test_app("null_test");
        app.webhooks = None;
        app.enable_watchlist_events = None;
        app.allowed_origins = None;
        app.max_backend_events_per_second = None;
        app.max_read_requests_per_second = None;
        app.enable_user_authentication = None;

        manager.create_app(app).await.unwrap();

        // Retrieve and verify None values are handled correctly
        let retrieved = manager.find_by_id("null_test").await.unwrap().unwrap();
        assert_eq!(retrieved.webhooks, None);
        assert_eq!(retrieved.enable_watchlist_events, None);
        assert_eq!(retrieved.allowed_origins, None);
        assert_eq!(retrieved.max_backend_events_per_second, None);

        // Cleanup
        manager.delete_app("null_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_empty_webhooks_array() {
        let config = get_test_db_config("apps_empty_webhooks_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with empty webhooks array
        let mut app = create_test_app("empty_webhooks");
        app.webhooks = Some(vec![]);
        manager.create_app(app).await.unwrap();

        // Retrieve and verify
        let retrieved = manager.find_by_id("empty_webhooks").await.unwrap().unwrap();
        assert!(retrieved.webhooks.is_some());
        assert_eq!(retrieved.webhooks.unwrap().len(), 0);

        // Cleanup
        manager.delete_app("empty_webhooks").await.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_with_lambda_config() {
        let config = get_test_db_config("apps_lambda_test").await;

        let manager = PgSQLAppManager::new(config, DatabasePooling::default())
            .await
            .unwrap();

        // Create app with Lambda webhook
        let webhook = Webhook {
            url: None,
            lambda_function: None,
            lambda: Some(crate::webhook::types::LambdaConfig {
                function_name: "my-webhook-function".to_string(),
                region: "us-east-1".to_string(),
            }),
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        };

        let mut app = create_test_app("lambda_test");
        app.webhooks = Some(vec![webhook]);
        manager.create_app(app).await.unwrap();

        // Retrieve and verify Lambda config
        let retrieved = manager.find_by_id("lambda_test").await.unwrap().unwrap();
        assert!(retrieved.webhooks.is_some());
        let webhooks = retrieved.webhooks.unwrap();
        assert_eq!(webhooks.len(), 1);
        assert!(webhooks[0].lambda.is_some());
        assert_eq!(
            webhooks[0].lambda.as_ref().unwrap().function_name,
            "my-webhook-function"
        );

        // Cleanup
        manager.delete_app("lambda_test").await.unwrap();
    }
}
