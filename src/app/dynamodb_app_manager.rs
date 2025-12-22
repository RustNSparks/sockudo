// src/app/dynamodb_manager.rs
use super::config::App;
use crate::app::manager::AppManager;
use crate::error::{Error, Result};
use crate::webhook::types::Webhook;
use async_trait::async_trait;
use moka::future::Cache;
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;

/// Configuration for DynamoDB App Manager
#[derive(Debug, Clone)]
pub struct DynamoDbConfig {
    pub region: String,
    pub table_name: String,
    pub endpoint: Option<String>, // For local development
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub profile_name: Option<String>,
    pub cache_ttl: u64,
    pub cache_max_capacity: u64,
}

impl Default for DynamoDbConfig {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_string(),
            table_name: "sockudo-applications".to_string(),
            endpoint: None,
            access_key: None,
            secret_key: None,
            profile_name: None,
            cache_ttl: 3600,
            cache_max_capacity: 10000,
        }
    }
}

// Type aliases for AWS SDK
type DynamoClient = aws_sdk_dynamodb::Client;

// Struct for the DynamoDB App Manager
pub struct DynamoDbAppManager {
    config: DynamoDbConfig,
    client: DynamoClient,
    app_cache: Cache<String, App>,
}

impl DynamoDbAppManager {
    pub async fn new(config: DynamoDbConfig) -> Result<Self> {
        // Build AWS config
        let mut aws_config_builder = aws_config::from_env();

        // Set region
        aws_config_builder =
            aws_config_builder.region(aws_sdk_dynamodb::config::Region::new(config.region.clone()));

        // Set endpoint if provided (for local development)
        if let Some(endpoint) = &config.endpoint {
            aws_config_builder = aws_config_builder.endpoint_url(endpoint);
        }

        // Set credentials if provided
        if let (Some(access_key), Some(secret_key)) = (&config.access_key, &config.secret_key) {
            let credentials_provider = aws_sdk_dynamodb::config::Credentials::new(
                access_key, secret_key, None, // session token
                None, // expiry
                "static",
            );
            aws_config_builder = aws_config_builder.credentials_provider(credentials_provider);
        }

        // Set profile if provided
        if let Some(profile) = &config.profile_name {
            aws_config_builder = aws_config_builder.profile_name(profile);
        }

        // Build AWS config
        let aws_config = aws_config_builder.load().await;

        // Create DynamoDB client
        let client = aws_sdk_dynamodb::Client::new(&aws_config);

        // Initialize cache
        let app_cache = Cache::builder()
            .time_to_live(Duration::from_secs(config.cache_ttl))
            .max_capacity(config.cache_max_capacity)
            .build();

        // Create the manager
        let manager = Self {
            config,
            client,
            app_cache,
        };

        Ok(manager)
    }
    /// Convert a DynamoDB item to an App struct
    fn item_to_app(&self, item: aws_sdk_dynamodb::types::AttributeValue) -> Result<App> {
        if let aws_sdk_dynamodb::types::AttributeValue::M(map) = item {
            let get_string = |key: &str| -> Result<String> {
                if let Some(aws_sdk_dynamodb::types::AttributeValue::S(s)) = map.get(key) {
                    Ok(s.clone())
                } else {
                    Err(Error::Internal(format!(
                        "Missing or invalid {key} attribute"
                    )))
                }
            };

            let get_bool = |key: &str, default: bool| -> bool {
                if let Some(aws_sdk_dynamodb::types::AttributeValue::Bool(b)) = map.get(key) {
                    *b
                } else {
                    default
                }
            };

            let get_u32 = |key: &str, default: Option<u32>| -> Option<u32> {
                if let Some(aws_sdk_dynamodb::types::AttributeValue::N(n)) = map.get(key) {
                    n.parse::<u32>().ok()
                } else {
                    default
                }
            };

            Ok(App {
                id: get_string("id")?,
                key: get_string("key")?,
                secret: get_string("secret")?,
                max_connections: get_u32("max_connections", Some(0)).unwrap_or(0),
                enable_client_messages: get_bool("enable_client_messages", false),
                enabled: get_bool("enabled", true),
                max_backend_events_per_second: get_u32("max_backend_events_per_second", None),
                max_client_events_per_second: get_u32("max_client_events_per_second", Some(0))
                    .unwrap_or(0),
                max_read_requests_per_second: get_u32("max_read_requests_per_second", None),
                max_presence_members_per_channel: get_u32("max_presence_members_per_channel", None),
                max_presence_member_size_in_kb: get_u32("max_presence_member_size_in_kb", None),
                max_channel_name_length: get_u32("max_channel_name_length", None),
                max_event_channels_at_once: get_u32("max_event_channels_at_once", None),
                max_event_name_length: get_u32("max_event_name_length", None),
                max_event_payload_in_kb: get_u32("max_event_payload_in_kb", None),
                max_event_batch_size: get_u32("max_event_batch_size", None),
                enable_user_authentication: if let Some(
                    aws_sdk_dynamodb::types::AttributeValue::Bool(b),
                ) = map.get("enable_user_authentication")
                {
                    Some(*b)
                } else {
                    None
                },
                webhooks: if let Some(aws_sdk_dynamodb::types::AttributeValue::S(json_str)) =
                    map.get("webhooks")
                {
                    serde_json::from_str::<Vec<Webhook>>(json_str)
                        .map_err(|e| {
                            tracing::warn!("Failed to parse webhooks JSON: {}", e);
                            e
                        })
                        .ok()
                } else {
                    None
                },
                enable_watchlist_events: if let Some(
                    aws_sdk_dynamodb::types::AttributeValue::Bool(b),
                ) = map.get("enable_watchlist_events")
                {
                    Some(*b)
                } else {
                    None
                },
                allowed_origins: if let Some(aws_sdk_dynamodb::types::AttributeValue::L(list)) =
                    map.get("allowed_origins")
                {
                    let origins: Vec<String> = list
                        .iter()
                        .filter_map(|item| {
                            if let aws_sdk_dynamodb::types::AttributeValue::S(s) = item {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if origins.is_empty() {
                        None
                    } else {
                        Some(origins)
                    }
                } else {
                    None
                },
            })
        } else {
            Err(Error::Internal("Invalid DynamoDB item format".to_string()))
        }
    }

    /// Convert an App struct to DynamoDB item
    fn app_to_item(&self, app: &App) -> HashMap<String, aws_sdk_dynamodb::types::AttributeValue> {
        let mut item = HashMap::new();

        // Required fields
        item.insert(
            "id".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S(app.id.clone()),
        );
        item.insert(
            "key".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S(app.key.clone()),
        );
        item.insert(
            "secret".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::S(app.secret.clone()),
        );
        item.insert(
            "max_connections".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::N(app.max_connections.to_string()),
        );
        item.insert(
            "enable_client_messages".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::Bool(app.enable_client_messages),
        );
        item.insert(
            "enabled".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::Bool(app.enabled),
        );
        item.insert(
            "max_client_events_per_second".to_string(),
            aws_sdk_dynamodb::types::AttributeValue::N(
                app.max_client_events_per_second.to_string(),
            ),
        );

        // Optional fields
        if let Some(val) = app.max_backend_events_per_second {
            item.insert(
                "max_backend_events_per_second".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_read_requests_per_second {
            item.insert(
                "max_read_requests_per_second".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_presence_members_per_channel {
            item.insert(
                "max_presence_members_per_channel".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_presence_member_size_in_kb {
            item.insert(
                "max_presence_member_size_in_kb".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_channel_name_length {
            item.insert(
                "max_channel_name_length".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_event_channels_at_once {
            item.insert(
                "max_event_channels_at_once".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_event_name_length {
            item.insert(
                "max_event_name_length".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_event_payload_in_kb {
            item.insert(
                "max_event_payload_in_kb".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.max_event_batch_size {
            item.insert(
                "max_event_batch_size".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::N(val.to_string()),
            );
        }

        if let Some(val) = app.enable_user_authentication {
            item.insert(
                "enable_user_authentication".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::Bool(val),
            );
        }

        if let Some(val) = app.enable_watchlist_events {
            item.insert(
                "enable_watchlist_events".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::Bool(val),
            );
        }

        if let Some(webhooks) = &app.webhooks {
            let json_str = serde_json::to_string(webhooks)
                .expect("Failed to serialize webhooks to JSON. This indicates a bug.");
            item.insert(
                "webhooks".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::S(json_str),
            );
        }

        if let Some(origins) = &app.allowed_origins {
            let origin_list: Vec<aws_sdk_dynamodb::types::AttributeValue> = origins
                .iter()
                .map(|s| aws_sdk_dynamodb::types::AttributeValue::S(s.clone()))
                .collect();
            item.insert(
                "allowed_origins".to_string(),
                aws_sdk_dynamodb::types::AttributeValue::L(origin_list),
            );
        }

        item
    }

    /// Check if the DynamoDB table exists
    async fn table_exists(&self) -> Result<bool> {
        let result = self
            .client
            .describe_table()
            .table_name(&self.config.table_name)
            .send()
            .await;

        Ok(result.is_ok())
    }

    /// Create the DynamoDB table if it doesn't exist
    async fn ensure_table_exists(&self) -> Result<()> {
        if self.table_exists().await? {
            return Ok(());
        }

        // Create table with primary key 'id'
        self.client
            .create_table()
            .table_name(&self.config.table_name)
            .key_schema(
                aws_sdk_dynamodb::types::KeySchemaElement::builder()
                    .attribute_name("id")
                    .key_type(aws_sdk_dynamodb::types::KeyType::Hash)
                    .build()
                    .unwrap(),
            )
            .attribute_definitions(
                aws_sdk_dynamodb::types::AttributeDefinition::builder()
                    .attribute_name("id")
                    .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            // Add attribute definition for the GSI key
            .attribute_definitions(
                aws_sdk_dynamodb::types::AttributeDefinition::builder()
                    .attribute_name("key")
                    .attribute_type(aws_sdk_dynamodb::types::ScalarAttributeType::S)
                    .build()
                    .unwrap(),
            )
            // Add GSI for looking up by key
            .global_secondary_indexes(
                aws_sdk_dynamodb::types::GlobalSecondaryIndex::builder()
                    .index_name("KeyIndex")
                    .key_schema(
                        aws_sdk_dynamodb::types::KeySchemaElement::builder()
                            .attribute_name("key")
                            .key_type(aws_sdk_dynamodb::types::KeyType::Hash)
                            .build()
                            .unwrap(),
                    )
                    .projection(
                        aws_sdk_dynamodb::types::Projection::builder()
                            .projection_type(aws_sdk_dynamodb::types::ProjectionType::All)
                            .build(),
                    )
                    .provisioned_throughput(
                        aws_sdk_dynamodb::types::ProvisionedThroughput::builder()
                            .read_capacity_units(5)
                            .write_capacity_units(5)
                            .build()
                            .unwrap(),
                    )
                    .build()
                    .unwrap(),
            )
            .provisioned_throughput(
                aws_sdk_dynamodb::types::ProvisionedThroughput::builder()
                    .read_capacity_units(5)
                    .write_capacity_units(5)
                    .build()
                    .unwrap(),
            )
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to create DynamoDB table: {e}")))?;

        // Wait for table to be created
        let mut retries = 0;
        while retries < 10 {
            // Wait a bit before checking
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let table_status = self
                .client
                .describe_table()
                .table_name(&self.config.table_name)
                .send()
                .await;

            if let Ok(response) = table_status
                && let Some(table) = response.table()
                && let Some(status) = table.table_status()
                && status == &aws_sdk_dynamodb::types::TableStatus::Active
            {
                return Ok(());
            }

            retries += 1;
        }

        Err(Error::Internal(
            "Timeout waiting for DynamoDB table to be created".to_string(),
        ))
    }

    /// Get an app from cache or DynamoDB
    async fn get_app_internal(&self, app_id: &str) -> Result<Option<App>> {
        // Check cache first
        if let Some(app) = self.app_cache.get(app_id).await {
            return Ok(Some(app));
        }

        debug!("Cache miss for app {}, fetching from DynamoDB", app_id);

        // Fetch from DynamoDB
        let response = self
            .client
            .get_item()
            .table_name(&self.config.table_name)
            .key(
                "id",
                aws_sdk_dynamodb::types::AttributeValue::S(app_id.to_string()),
            )
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to get item from DynamoDB: {e}")))?;

        if let Some(item) = response.item() {
            // Convert DynamoDB item to App
            let app = self.item_to_app(aws_sdk_dynamodb::types::AttributeValue::M(item.clone()))?;

            // Update cache
            self.app_cache.insert(app_id.to_string(), app.clone()).await;

            Ok(Some(app))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl AppManager for DynamoDbAppManager {
    async fn init(&self) -> Result<()> {
        // Create the table if it doesn't exist
        self.ensure_table_exists().await
    }

    async fn create_app(&self, config: App) -> Result<()> {
        // Convert App to DynamoDB item
        let item = self.app_to_item(&config);

        // Insert item into DynamoDB
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to insert app into DynamoDB: {e}")))?;

        // Update cache
        self.app_cache.insert(config.id.clone(), config).await;

        Ok(())
    }

    async fn update_app(&self, config: App) -> Result<()> {
        // Convert App to DynamoDB item
        let item = self.app_to_item(&config);

        // Update item in DynamoDB
        self.client
            .put_item()
            .table_name(&self.config.table_name)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to update app in DynamoDB: {e}")))?;

        // Update cache
        self.app_cache.insert(config.id.clone(), config).await;

        Ok(())
    }

    async fn delete_app(&self, app_id: &str) -> Result<()> {
        // Remove item from DynamoDB
        self.client
            .delete_item()
            .table_name(&self.config.table_name)
            .key(
                "id",
                aws_sdk_dynamodb::types::AttributeValue::S(app_id.to_string()),
            )
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to delete app from DynamoDB: {e}")))?;

        // Remove from cache
        self.app_cache.invalidate(app_id).await;

        Ok(())
    }

    async fn get_apps(&self) -> Result<Vec<App>> {
        // Scan DynamoDB for all apps
        let response = self
            .client
            .scan()
            .table_name(&self.config.table_name)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to scan DynamoDB: {e}")))?;

        // Process items and convert to App objects
        let mut apps = Vec::new();
        let items = response.items();
        for item in items {
            let app = self.item_to_app(aws_sdk_dynamodb::types::AttributeValue::M(item.clone()))?;
            apps.push(app);
        }

        Ok(apps)
    }

    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        // Query DynamoDB by key (using GSI)
        let response = self
            .client
            .query()
            .table_name(&self.config.table_name)
            .index_name("KeyIndex")
            .key_condition_expression("#app_key = :key_val")
            .expression_attribute_names("#app_key", "key")
            .expression_attribute_values(
                ":key_val",
                aws_sdk_dynamodb::types::AttributeValue::S(key.to_string()),
            )
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to query DynamoDB: {e}")))?;

        let items = response.items();
        if !items.is_empty()
            && let Some(item) = items.first()
        {
            // Convert DynamoDB item to App
            let app = self.item_to_app(aws_sdk_dynamodb::types::AttributeValue::M(item.clone()))?;

            // Update cache using app ID as key
            self.app_cache.insert(app.id.clone(), app.clone()).await;

            return Ok(Some(app));
        }

        Ok(None)
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        self.get_app_internal(app_id).await
    }

    async fn check_health(&self) -> Result<()> {
        self.client.list_tables().send().await.map_err(|e| {
            crate::error::Error::Internal(format!("App manager DynamoDB connection failed: {e}"))
        })?;
        Ok(())
    }
}

#[cfg(test)]
impl DynamoDbAppManager {
    // Check if a specific app ID is in the cache.
    pub async fn is_cached(&self, app_id: &str) -> bool {
        self.app_cache.get(app_id).await.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a test config for DynamoDB Local
    fn get_test_config(table_name: &str) -> DynamoDbConfig {
        DynamoDbConfig {
            region: std::env::var("DYNAMODB_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            table_name: table_name.to_string(),
            endpoint: Some(
                std::env::var("DYNAMODB_ENDPOINT")
                    .unwrap_or_else(|_| "http://localhost:8000".to_string()),
            ),
            access_key: Some("test".to_string()),
            secret_key: Some("test".to_string()),
            profile_name: None,
            cache_ttl: 2, // Short TTL for testing
            cache_max_capacity: 100,
        }
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

    async fn is_dynamodb_available() -> bool {
        let config = get_test_config("test_availability");
        match DynamoDbAppManager::new(config).await {
            Ok(manager) => manager.check_health().await.is_ok(),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn test_dynamodb_app_manager() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_test_apps");
        let manager = DynamoDbAppManager::new(config).await.unwrap();
        manager.init().await.unwrap();

        // Test registering an app
        let test_app = create_test_app("dynamo_test1");
        manager.create_app(test_app.clone()).await.unwrap();

        // Test getting an app by ID
        let app = manager.find_by_id("dynamo_test1").await.unwrap().unwrap();
        assert_eq!(app.id, "dynamo_test1");
        assert_eq!(app.key, "dynamo_test1_key");

        // Test getting an app by key
        let app = manager
            .find_by_key("dynamo_test1_key")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app.id, "dynamo_test1");

        // Test updating an app
        let mut updated_app = test_app.clone();
        updated_app.max_connections = 200;
        manager.update_app(updated_app).await.unwrap();

        let app = manager.find_by_id("dynamo_test1").await.unwrap().unwrap();
        assert_eq!(app.max_connections, 200);

        // Test getting all apps
        let test_app2 = create_test_app("dynamo_test2");
        manager.create_app(test_app2).await.unwrap();

        let apps = manager.get_apps().await.unwrap();
        assert!(apps.len() >= 2);

        // Test removing an app
        manager.delete_app("dynamo_test1").await.unwrap();
        assert!(manager.find_by_id("dynamo_test1").await.unwrap().is_none());

        // Cleanup
        manager.delete_app("dynamo_test2").await.unwrap();
    }

    #[tokio::test]
    async fn test_cache_behavior() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_cache_test");
        let manager = DynamoDbAppManager::new(config).await.unwrap();
        manager.init().await.unwrap();

        // Cache should be empty initially
        assert!(!manager.is_cached("cache_test").await);

        // Create an app (should populate cache)
        let app = create_test_app("cache_test");
        manager.create_app(app).await.unwrap();

        // After create, app should be in cache
        assert!(manager.is_cached("cache_test").await);

        // First retrieval - should hit cache
        let retrieved1 = manager.find_by_id("cache_test").await.unwrap().unwrap();
        assert_eq!(retrieved1.id, "cache_test");

        // Verify still cached after retrieval
        assert!(manager.is_cached("cache_test").await);

        // Second retrieval - should still hit cache
        let retrieved2 = manager.find_by_id("cache_test").await.unwrap().unwrap();
        assert_eq!(retrieved2.id, "cache_test");
        assert!(manager.is_cached("cache_test").await);

        // Wait for cache to expire (TTL is 2 seconds)
        tokio::time::sleep(Duration::from_secs(3)).await;

        // After TTL expiration, cache entry should be gone
        assert!(!manager.is_cached("cache_test").await);

        // Third retrieval - should hit DynamoDB and repopulate cache
        let retrieved3 = manager.find_by_id("cache_test").await.unwrap().unwrap();
        assert_eq!(retrieved3.id, "cache_test");

        // Cache should be populated again
        assert!(manager.is_cached("cache_test").await);

        // Cleanup
        manager.delete_app("cache_test").await.unwrap();

        // After delete, cache should be invalidated
        assert!(!manager.is_cached("cache_test").await);
    }

    #[tokio::test]
    async fn test_cache_invalidation_on_update() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_cache_update_test");
        let manager = DynamoDbAppManager::new(config).await.unwrap();
        manager.init().await.unwrap();

        // Create an app
        let mut app = create_test_app("cache_update_test");
        manager.create_app(app.clone()).await.unwrap();

        // Retrieve to populate cache
        let retrieved1 = manager
            .find_by_id("cache_update_test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved1.max_connections, 100);

        // Update the app
        app.max_connections = 500;
        manager.update_app(app).await.unwrap();

        // Retrieve again - should get updated value from cache
        let retrieved2 = manager
            .find_by_id("cache_update_test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved2.max_connections, 500);

        // Cleanup
        manager.delete_app("cache_update_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_cache_invalidation_on_delete() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_cache_delete_test");
        let manager = DynamoDbAppManager::new(config).await.unwrap();
        manager.init().await.unwrap();

        // Create an app
        let app = create_test_app("cache_delete_test");
        manager.create_app(app).await.unwrap();

        // Retrieve to populate cache
        let retrieved = manager
            .find_by_id("cache_delete_test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.id, "cache_delete_test");

        // Delete the app
        manager.delete_app("cache_delete_test").await.unwrap();

        // Should not find the app (cache should be invalidated)
        assert!(
            manager
                .find_by_id("cache_delete_test")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_allowed_origins() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_origins_test");
        let manager = DynamoDbAppManager::new(config).await.unwrap();
        manager.init().await.unwrap();

        // Create app with allowed origins
        let mut app = create_test_app("origins_test");
        app.allowed_origins = Some(vec![
            "https://example.com".to_string(),
            "https://*.example.com".to_string(),
            "http://localhost:3000".to_string(),
        ]);
        manager.create_app(app).await.unwrap();

        // Retrieve and verify
        let retrieved = manager.find_by_id("origins_test").await.unwrap().unwrap();
        assert!(retrieved.allowed_origins.is_some());
        let origins = retrieved.allowed_origins.unwrap();
        assert_eq!(origins.len(), 3);
        assert!(origins.contains(&"https://example.com".to_string()));

        // Cleanup
        manager.delete_app("origins_test").await.unwrap();
    }

    #[tokio::test]
    async fn test_health_check() {
        // Skip test if DynamoDB Local is not available
        if !is_dynamodb_available().await {
            eprintln!("Skipping test: DynamoDB Local not available");
            return;
        }

        let config = get_test_config("sockudo_health_test");
        let manager = DynamoDbAppManager::new(config).await.unwrap();

        // Health check should succeed
        let result = manager.check_health().await;
        assert!(result.is_ok());
    }
}
