// src/app/memory_manager.rs
use super::config::App;
use crate::app::manager::AppManager;
use crate::error::Result;
use async_trait::async_trait;
use dashmap::DashMap;

pub struct MemoryAppManager {
    apps: DashMap<String, App>,
}

impl Default for MemoryAppManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAppManager {
    pub fn new() -> Self {
        Self {
            apps: DashMap::new(),
        }
    }
}

#[async_trait]
impl AppManager for MemoryAppManager {
    async fn init(&self) -> Result<()> {
        // In-memory implementation doesn't need initialization
        Ok(())
    }

    async fn create_app(&self, config: App) -> Result<()> {
        self.apps.insert(config.id.clone(), config);
        Ok(())
    }

    async fn update_app(&self, config: App) -> Result<()> {
        self.apps.insert(config.id.clone(), config);
        Ok(())
    }

    async fn delete_app(&self, app_id: &str) -> Result<()> {
        self.apps.remove(app_id);
        Ok(())
    }

    async fn get_apps(&self) -> Result<Vec<App>> {
        let apps = self
            .apps
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        Ok(apps)
    }
    async fn find_by_key(&self, key: &str) -> Result<Option<App>> {
        let app = self
            .apps
            .iter()
            .find(|app| app.key == key)
            .map(|app| app.clone());
        Ok(app)
    }

    async fn find_by_id(&self, app_id: &str) -> Result<Option<App>> {
        Ok(self.apps.get(app_id).map(|app| app.clone()))
    }

    async fn check_health(&self) -> Result<()> {
        Ok(())
    }
}
