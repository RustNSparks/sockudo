// src/app/surrealdb_app_manager.rs
use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};
use crate::token::Token;
use crate::websocket::SocketId;
use async_trait::async_trait;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::{RecordId, Surreal};
use tracing::{debug, error, info, warn};

/// Configuration for SurrealDB AppManager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurrealDbConfig {
    /// SurrealDB connection URL (e.g., "ws://localhost:8000")
    pub url: String,
    /// Database namespace
    pub namespace: String,
    /// Database name
    pub database: String,
    /// Root username for authentication
    pub username: Option<String>,
    /// Root password for authentication
    pub password: Option<String>,
    /// Table name for applications (default: "applications")
    pub table_name: String,
    /// Cache TTL in seconds
    pub cache_ttl: u64,
    /// Maximum cache capacity
    pub cache_max_capacity: u64,
}

impl Default for SurrealDbConfig {
    fn default() -> Self {
        Self {
            url: "ws://localhost:8000".to_string(),
            namespace: "sockudo".to_string(),
            database: "sockudo".to_string(),
            username: Some("root".to_string()),
            password: Some("root".to_string()),
            table_name: "applications".to_string(),
            cache_ttl: 300, // 5 minutes
            cache_max_capacity: 1000,
        }
    }
}

/// SurrealDB record structure for applications
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppRecord {
    id: RecordId,
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
    enable_watchlist_events: Option<bool>,
    allowed_origins: Option<Vec<String>>,
}

impl AppRecord {
    /// Convert AppRecord to App struct
    fn into_app(self) -> App {
        let id_str = self.id.to_string();
        App {
            id: id_str,
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
            webhooks: None,
            enable_watchlist_events: self.enable_watchlist_events,
            allowed_origins: self.allowed_origins,
        }
    }
}

impl From<&App> for AppRecord {
    fn from(app: &App) -> Self {
        Self {
            id: RecordId::from_table_key(&app.id, &app.id),
            key: app.key.clone(),
            secret: app.secret.clone(),
            max_connections: app.max_connections,
            enable_client_messages: app.enable_client_messages,
            enabled: app.enabled,
            max_backend_events_per_second: app.max_backend_events_per_second,
            max_client_events_per_second: app.max_client_events_per_second,
            max_read_requests_per_second: app.max_read_requests_per_second,
            max_presence_members_per_channel: app.max_presence_members_per_channel,
            max_presence_member_size_in_kb: app.max_presence_member_size_in_kb,
            max_channel_name_length: app.max_channel_name_length,
            max_event_channels_at_once: app.max_event_channels_at_once,
            max_event_name_length: app.max_event_name_length,
            max_event_payload_in_kb: app.max_event_payload_in_kb,
            max_event_batch_size: app.max_event_batch_size,
            enable_user_authentication: app.enable_user_authentication,
            enable_watchlist_events: app.enable_watchlist_events,
            allowed_origins: app.allowed_origins.clone(),
        }
    }
}

/// SurrealDB-based implementation of the AppManager
pub struct SurrealDbAppManager {
    config: SurrealDbConfig,
    db: Surreal<Client>,
    app_cache: Cache<String, App>, // App ID -> App
}

impl SurrealDbAppManager {
    /// Create a new SurrealDB-based AppManager with the provided configuration
    pub async fn new(config: SurrealDbConfig) -> Result<Self> {
        info!("Initializing SurrealDB AppManager with URL: {}", config.url);

        // Connect to SurrealDB
        let db = Surreal::new::<Ws>(config.url.clone())
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to SurrealDB: {e}")))?;

        // Authenticate if credentials are provided
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            db.signin(Root {
                username: username,
                password: password,
            })
            .await
            .map_err(|e| Error::Internal(format!("Failed to authenticate with SurrealDB: {e}")))?;
        }

        // Use the specified namespace and database
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .map_err(|e| {
                Error::Internal(format!(
                    "Failed to use namespace/database {}/{}: {e}",
                    config.namespace, config.database
                ))
            })?;

        // Initialize cache
        let app_cache = Cache::builder()
            .time_to_live(Duration::from_secs(config.cache_ttl))
            .max_capacity(config.cache_max_capacity)
            .build();

        let manager = Self {
            config,
            db,
            app_cache,
        };

        // Ensure proper schema/indexes exist
        manager.ensure_schema().await?;

        Ok(manager)
    }

    /// Ensure proper schema and indexes exist for the applications table
    async fn ensure_schema(&self) -> Result<()> {
        info!("Setting up SurrealDB schema for applications");

        // Create unique index on the key field for fast lookups
        let index_query = format!(
            "DEFINE INDEX unique_key ON TABLE {} COLUMNS key UNIQUE",
            self.config.table_name
        );

        self.db
            .query(index_query)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create unique key index: {e}")))?;

        // Create index on enabled field for filtering
        let enabled_index_query = format!(
            "DEFINE INDEX enabled_idx ON TABLE {} COLUMNS enabled",
            self.config.table_name
        );

        self.db
            .query(enabled_index_query)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create enabled index: {e}")))?;

        info!("SurrealDB schema setup completed");
        Ok(())
    }

    /// Get all apps from the database
    pub async fn get_apps(&self) -> Result<Vec<App>> {
        debug!("Fetching all apps from SurrealDB");

        let query = format!("SELECT * FROM {}", self.config.table_name);

        let mut response =
            self.db.query(query).await.map_err(|e| {
                Error::Internal(format!("Failed to fetch apps from SurrealDB: {e}"))
            })?;

        let app_records: Vec<AppRecord> = response
            .take(0)
            .map_err(|e| Error::Internal(format!("Failed to parse app records: {e}")))?;

        let apps: Vec<App> = app_records
            .into_iter()
            .map(|record| record.into_app())
            .collect();

        // Update cache for all apps
        for app in &apps {
            self.app_cache.insert(app.id.clone(), app.clone()).await;
        }

        info!("Retrieved {} apps from SurrealDB", apps.len());
        Ok(apps)
    }

    /// Get an app by ID from cache or database
    pub async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        // Check cache first
        if let Some(app) = self.app_cache.get(app_id).await {
            return Ok(Some(app));
        }

        // Not found in cache, query database
        debug!("Cache miss for app ID {}, fetching from SurrealDB", app_id);

        let record_id = RecordId::from_table_key(&self.config.table_name, app_id);

        let app_record: Option<AppRecord> = self
            .db
            .select(record_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch app from SurrealDB: {e}")))?;

        if let Some(record) = app_record {
            let app = record.into_app();
            // Update cache
            self.app_cache.insert(app_id.to_string(), app.clone()).await;
            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    /// Get an app by key from cache or database
    pub async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        // Check cache first - we need to check if any cached app has this key
        // This is less efficient than caching by key, but maintains consistency with other implementations

        // Query database directly since we don't cache by key
        debug!("Fetching app by key {} from SurrealDB", key);

        let query = format!(
            "SELECT * FROM {} WHERE key = $key LIMIT 1",
            self.config.table_name
        );
        let key_ref = key.to_string();
        let mut response = self
            .db
            .query(query)
            .bind(("key", key_ref))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to fetch app by key from SurrealDB: {e}"))
            })?;

        let app_records: Vec<AppRecord> = response
            .take(0)
            .map_err(|e| Error::Internal(format!("Failed to parse app record: {e}")))?;

        if let Some(record) = app_records.into_iter().next() {
            let app = record.into_app();
            // Update cache with this app
            self.app_cache.insert(app.id.clone(), app.clone()).await;
            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    /// Register a new app in the database
    pub async fn create_app(&self, app: App) -> Result<()> {
        info!("Registering new app: {}", app.id);

        let app_record = AppRecord::from(&app);
        let record_id = RecordId::from_table_key(&self.config.table_name, &app.id);

        let _: Option<AppRecord> = self
            .db
            .create(record_id)
            .content(app_record)
            .await
            .map_err(|e| {
                error!("Database error creating app {}: {}", app.id, e);
                Error::Internal(format!("Failed to create app in SurrealDB: {e}"))
            })?;

        // Update cache
        self.app_cache.insert(app.id.clone(), app).await;

        Ok(())
    }

    /// Update an existing app in the database
    pub async fn update_app(&self, app: App) -> Result<()> {
        info!("Updating app: {}", app.id);

        let app_record = AppRecord::from(&app);
        let record_id = RecordId::from_table_key(&self.config.table_name, &app.id);

        let updated: Option<AppRecord> = self
            .db
            .update(record_id)
            .content(app_record)
            .await
            .map_err(|e| {
                error!("Database error updating app {}: {}", app.id, e);
                Error::Internal(format!("Failed to update app in SurrealDB: {e}"))
            })?;

        if updated.is_none() {
            return Err(Error::InvalidAppKey);
        }

        // Update cache
        self.app_cache.insert(app.id.clone(), app).await;

        Ok(())
    }

    /// Remove an app from the database
    pub async fn delete_app(&self, app_id: &str) -> Result<()> {
        info!("Removing app: {}", app_id);

        let record_id = RecordId::from_table_key(&self.config.table_name, app_id);

        let deleted: Option<AppRecord> = self.db.delete(record_id).await.map_err(|e| {
            error!("Database error deleting app {}: {}", app_id, e);
            Error::Internal(format!("Failed to delete app from SurrealDB: {e}"))
        })?;

        if deleted.is_none() {
            return Err(Error::InvalidAppKey);
        }

        // Remove from cache
        self.app_cache.remove(app_id).await;

        Ok(())
    }

    /// Verify a user authentication signature for the given app and socket
    pub async fn verify_user_signature(
        &self,
        app_id: &str,
        socket_id: &SocketId,
        signature: &str,
    ) -> Result<bool> {
        let app = self
            .find_by_id(app_id)
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

// Implement AppManager trait for SurrealDbAppManager
#[async_trait]
impl AppManager for SurrealDbAppManager {
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

    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        self.find_by_key(key).await
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        self.find_by_id(app_id).await
    }

    async fn check_health(&self) -> Result<()> {
        // Simple health check by querying database info
        self.db.query("INFO FOR DB").await.map_err(|e| {
            Error::Internal(format!("SurrealDB connection health check failed: {e}"))
        })?;
        Ok(())
    }
}

// Make the SurrealDbAppManager clonable for use in async contexts
impl Clone for SurrealDbAppManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            db: self.db.clone(),
            app_cache: self.app_cache.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test app
    fn create_test_app(id: &str) -> App {
        App {
            id: id.to_string(),
            key: format!("{id}_key"),
            secret: format!("{id}_secret"),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_backend_events_per_second: Some(50),
            max_client_events_per_second: 50,
            max_read_requests_per_second: Some(100),
            max_presence_members_per_channel: Some(10),
            max_presence_member_size_in_kb: Some(2),
            max_channel_name_length: Some(100),
            max_event_channels_at_once: Some(5),
            max_event_name_length: Some(50),
            max_event_payload_in_kb: Some(32),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(false),
            webhooks: None,
            enable_watchlist_events: Some(false),
            allowed_origins: Some(vec!["http://localhost:3000".to_string()]),
        }
    }

    #[tokio::test]
    #[ignore] // Only run when SurrealDB is available
    async fn test_surrealdb_app_manager() {
        let config = SurrealDbConfig::default();
        let manager = SurrealDbAppManager::new(config)
            .await
            .expect("Failed to create manager");

        let test_app = create_test_app("test_app_1");

        // Test create
        manager
            .create_app(test_app.clone())
            .await
            .expect("Failed to create app");

        // Test find by ID
        let found_app = manager
            .find_by_id("test_app_1")
            .await
            .expect("Failed to find app");
        assert!(found_app.is_some());
        assert_eq!(found_app.unwrap().id, "test_app_1");

        // Test find by key
        let found_app = manager
            .find_by_key("test_app_1_key")
            .await
            .expect("Failed to find app by key");
        assert!(found_app.is_some());
        assert_eq!(found_app.unwrap().key, "test_app_1_key");

        // Test update
        let mut updated_app = test_app.clone();
        updated_app.max_connections = 200;
        manager
            .update_app(updated_app)
            .await
            .expect("Failed to update app");

        let found_app = manager
            .find_by_id("test_app_1")
            .await
            .expect("Failed to find updated app");
        assert_eq!(found_app.unwrap().max_connections, 200);

        // Test delete
        manager
            .delete_app("test_app_1")
            .await
            .expect("Failed to delete app");
        let found_app = manager
            .find_by_id("test_app_1")
            .await
            .expect("Failed to check deleted app");
        assert!(found_app.is_none());
    }
}
