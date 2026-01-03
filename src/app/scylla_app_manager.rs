use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};
use crate::webhook::types::Webhook;
use async_trait::async_trait;
use futures::TryStreamExt;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use scylla::statement::prepared::PreparedStatement;
use scylla::{DeserializeRow, SerializeRow};
use std::sync::Arc;
use tracing::{debug, error, info};

/// Configuration for ScyllaDB App Manager
#[derive(Debug, Clone)]
pub struct ScyllaDbConfig {
    pub nodes: Vec<String>,
    pub keyspace: String,
    pub table_name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub replication_class: String,
    pub replication_factor: u32,
}

impl Default for ScyllaDbConfig {
    fn default() -> Self {
        Self {
            nodes: vec!["127.0.0.1:9042".to_string()],
            keyspace: "sockudo".to_string(),
            table_name: "applications".to_string(),
            username: None,
            password: None,
            replication_class: "SimpleStrategy".to_string(),
            replication_factor: 3,
        }
    }
}

/// ScyllaDB-based implementation of the AppManager
pub struct ScyllaDbAppManager {
    config: ScyllaDbConfig,
    session: Arc<Session>,
    insert_stmt: Arc<PreparedStatement>,
    update_stmt: Arc<PreparedStatement>,
    delete_stmt: Arc<PreparedStatement>,
}

impl ScyllaDbAppManager {
    /// Create a new ScyllaDB-based AppManager with the provided configuration
    pub async fn new(config: ScyllaDbConfig) -> Result<Self> {
        info!(
            "Initializing ScyllaDB AppManager with nodes: {:?}",
            config.nodes
        );

        // Build session
        let mut builder = SessionBuilder::new().known_nodes(&config.nodes);

        // Add authentication if provided
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            builder = builder.user(username, password);
        }

        let session = builder
            .build()
            .await
            .map_err(|e| Error::Internal(format!("Failed to connect to ScyllaDB cluster: {e}")))?;

        let session = Arc::new(session);

        // Ensure keyspace and table exist before preparing statements
        Self::ensure_keyspace_and_table(&session, &config).await?;

        // Prepare statements once for reuse
        let insert_query = format!(
            r#"INSERT INTO {}.{} (
                id, key, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size,
                enable_user_authentication, enable_watchlist_events,
                webhooks, allowed_origins,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, toTimestamp(now()), toTimestamp(now()))"#,
            config.keyspace, config.table_name
        );

        let update_query = format!(
            r#"UPDATE {}.{} SET
                key = ?, secret = ?, max_connections = ?, enable_client_messages = ?, enabled = ?,
                max_backend_events_per_second = ?, max_client_events_per_second = ?,
                max_read_requests_per_second = ?, max_presence_members_per_channel = ?,
                max_presence_member_size_in_kb = ?, max_channel_name_length = ?,
                max_event_channels_at_once = ?, max_event_name_length = ?,
                max_event_payload_in_kb = ?, max_event_batch_size = ?,
                enable_user_authentication = ?, enable_watchlist_events = ?,
                webhooks = ?, allowed_origins = ?,
                updated_at = toTimestamp(now())
            WHERE id = ?"#,
            config.keyspace, config.table_name
        );

        let delete_query = format!(
            r#"DELETE FROM {}.{} WHERE id = ?"#,
            config.keyspace, config.table_name
        );

        let insert_stmt = session
            .prepare(insert_query)
            .await
            .map_err(|e| Error::Internal(format!("Failed to prepare insert statement: {e}")))?;

        let update_stmt = session
            .prepare(update_query)
            .await
            .map_err(|e| Error::Internal(format!("Failed to prepare update statement: {e}")))?;

        let delete_stmt = session
            .prepare(delete_query)
            .await
            .map_err(|e| Error::Internal(format!("Failed to prepare delete statement: {e}")))?;

        Ok(Self {
            config,
            session,
            insert_stmt: Arc::new(insert_stmt),
            update_stmt: Arc::new(update_stmt),
            delete_stmt: Arc::new(delete_stmt),
        })
    }

    /// Create the keyspace and applications table if they don't exist
    async fn ensure_keyspace_and_table(
        session: &Arc<Session>,
        config: &ScyllaDbConfig,
    ) -> Result<()> {
        // Create keyspace if it doesn't exist
        let create_keyspace_query = format!(
            r#"CREATE KEYSPACE IF NOT EXISTS {}
               WITH replication = {{'class': '{}', 'replication_factor': {}}}"#,
            config.keyspace, config.replication_class, config.replication_factor
        );

        session
            .query_unpaged(create_keyspace_query, &[])
            .await
            .map_err(|e| Error::Internal(format!("Failed to create keyspace: {e}")))?;

        info!("Ensured keyspace '{}' exists", config.keyspace);

        // Create table if it doesn't exist
        let create_table_query = format!(
            r#"CREATE TABLE IF NOT EXISTS {}.{} (
                id text PRIMARY KEY,
                key text,
                secret text,
                max_connections int,
                enable_client_messages boolean,
                enabled boolean,
                max_backend_events_per_second int,
                max_client_events_per_second int,
                max_read_requests_per_second int,
                max_presence_members_per_channel int,
                max_presence_member_size_in_kb int,
                max_channel_name_length int,
                max_event_channels_at_once int,
                max_event_name_length int,
                max_event_payload_in_kb int,
                max_event_batch_size int,
                enable_user_authentication boolean,
                enable_watchlist_events boolean,
                webhooks text,
                allowed_origins text,
                created_at timestamp,
                updated_at timestamp
            )"#,
            config.keyspace, config.table_name
        );

        session
            .query_unpaged(create_table_query, &[])
            .await
            .map_err(|e| Error::Internal(format!("Failed to create table: {e}")))?;

        info!(
            "Ensured table '{}.{}' exists",
            config.keyspace, config.table_name
        );

        // Create secondary index on key for lookups
        let create_index_query = format!(
            r#"CREATE INDEX IF NOT EXISTS ON {}.{} (key)"#,
            config.keyspace, config.table_name
        );

        session
            .query_unpaged(create_index_query, &[])
            .await
            .map_err(|e| Error::Internal(format!("Failed to create index on key: {e}")))?;

        Ok(())
    }
}

/// Struct for ScyllaDB app row (both serialization and deserialization)
#[derive(SerializeRow, DeserializeRow)]
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
    webhooks: Option<String>,
    allowed_origins: Option<String>,
}

/// Struct for UPDATE (SET fields first, then id for WHERE)
#[derive(SerializeRow)]
struct UpdateRow {
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
    webhooks: Option<String>,
    allowed_origins: Option<String>,
    id: String,
}

impl UpdateRow {
    fn from_app(app: &App) -> Result<Self> {
        let webhooks = app
            .webhooks
            .as_ref()
            .map(|w| {
                serde_json::to_string(w)
                    .map_err(|e| Error::Internal(format!("Failed to serialize webhooks: {}", e)))
            })
            .transpose()?;

        let allowed_origins = app
            .allowed_origins
            .as_ref()
            .map(|o| {
                serde_json::to_string(o).map_err(|e| {
                    Error::Internal(format!("Failed to serialize allowed_origins: {}", e))
                })
            })
            .transpose()?;

        Ok(Self {
            key: app.key.clone(),
            secret: app.secret.clone(),
            max_connections: app.max_connections as i32,
            enable_client_messages: app.enable_client_messages,
            enabled: app.enabled,
            max_backend_events_per_second: app.max_backend_events_per_second.map(|v| v as i32),
            max_client_events_per_second: app.max_client_events_per_second as i32,
            max_read_requests_per_second: app.max_read_requests_per_second.map(|v| v as i32),
            max_presence_members_per_channel: app
                .max_presence_members_per_channel
                .map(|v| v as i32),
            max_presence_member_size_in_kb: app.max_presence_member_size_in_kb.map(|v| v as i32),
            max_channel_name_length: app.max_channel_name_length.map(|v| v as i32),
            max_event_channels_at_once: app.max_event_channels_at_once.map(|v| v as i32),
            max_event_name_length: app.max_event_name_length.map(|v| v as i32),
            max_event_payload_in_kb: app.max_event_payload_in_kb.map(|v| v as i32),
            max_event_batch_size: app.max_event_batch_size.map(|v| v as i32),
            enable_user_authentication: app.enable_user_authentication,
            enable_watchlist_events: app.enable_watchlist_events,
            webhooks,
            allowed_origins,
            id: app.id.clone(),
        })
    }
}

impl AppRow {
    fn from_app(app: &App) -> Result<Self> {
        let webhooks = app
            .webhooks
            .as_ref()
            .map(|w| {
                serde_json::to_string(w)
                    .map_err(|e| Error::Internal(format!("Failed to serialize webhooks: {}", e)))
            })
            .transpose()?;

        let allowed_origins = app
            .allowed_origins
            .as_ref()
            .map(|o| {
                serde_json::to_string(o).map_err(|e| {
                    Error::Internal(format!("Failed to serialize allowed_origins: {}", e))
                })
            })
            .transpose()?;

        Ok(Self {
            id: app.id.clone(),
            key: app.key.clone(),
            secret: app.secret.clone(),
            max_connections: app.max_connections as i32,
            enable_client_messages: app.enable_client_messages,
            enabled: app.enabled,
            max_backend_events_per_second: app.max_backend_events_per_second.map(|v| v as i32),
            max_client_events_per_second: app.max_client_events_per_second as i32,
            max_read_requests_per_second: app.max_read_requests_per_second.map(|v| v as i32),
            max_presence_members_per_channel: app
                .max_presence_members_per_channel
                .map(|v| v as i32),
            max_presence_member_size_in_kb: app.max_presence_member_size_in_kb.map(|v| v as i32),
            max_channel_name_length: app.max_channel_name_length.map(|v| v as i32),
            max_event_channels_at_once: app.max_event_channels_at_once.map(|v| v as i32),
            max_event_name_length: app.max_event_name_length.map(|v| v as i32),
            max_event_payload_in_kb: app.max_event_payload_in_kb.map(|v| v as i32),
            max_event_batch_size: app.max_event_batch_size.map(|v| v as i32),
            enable_user_authentication: app.enable_user_authentication,
            enable_watchlist_events: app.enable_watchlist_events,
            webhooks,
            allowed_origins,
        })
    }

    fn into_app(self) -> App {
        App {
            id: self.id.clone(),
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
            enable_watchlist_events: self.enable_watchlist_events,
            webhooks: self.webhooks.and_then(|json| {
                serde_json::from_str::<Vec<Webhook>>(&json)
                    .map_err(|e| {
                        error!("Failed to deserialize webhooks for app {}: {}", self.id, e)
                    })
                    .ok()
            }),
            allowed_origins: self.allowed_origins.and_then(|json| {
                serde_json::from_str::<Vec<String>>(&json)
                    .map_err(|e| {
                        error!(
                            "Failed to deserialize allowed_origins for app {}: {}",
                            self.id, e
                        )
                    })
                    .ok()
            }),
        }
    }
}

#[async_trait]
impl AppManager for ScyllaDbAppManager {
    async fn init(&self) -> Result<()> {
        // Already initialized in new()
        Ok(())
    }

    async fn create_app(&self, app: App) -> Result<()> {
        let values = AppRow::from_app(&app)?;

        self.session
            .execute_unpaged(&self.insert_stmt, values)
            .await
            .map_err(|e| {
                error!("Database error creating app {}: {}", app.id, e);
                Error::Internal(format!("Failed to insert app into ScyllaDB: {e}"))
            })?;

        Ok(())
    }

    async fn update_app(&self, app: App) -> Result<()> {
        let values = UpdateRow::from_app(&app)?;

        self.session
            .execute_unpaged(&self.update_stmt, values)
            .await
            .map_err(|e| {
                error!("Database error updating app {}: {}", app.id, e);
                Error::Internal(format!("Failed to update app in ScyllaDB: {e}"))
            })?;

        Ok(())
    }

    async fn delete_app(&self, app_id: &str) -> Result<()> {
        self.session
            .execute_unpaged(&self.delete_stmt, (app_id,))
            .await
            .map_err(|e| {
                error!("Database error deleting app {}: {}", app_id, e);
                Error::Internal(format!("Failed to delete app from ScyllaDB: {e}"))
            })?;

        Ok(())
    }

    async fn get_apps(&self) -> Result<Vec<App>> {
        let query = format!(
            r#"SELECT id, key, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size,
                enable_user_authentication, enable_watchlist_events,
                webhooks, allowed_origins
            FROM {}.{}"#,
            self.config.keyspace, self.config.table_name
        );

        let mut apps = Vec::new();
        let mut rows_stream = self
            .session
            .query_iter(query, &[])
            .await
            .map_err(|e| Error::Internal(format!("Failed to query apps from ScyllaDB: {e}")))?
            .rows_stream::<AppRow>()
            .map_err(|e| Error::Internal(format!("Failed to create rows stream: {e}")))?;

        while let Some(app_row) = rows_stream
            .try_next()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch app row: {e}")))?
        {
            apps.push(app_row.into_app());
        }

        Ok(apps)
    }

    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        debug!("Fetching app by key {} from ScyllaDB", key);

        // The secondary index on the key column allows efficient lookup without ALLOW FILTERING
        let query = format!(
            r#"SELECT id, key, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size,
                enable_user_authentication, enable_watchlist_events,
                webhooks, allowed_origins
            FROM {}.{} WHERE key = ?"#,
            self.config.keyspace, self.config.table_name
        );

        let mut rows_stream = self
            .session
            .query_iter(query, (key,))
            .await
            .map_err(|e| {
                error!("Database error fetching app by key {}: {}", key, e);
                Error::Internal(format!("Failed to fetch app by key from ScyllaDB: {e}"))
            })?
            .rows_stream::<AppRow>()
            .map_err(|e| Error::Internal(format!("Failed to create rows stream: {e}")))?;

        if let Some(app_row) = rows_stream
            .try_next()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch app row: {e}")))?
        {
            let app = app_row.into_app();
            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        debug!("Fetching app {} from ScyllaDB", app_id);

        let query = format!(
            r#"SELECT id, key, secret, max_connections, enable_client_messages, enabled,
                max_backend_events_per_second, max_client_events_per_second,
                max_read_requests_per_second, max_presence_members_per_channel,
                max_presence_member_size_in_kb, max_channel_name_length,
                max_event_channels_at_once, max_event_name_length,
                max_event_payload_in_kb, max_event_batch_size,
                enable_user_authentication, enable_watchlist_events,
                webhooks, allowed_origins
            FROM {}.{} WHERE id = ?"#,
            self.config.keyspace, self.config.table_name
        );

        let mut rows_stream = self
            .session
            .query_iter(query, (app_id,))
            .await
            .map_err(|e| {
                error!("Database error fetching app {}: {}", app_id, e);
                Error::Internal(format!("Failed to fetch app from ScyllaDB: {e}"))
            })?
            .rows_stream::<AppRow>()
            .map_err(|e| Error::Internal(format!("Failed to create rows stream: {e}")))?;

        if let Some(app_row) = rows_stream
            .try_next()
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch app row: {e}")))?
        {
            let app = app_row.into_app();
            Ok(Some(app))
        } else {
            Ok(None)
        }
    }

    async fn check_health(&self) -> Result<()> {
        // Simple health check: query system tables
        let query = "SELECT key FROM system.local WHERE key = 'local'";
        self.session
            .query_unpaged(query, &[])
            .await
            .map_err(|e| Error::Internal(format!("App manager ScyllaDB connection failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn create_test_config() -> ScyllaDbConfig {
        let nodes = env::var("SCYLLADB_NODES")
            .unwrap_or_else(|_| "localhost:19042".to_string())
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let keyspace = env::var("SCYLLADB_KEYSPACE").unwrap_or_else(|_| "sockudo_test".to_string());

        let replication_class =
            env::var("SCYLLADB_REPLICATION_CLASS").unwrap_or_else(|_| "SimpleStrategy".to_string());

        let replication_factor = env::var("SCYLLADB_REPLICATION_FACTOR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        ScyllaDbConfig {
            nodes,
            keyspace,
            table_name: "applications_test".to_string(),
            username: None,
            password: None,
            replication_class,
            replication_factor,
        }
    }

    fn create_test_app(id: &str) -> App {
        App {
            id: id.to_string(),
            key: format!("{}_key", id),
            secret: format!("{}_secret", id),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_backend_events_per_second: Some(100),
            max_client_events_per_second: 100,
            max_read_requests_per_second: Some(100),
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(2),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(10),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(10),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: None,
            enable_watchlist_events: None,
            allowed_origins: None,
        }
    }

    fn create_test_app_with_webhooks(id: &str) -> App {
        use crate::webhook::types::WebhookFilter;
        use url::Url;

        let webhooks = vec![
            Webhook {
                url: Some(Url::parse("https://example.com/webhook1").unwrap()),
                lambda_function: None,
                lambda: None,
                event_types: vec!["channel_occupied".to_string()],
                filter: None,
                headers: None,
            },
            Webhook {
                url: Some(Url::parse("https://example.com/webhook2").unwrap()),
                lambda_function: None,
                lambda: None,
                event_types: vec!["channel_vacated".to_string(), "member_added".to_string()],
                filter: Some(WebhookFilter {
                    channel_prefix: Some("private-".to_string()),
                    channel_suffix: None,
                    channel_pattern: None,
                }),
                headers: None,
            },
        ];

        App {
            id: id.to_string(),
            key: format!("{}_key", id),
            secret: format!("{}_secret", id),
            max_connections: 100,
            enable_client_messages: true,
            enabled: true,
            max_backend_events_per_second: Some(100),
            max_client_events_per_second: 100,
            max_read_requests_per_second: Some(100),
            max_presence_members_per_channel: Some(100),
            max_presence_member_size_in_kb: Some(2),
            max_channel_name_length: Some(200),
            max_event_channels_at_once: Some(10),
            max_event_name_length: Some(200),
            max_event_payload_in_kb: Some(10),
            max_event_batch_size: Some(10),
            enable_user_authentication: Some(true),
            webhooks: Some(webhooks),
            enable_watchlist_events: Some(true),
            allowed_origins: Some(vec![
                "https://example.com".to_string(),
                "https://app.example.com".to_string(),
            ]),
        }
    }

    #[tokio::test]
    async fn test_create_and_find_app() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        let app = create_test_app("test_app_create");
        manager.create_app(app.clone()).await.unwrap();

        let found = manager.find_by_id("test_app_create").await.unwrap();
        assert!(found.is_some());
        let found_app = found.unwrap();
        assert_eq!(found_app.id, "test_app_create");
        assert_eq!(found_app.key, "test_app_create_key");
        assert_eq!(found_app.secret, "test_app_create_secret");
        assert_eq!(found_app.max_connections, 100);

        manager.delete_app("test_app_create").await.unwrap();
    }

    #[tokio::test]
    async fn test_create_app_with_webhooks() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        let app = create_test_app_with_webhooks("test_app_webhooks");
        manager.create_app(app.clone()).await.unwrap();

        let found = manager.find_by_id("test_app_webhooks").await.unwrap();
        assert!(found.is_some());
        let found_app = found.unwrap();

        // Verify webhooks
        assert!(found_app.webhooks.is_some());
        let webhooks = found_app.webhooks.unwrap();
        assert_eq!(webhooks.len(), 2);
        assert_eq!(
            webhooks[0].url.as_ref().unwrap().as_str(),
            "https://example.com/webhook1"
        );
        assert_eq!(webhooks[0].event_types.len(), 1);
        assert_eq!(
            webhooks[1].url.as_ref().unwrap().as_str(),
            "https://example.com/webhook2"
        );
        assert_eq!(webhooks[1].event_types.len(), 2);

        // Verify enable_watchlist_events
        assert_eq!(found_app.enable_watchlist_events, Some(true));

        // Verify allowed_origins
        assert!(found_app.allowed_origins.is_some());
        let origins = found_app.allowed_origins.unwrap();
        assert_eq!(origins.len(), 2);
        assert_eq!(origins[0], "https://example.com");

        manager.delete_app("test_app_webhooks").await.unwrap();
    }

    #[tokio::test]
    async fn test_update_app() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        // Create initial app
        let mut app = create_test_app("test_app_update");
        manager.create_app(app.clone()).await.unwrap();

        // Update app with webhooks
        use url::Url;

        app.max_connections = 200;
        app.webhooks = Some(vec![Webhook {
            url: Some(Url::parse("https://updated.com/webhook").unwrap()),
            lambda_function: None,
            lambda: None,
            event_types: vec!["channel_occupied".to_string()],
            filter: None,
            headers: None,
        }]);
        app.enable_watchlist_events = Some(true);

        manager.update_app(app.clone()).await.unwrap();

        // Verify update
        let found = manager.find_by_id("test_app_update").await.unwrap();
        assert!(found.is_some());
        let found_app = found.unwrap();
        assert_eq!(found_app.max_connections, 200);
        assert!(found_app.webhooks.is_some());
        assert_eq!(
            found_app.webhooks.unwrap()[0]
                .url
                .as_ref()
                .unwrap()
                .as_str(),
            "https://updated.com/webhook"
        );
        assert_eq!(found_app.enable_watchlist_events, Some(true));

        manager.delete_app("test_app_update").await.unwrap();
    }

    #[tokio::test]
    async fn test_find_by_key() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        let app = create_test_app("test_app_key");
        manager.create_app(app.clone()).await.unwrap();

        let found = manager.find_by_key("test_app_key_key").await.unwrap();
        assert!(found.is_some());
        let found_app = found.unwrap();
        assert_eq!(found_app.key, "test_app_key_key");
        assert_eq!(found_app.id, "test_app_key");

        manager.delete_app("test_app_key").await.unwrap();
    }

    #[tokio::test]
    async fn test_get_apps() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        // Create multiple apps
        let app1 = create_test_app("test_get_apps_1");
        let app2 = create_test_app_with_webhooks("test_get_apps_2");
        let app3 = create_test_app("test_get_apps_3");

        manager.create_app(app1).await.unwrap();
        manager.create_app(app2).await.unwrap();
        manager.create_app(app3).await.unwrap();

        // Get all apps
        let apps = manager.get_apps().await.unwrap();

        // Should have at least our 3 test apps
        let test_apps: Vec<_> = apps
            .iter()
            .filter(|a| a.id.starts_with("test_get_apps_"))
            .collect();
        assert!(test_apps.len() >= 3);

        // Cleanup
        manager.delete_app("test_get_apps_1").await.unwrap();
        manager.delete_app("test_get_apps_2").await.unwrap();
        manager.delete_app("test_get_apps_3").await.unwrap();
    }

    #[tokio::test]
    async fn test_delete_app() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        let app = create_test_app("test_app_delete");
        manager.create_app(app.clone()).await.unwrap();

        // Verify it exists
        let found = manager.find_by_id("test_app_delete").await.unwrap();
        assert!(found.is_some());

        // Delete it
        manager.delete_app("test_app_delete").await.unwrap();

        // Verify it's gone
        let found = manager.find_by_id("test_app_delete").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_update_webhooks_to_none() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        // Create app with webhooks
        let mut app = create_test_app_with_webhooks("test_app_webhooks_none");
        manager.create_app(app.clone()).await.unwrap();

        // Update to remove webhooks
        app.webhooks = None;
        app.enable_watchlist_events = Some(false);
        manager.update_app(app.clone()).await.unwrap();

        // Verify webhooks are removed
        let found = manager.find_by_id("test_app_webhooks_none").await.unwrap();
        assert!(found.is_some());
        let found_app = found.unwrap();
        assert!(found_app.webhooks.is_none());
        assert_eq!(found_app.enable_watchlist_events, Some(false));

        manager.delete_app("test_app_webhooks_none").await.unwrap();
    }

    #[tokio::test]
    async fn test_health_check() {
        let config = create_test_config();
        let manager = ScyllaDbAppManager::new(config).await.unwrap();

        let result = manager.check_health().await;
        assert!(result.is_ok());
    }
}
