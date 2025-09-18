// Updates to src/app/factory.rs to add SurrealDB support

use crate::app::dynamodb_app_manager::{DynamoDbAppManager, DynamoDbConfig};
use crate::app::manager::AppManager;
use crate::app::memory_app_manager::MemoryAppManager;
use crate::app::mysql_app_manager::MySQLAppManager;
use crate::app::pg_app_manager::PgSQLAppManager;
use crate::app::surrealdb_app_manager::{SurrealDbAppManager, SurrealDbConfig}; // Add this import
use crate::error::Result;
use crate::options::{AppManagerConfig, AppManagerDriver, DatabaseConfig};
use std::sync::Arc;
use tracing::{info, warn};

pub struct AppManagerFactory;

impl AppManagerFactory {
    pub async fn create(
        config: &AppManagerConfig,
        db_config: &DatabaseConfig,
    ) -> Result<Arc<dyn AppManager + Send + Sync>> {
        info!("Initializing AppManager with driver: {:?}", config.driver);
        match config.driver {
            AppManagerDriver::Memory => Ok(Arc::new(MemoryAppManager::new())),

            AppManagerDriver::Mysql => {
                let mysql_db_config = db_config.mysql.clone();
                match MySQLAppManager::new(mysql_db_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "Failed to initialize MySQL app manager: {}, falling back to memory manager",
                            e
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }

            AppManagerDriver::PgSql => {
                let pg_db_config = db_config.postgres.clone();
                match PgSQLAppManager::new(pg_db_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "Failed to initialize PostgreSQL app manager: {}, falling back to memory manager",
                            e
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }

            AppManagerDriver::Dynamodb => {
                let dynamo_settings = &db_config.dynamodb;
                let dynamo_app_config = DynamoDbConfig {
                    region: dynamo_settings.region.clone(),
                    table_name: dynamo_settings.table_name.clone(),
                    endpoint: dynamo_settings.endpoint_url.clone(),
                    access_key: dynamo_settings.aws_access_key_id.clone(),
                    secret_key: dynamo_settings.aws_secret_access_key.clone(),
                    profile_name: dynamo_settings.aws_profile_name.clone(),
                };
                match DynamoDbAppManager::new(dynamo_app_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "Failed to initialize DynamoDB app manager: {}, falling back to memory manager",
                            e
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }

            AppManagerDriver::SurrealDB => {
                let surrealdb_settings = &db_config.surrealdb;
                let surrealdb_config = SurrealDbConfig {
                    url: surrealdb_settings.url.clone(),
                    namespace: surrealdb_settings.namespace.clone(),
                    database: surrealdb_settings.database.clone(),
                    username: surrealdb_settings.username.clone(),
                    password: surrealdb_settings.password.clone(),
                    table_name: surrealdb_settings.table_name.clone(),
                    cache_ttl: surrealdb_settings.cache_ttl,
                    cache_max_capacity: surrealdb_settings.cache_max_capacity,
                };
                match SurrealDbAppManager::new(surrealdb_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "Failed to initialize SurrealDB app manager: {}, falling back to memory manager",
                            e
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }
        }
    }
}
