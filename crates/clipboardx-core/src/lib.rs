#![forbid(unsafe_code)]

//! Pure business domain engine for ClipboardX:
//! - Standard OS data/config directory resolution (zero extra dependencies)
//! - TOML configuration management
//! - Smart subtype detection & pinyin indexing
//! - Clipboard ingestion, deduplication, and payload reconstruction

pub mod archive;
pub mod config;
pub mod inspector;
pub mod paths;
pub mod pinyin_index;
pub mod search;
pub mod service;
pub mod vault_service;

pub use archive::{CbxArchive, CbxExportStats, CbxImportStats};
pub use config::ConfigStore;
pub use paths::AppPaths;
pub use service::ClipboardService;
pub use vault_service::VaultService;
