use tsdb_types::model::DataPoint;
use crate::traits::{BusinessPlugin, QueryPlugin, StoragePlugin};
use std::collections::HashMap;

pub struct PluginRegistry {
    business_plugins: HashMap<String, Box<dyn BusinessPlugin>>,
    query_plugins: HashMap<String, Box<dyn QueryPlugin>>,
    storage_plugins: HashMap<String, Box<dyn StoragePlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            business_plugins: HashMap::new(),
            query_plugins: HashMap::new(),
            storage_plugins: HashMap::new(),
        }
    }

    pub fn register_business(&mut self, name: impl Into<String>, plugin: Box<dyn BusinessPlugin>) {
        self.business_plugins.insert(name.into(), plugin);
    }

    pub fn register_query(&mut self, name: impl Into<String>, plugin: Box<dyn QueryPlugin>) {
        self.query_plugins.insert(name.into(), plugin);
    }

    pub fn register_storage(&mut self, name: impl Into<String>, plugin: Box<dyn StoragePlugin>) {
        self.storage_plugins.insert(name.into(), plugin);
    }

    pub fn get_business(&self, name: &str) -> Option<&dyn BusinessPlugin> {
        self.business_plugins.get(name).map(|p| p.as_ref())
    }

    pub fn get_query(&self, name: &str) -> Option<&dyn QueryPlugin> {
        self.query_plugins.get(name).map(|p| p.as_ref())
    }

    pub fn get_storage(&self, name: &str) -> Option<&dyn StoragePlugin> {
        self.storage_plugins.get(name).map(|p| p.as_ref())
    }

    pub fn list_business(&self) -> Vec<&str> {
        self.business_plugins.keys().map(|s| s.as_str()).collect()
    }

    pub fn list_query(&self) -> Vec<&str> {
        self.query_plugins.keys().map(|s| s.as_str()).collect()
    }

    pub fn list_storage(&self) -> Vec<&str> {
        self.storage_plugins.keys().map(|s| s.as_str()).collect()
    }

    pub fn validate_data_point(&self, business: &str, dp: &DataPoint) -> bool {
        if let Some(plugin) = self.business_plugins.get(business) {
            plugin.validate(dp)
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBusinessPlugin;

    impl BusinessPlugin for TestBusinessPlugin {
        fn name(&self) -> &str { "test" }
        fn validate(&self, _dp: &DataPoint) -> bool { true }
        fn default_aggregations(&self) -> Vec<String> { vec!["avg".to_string()] }
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = PluginRegistry::new();
        registry.register_business("test", Box::new(TestBusinessPlugin));
        assert!(registry.get_business("test").is_some());
        assert!(registry.get_business("nonexistent").is_none());
    }
}
