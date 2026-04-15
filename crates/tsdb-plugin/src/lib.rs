pub mod registry;
pub mod traits;

pub use traits::{BusinessPlugin, QueryPlugin, StoragePlugin};
pub use registry::PluginRegistry;
