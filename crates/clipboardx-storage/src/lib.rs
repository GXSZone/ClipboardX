#![forbid(unsafe_code)]

//! Encrypted persistence layer for ClipboardX:
//! - SQLite3 Multiple Ciphers (`ChaCha20-Poly1305`) for `history.db` and `vault.db`
//! - Content-addressed `XChaCha20-Poly1305` encrypted blob storage
//! - OS keyring integration with `Zeroizing` secret wrappers

pub mod crypto;
pub mod db;
pub mod keyring;
pub mod migrations;
pub mod vault;

pub use crypto::{BlobStore, CryptoKeys, sha256_hex};
pub use db::HistoryStore;
pub use keyring::OsSecretStore;
pub use vault::VaultStore;
