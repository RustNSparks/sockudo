// src/cache/memory_cache_manager.rs

use crate::cache::manager::CacheManager;
use crate::error::Result; // Assuming your project's Error/Result types
use crate::options::MemoryCacheOptions; // Using the type-safe options
use async_trait::async_trait;
use moka::future::Cache;
use std::time::Duration;
// std::sync::Arc and Mutex are not directly needed in this struct if CacheManager trait doesn't require them for self

/// A Memory-based implementation of the CacheManager trait using Moka.
#[derive(Clone)] // Add Clone if CacheManager instances need to be cloned (e.g., for Arc<Mutex<CacheManager>>)
pub struct MemoryCacheManager {
    /// Moka async cache for storing entries. Key and Value are Strings.
    cache: Cache<String, String, ahash::RandomState>,
    /// Configuration options for this cache instance.
    options: MemoryCacheOptions,
    /// Prefix for all keys in this cache instance.
    prefix: String,
}

impl MemoryCacheManager {
    /// Creates a new Memory cache manager with Moka configuration.
    pub fn new(prefix: String, options: MemoryCacheOptions) -> Self {
        let cache_builder = Cache::builder()
            .max_capacity(options.max_capacity)
            // Moka's cleanup is internal and efficient, so options.cleanup_interval is not directly used here.
            .name(format!("sockudo-memory-cache-{prefix}").as_str()); // Optional: name the cache for monitoring

        // Set default time_to_live if options.ttl > 0
        let cache = if options.ttl > 0 {
            cache_builder.time_to_live(Duration::from_secs(options.ttl))
        } else {
            // No default TTL, entries live forever unless Moka's default (which is no expiry)
            // or if a more complex per-entry expiry was implemented (not done here for simplicity).
            cache_builder
        }
        .build_with_hasher(ahash::RandomState::new());

        Self {
            cache,
            options,
            prefix,
        }
    }

    /// Get the prefixed key.
    fn prefixed_key(&self, key: &str) -> String {
        format!("{}:{}", self.prefix, key)
    }
}

#[async_trait]
impl CacheManager for MemoryCacheManager {
    async fn has(&mut self, key: &str) -> Result<bool> {
        let prefixed_key = self.prefixed_key(key);
        // Moka's `get` implicitly checks expiry.
        let exists = self.cache.get(&prefixed_key).await.is_some();
        Ok(exists)
    }

    async fn get(&mut self, key: &str) -> Result<Option<String>> {
        let prefixed_key = self.prefixed_key(key);
        Ok(self.cache.get(&prefixed_key).await)
    }

    /// Set or overwrite the value in the Moka cache.
    /// The `ttl_seconds` parameter is largely advisory with the current Moka setup.
    /// If `options.ttl` was set during cache creation, that TTL applies to all entries.
    /// Moka's `insert` does not take a per-entry TTL if a global TTL is set.
    /// To have per-entry TTLs that override a default or work without one,
    //  you'd need `Cache::builder().expire_after(MyCustomExpiry::new())`
    //  or use `insert_with_expiry` if available and appropriate.
    /// For simplicity, this implementation relies on the cache's global TTL if configured.
    async fn set(&mut self, key: &str, value: &str, _ttl_seconds: u64) -> Result<()> {
        // Note: _ttl_seconds is marked as unused. If you need per-entry TTL that differs
        // from the cache's default, Moka requires a more complex Expiry policy or
        // using a different cache for entries with different TTL characteristics.
        // If options.ttl is 0 (meaning indefinite), then entries inserted here
        // will also be indefinite unless _ttl_seconds was used with a custom expiry.
        let prefixed_key = self.prefixed_key(key);
        let value_string = value.to_string();

        self.cache.insert(prefixed_key, value_string).await;
        Ok(())
    }

    async fn remove(&mut self, key: &str) -> Result<()> {
        let prefixed_key = self.prefixed_key(key);
        // Moka's invalidate does not return a value, it just removes the entry.
        self.cache.invalidate(&prefixed_key).await;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        // Moka's cache is in-memory and managed by RAII.
        // "Disconnecting" can mean clearing all entries.
        self.cache.invalidate_all();
        Ok(())
    }

    /// Returns the configured default TTL of the cache if a key exists,
    /// or None if the key doesn't exist or no default TTL is configured.
    async fn ttl(&mut self, key: &str) -> Result<Option<Duration>> {
        let prefixed_key = self.prefixed_key(key);
        if self.cache.contains_key(&prefixed_key) {
            // Check if key exists (Moka's contains_key is sync)
            if self.options.ttl > 0 {
                Ok(Some(Duration::from_secs(self.options.ttl)))
            } else {
                // No default TTL configured for the cache, so entry effectively has no TTL from this perspective
                Ok(None)
            }
        } else {
            Ok(None) // Key does not exist
        }
    }
}

impl MemoryCacheManager {
    /// Delete a key from the cache.
    pub async fn delete(&mut self, key: &str) -> Result<bool> {
        let prefixed_key = self.prefixed_key(key);
        // Moka's invalidate doesn't return if the key existed.
        // To match potential expectations of `delete` returning true if item was deleted:
        if self.cache.contains_key(&prefixed_key) {
            // Sync check, but common
            self.cache.invalidate(&prefixed_key).await;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get multiple keys at once.
    pub async fn get_many(&mut self, keys: &[&str]) -> Result<Vec<Option<String>>> {
        let mut results = Vec::with_capacity(keys.len());
        for &key in keys {
            results.push(self.get(key).await?);
        }
        Ok(results)
    }

    /// Set multiple key-value pairs at once.
    /// All pairs will use the cache's default TTL if configured.
    pub async fn set_many(&mut self, pairs: &[(&str, &str)], _ttl_seconds: u64) -> Result<()> {
        // As with `set`, _ttl_seconds is advisory here. All entries will use the cache's
        // configured default TTL (if any).
        for (key, value) in pairs {
            let prefixed_key = self.prefixed_key(key);
            let value_string = value.to_string();
            self.cache.insert(prefixed_key, value_string).await;
        }
        Ok(())
    }
}

// No Drop implementation needed as Moka's Cache handles its own resource cleanup.
