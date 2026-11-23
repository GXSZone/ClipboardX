#![forbid(unsafe_code)]

//! ClipboardX offline-verified background updater:
//! zero-telemetry GitHub Releases manifest polling, Ed25519 + SHA-256 dual verification,
//! and atomic cross-platform executable replacement with rollback protection.

pub mod github;
pub mod installer;
pub mod manifest;

pub use github::GitHubUpdateClient;
pub use installer::{
    apply_atomic_binary_replacement, can_self_update_directory, cleanup_stale_backup,
    download_and_verify_release, verify_staged_file,
};
pub use manifest::{
    DEFAULT_RELEASE_PUBLIC_KEY_HEX, ManifestAsset, RELEASE_MANIFEST_ASSET_NAME,
    SignedUpdateManifest, canonical_asset_signing_bytes, current_platform_key, is_newer_semver,
    verify_ed25519_hex,
};
