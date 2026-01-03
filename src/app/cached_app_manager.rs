use crate::app::config::App;
use crate::app::manager::AppManager;
use crate::cache::manager::CacheManager;
use crate::error::{Error, Result};
use crate::options::CacheSettings;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const CACHE_PREFIX_ID: &str = "app:id";
const CACHE_PREFIX_KEY: &str = "app:key";

/// Caching decorator for AppManager implementations
pub struct CachedAppManager {
    inner: Arc<dyn AppManager + Send + Sync>,
    cache: Arc<Mutex<dyn CacheManager + Send + Sync>>,
    settings: CacheSettings,
}

impl CachedAppManager {
    pub fn new(
        inner: Arc<dyn AppManager + Send + Sync>,
        cache: Arc<Mutex<dyn CacheManager + Send + Sync>>,
        settings: CacheSettings,
    ) -> Self {
        Self {
            inner,
            cache,
            settings,
        }
    }

    fn cache_key_for_id(app_id: &str) -> String {
        format!("{}:{}", CACHE_PREFIX_ID, app_id)
    }

    fn cache_key_for_key(app_key: &str) -> String {
        format!("{}:{}", CACHE_PREFIX_KEY, app_key)
    }

    fn serialize<T: serde::Serialize>(value: &T) -> Result<String> {
        serde_json::to_string(value)
            .map_err(|e| Error::Internal(format!("Serialization failed: {}", e)))
    }

    fn deserialize<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
        serde_json::from_str(json)
            .map_err(|e| Error::Internal(format!("Deserialization failed: {}", e)))
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let mut cache = self.cache.lock().await;
        match cache.get(key).await {
            Ok(Some(json)) => match Self::deserialize(&json) {
                Ok(value) => {
                    debug!("Cache hit: {}", key);
                    Some(value)
                }
                Err(e) => {
                    warn!("Cache deserialize error for {}: {}", key, e);
                    None
                }
            },
            Ok(None) => {
                debug!("Cache miss: {}", key);
                None
            }
            Err(e) => {
                warn!("Cache get error for {}: {}", key, e);
                None
            }
        }
    }

    async fn set<T: serde::Serialize>(&self, key: &str, value: &T) {
        let json = match Self::serialize(value) {
            Ok(j) => j,
            Err(e) => {
                warn!("Cache serialize error for {}: {}", key, e);
                return;
            }
        };

        let mut cache = self.cache.lock().await;
        if let Err(e) = cache.set(key, &json, self.settings.ttl).await {
            warn!("Cache set error for {}: {}", key, e);
        }
    }

    async fn remove(&self, key: &str) {
        let mut cache = self.cache.lock().await;
        if let Err(e) = cache.remove(key).await {
            warn!("Cache remove error for {}: {}", key, e);
        }
    }

    async fn cache_app(&self, app: &App) {
        self.set(&Self::cache_key_for_id(&app.id), app).await;
        self.set(&Self::cache_key_for_key(&app.key), app).await;
    }

    async fn invalidate_app(&self, app_id: &str, app_key: &str) {
        self.remove(&Self::cache_key_for_id(app_id)).await;
        self.remove(&Self::cache_key_for_key(app_key)).await;
    }

    async fn invalidate_app_by_id(&self, app_id: &str) {
        let id_key = Self::cache_key_for_id(app_id);
        if let Some(app) = self.get::<App>(&id_key).await {
            self.invalidate_app(app_id, &app.key).await;
        } else {
            self.remove(&id_key).await;
        }
    }
}

#[async_trait]
impl AppManager for CachedAppManager {
    async fn init(&self) -> Result<()> {
        self.inner.init().await
    }

    /// Get an app by ID from cache or database
    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        if !self.settings.enabled {
            return self.inner.find_by_id(app_id).await;
        }

        let cache_key = Self::cache_key_for_id(app_id);
        if let Some(app) = self.get::<App>(&cache_key).await {
            return Ok(Some(app));
        }

        let app = self.inner.find_by_id(app_id).await?;
        if let Some(ref app) = app {
            self.cache_app(app).await;
        }

        Ok(app)
    }

    /// Get an app by key from cache or database
    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        if !self.settings.enabled {
            return self.inner.find_by_key(key).await;
        }

        let cache_key = Self::cache_key_for_key(key);
        if let Some(app) = self.get::<App>(&cache_key).await {
            return Ok(Some(app));
        }

        let app = self.inner.find_by_key(key).await?;
        if let Some(ref app) = app {
            self.cache_app(app).await;
        }

        Ok(app)
    }

    /// Register a new app in the database and cache
    async fn create_app(&self, config: App) -> Result<()> {
        self.inner.create_app(config.clone()).await?;

        if self.settings.enabled {
            self.cache_app(&config).await;
        }

        Ok(())
    }

    /// Update an existing app and refresh the cache
    async fn update_app(&self, config: App) -> Result<()> {
        self.inner.update_app(config.clone()).await?;

        if self.settings.enabled {
            self.invalidate_app(&config.id, &config.key).await;
            self.cache_app(&config).await;
        }

        Ok(())
    }

    /// Remove an app from the database and cache
    async fn delete_app(&self, app_id: &str) -> Result<()> {
        let app = if self.settings.enabled {
            self.inner.find_by_id(app_id).await?
        } else {
            None
        };

        self.inner.delete_app(app_id).await?;

        if self.settings.enabled {
            if let Some(app) = app {
                self.invalidate_app(app_id, &app.key).await;
            } else {
                self.invalidate_app_by_id(app_id).await;
            }
        }

        Ok(())
    }

    /// Get all apps from the database
    async fn get_apps(&self) -> Result<Vec<App>> {
        let apps = self.inner.get_apps().await?;

        if self.settings.enabled && !apps.is_empty() {
            for app in &apps {
                self.cache_app(app).await;
            }
            debug!("Cached {} apps", apps.len());
        }

        Ok(apps)
    }

    async fn check_health(&self) -> Result<()> {
        self.inner.check_health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::memory_app_manager::MemoryAppManager;
    use crate::cache::memory_cache_manager::MemoryCacheManager;
    use crate::options::MemoryCacheOptions;

    fn create_test_app(id: &str) -> App {
        App {
            id: id.to_string(),
            key: format!("{}_key", id),
            secret: format!("{}_secret", id),
            max_connections: 100,
            enable_client_messages: false,
            enabled: true,
            max_client_events_per_second: 100,
            ..Default::default()
        }
    }

    async fn create_test_manager() -> CachedAppManager {
        let inner = Arc::new(MemoryAppManager::new());
        let cache = Arc::new(Mutex::new(MemoryCacheManager::new(
            "test".to_string(),
            MemoryCacheOptions {
                ttl: 300,
                cleanup_interval: 60,
                max_capacity: 1000,
            },
        )));
        let settings = CacheSettings {
            enabled: true,
            ttl: 300,
        };

        CachedAppManager::new(inner, cache, settings)
    }

    #[tokio::test]
    async fn test_find_by_id_caches_result() {
        let manager = create_test_manager().await;
        let app = create_test_app("test1");

        manager.create_app(app.clone()).await.unwrap();

        let found1 = manager.find_by_id("test1").await.unwrap();
        assert!(found1.is_some());

        let found2 = manager.find_by_id("test1").await.unwrap();
        assert!(found2.is_some());
        assert_eq!(found2.unwrap().id, "test1");
    }

    #[tokio::test]
    async fn test_find_by_key_caches_result() {
        let manager = create_test_manager().await;
        let app = create_test_app("test2");

        manager.create_app(app.clone()).await.unwrap();

        let found = manager.find_by_key("test2_key").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().key, "test2_key");
    }

    #[tokio::test]
    async fn test_update_invalidates_cache() {
        let manager = create_test_manager().await;
        let mut app = create_test_app("test3");

        manager.create_app(app.clone()).await.unwrap();

        let found = manager.find_by_id("test3").await.unwrap().unwrap();
        assert_eq!(found.max_connections, 100);

        app.max_connections = 200;
        manager.update_app(app).await.unwrap();

        let found_updated = manager.find_by_id("test3").await.unwrap().unwrap();
        assert_eq!(found_updated.max_connections, 200);
    }

    #[tokio::test]
    async fn test_delete_invalidates_cache() {
        let manager = create_test_manager().await;
        let app = create_test_app("test4");

        manager.create_app(app).await.unwrap();
        assert!(manager.find_by_id("test4").await.unwrap().is_some());

        manager.delete_app("test4").await.unwrap();
        assert!(manager.find_by_id("test4").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cache_disabled() {
        let inner = Arc::new(MemoryAppManager::new());
        let cache = Arc::new(Mutex::new(MemoryCacheManager::new(
            "test".to_string(),
            MemoryCacheOptions::default(),
        )));
        let settings = CacheSettings {
            enabled: false,
            ttl: 300,
        };

        let manager = CachedAppManager::new(inner, cache, settings);
        let app = create_test_app("test5");

        manager.create_app(app).await.unwrap();

        let found = manager.find_by_id("test5").await.unwrap();
        assert!(found.is_some());
    }
}
