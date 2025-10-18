// src/app/factory.rs
#[cfg(feature = "dynamodb")]
use crate::app::dynamodb_app_manager::{DynamoDbAppManager, DynamoDbConfig};
use crate::app::manager::AppManager;
use crate::app::memory_app_manager::MemoryAppManager;
#[cfg(feature = "mysql")]
use crate::app::mysql_app_manager::MySQLAppManager;
use crate::error::Result;

#[cfg(feature = "postgres")]
use crate::app::pg_app_manager::PgSQLAppManager;
use crate::options::{AppManagerConfig, AppManagerDriver, DatabaseConfig}; // Import AppManagerDriver
use std::sync::Arc;
use tracing::{info, warn};

pub struct AppManagerFactory;

impl AppManagerFactory {
    #[allow(unused_variables)]
    pub async fn create(
        config: &AppManagerConfig,
        db_config: &DatabaseConfig,
    ) -> Result<Arc<dyn AppManager + Send + Sync>> {
        info!(
            "{}",
            format!("Initializing AppManager with driver: {:?}", config.driver)
        );
        match config.driver {
            // Match on the enum
            #[cfg(feature = "mysql")]
            AppManagerDriver::Mysql => {
                let mysql_db_config = db_config.mysql.clone();
                match MySQLAppManager::new(mysql_db_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "{}",
                            format!(
                                "Failed to initialize MySQL app manager: {}, falling back to memory manager",
                                e
                            )
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }
            #[cfg(feature = "dynamodb")]
            AppManagerDriver::Dynamodb => {
                let dynamo_settings = &db_config.dynamodb; // Use the new dedicated settings

                let dynamo_app_config = DynamoDbConfig {
                    // This is from app::dynamodb_app_manager
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
                            "{}",
                            format!(
                                "Failed to initialize DynamoDB app manager: {}, falling back to memory manager",
                                e
                            )
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }
            #[cfg(feature = "postgres")]
            AppManagerDriver::PgSql => {
                let pgsql_db_config = db_config.postgres.clone();
                match PgSQLAppManager::new(pgsql_db_config).await {
                    Ok(manager) => Ok(Arc::new(manager)),
                    Err(e) => {
                        warn!(
                            "{}",
                            format!(
                                "Failed to initialize PgSQL app manager: {}, falling back to memory manager",
                                e
                            )
                        );
                        Ok(Arc::new(MemoryAppManager::new()))
                    }
                }
            }
            AppManagerDriver::Memory => {
                // Handle unknown as Memory or make it an error
                info!("{}", "Using memory app manager.".to_string());
                Ok(Arc::new(MemoryAppManager::new()))
            }
            #[cfg(not(feature = "mysql"))]
            AppManagerDriver::Mysql => {
                warn!(
                    "{}",
                    "MySQL app manager requested but not compiled in. Falling back to memory manager."
                );
                Ok(Arc::new(MemoryAppManager::new()))
            }
            #[cfg(not(feature = "dynamodb"))]
            AppManagerDriver::Dynamodb => {
                warn!(
                    "{}",
                    "DynamoDB app manager requested but not compiled in. Falling back to memory manager."
                );
                Ok(Arc::new(MemoryAppManager::new()))
            }
            #[cfg(not(feature = "postgres"))]
            AppManagerDriver::PgSql => {
                warn!(
                    "{}",
                    "PostgreSQL app manager requested but not compiled in. Falling back to memory manager."
                );
                Ok(Arc::new(MemoryAppManager::new()))
            }
        }
    }
}
