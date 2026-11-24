use std::collections::BTreeMap;

use clipboardx_model::{UpdateChannel, UpdaterError};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Canonical manifest filename attached to each signed GitHub Release.
pub const RELEASE_MANIFEST_ASSET_NAME: &str = "clipboardx-manifest.json";

/// Official ClipboardX Ed25519 release verifying key (32-byte hex).
/// Override via `GitHubUpdateClient::with_public_key_hex` in custom forks or tests.
pub const DEFAULT_RELEASE_PUBLIC_KEY_HEX: &str =
    "ecc85dfdfcafe953e81df29f703af4a11143afeaefbc3a88b19664188bfb7079";

/// Platform-specific binary artifact metadata inside a signed release manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAsset {
    pub url: String,
    pub sha256_hex: String,
    pub signature_hex: String,
    pub size_bytes: u64,
}

/// Top-level release manifest parsed from `clipboardx-manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpdateManifest {
    pub schema_version: u32,
    pub version: String,
    pub channel: UpdateChannel,
    pub published_at: String,
    pub notes: String,
    pub assets: BTreeMap<String, ManifestAsset>,
}

/// Builds the deterministic canonical message signed by Ed25519 for each release asset.
pub fn canonical_asset_signing_bytes(version: &str, platform: &str, sha256_hex: &str) -> Vec<u8> {
    format!(
        "clipboardx-release-v1:{}:{}:{}",
        version.trim(),
        platform.trim(),
        sha256_hex.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

/// Verifies an Ed25519 detached signature over `message` using hex-encoded key and signature.
pub fn verify_ed25519_hex(
    verifying_key_hex: &str,
    message: &[u8],
    signature_hex: &str,
) -> Result<(), UpdaterError> {
    let key_bytes =
        hex::decode(verifying_key_hex.trim()).map_err(|_| UpdaterError::VerificationFailed)?;
    let key_array: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| UpdaterError::VerificationFailed)?;
    let verifying_key =
        VerifyingKey::from_bytes(&key_array).map_err(|_| UpdaterError::VerificationFailed)?;

    let sig_bytes =
        hex::decode(signature_hex.trim()).map_err(|_| UpdaterError::VerificationFailed)?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| UpdaterError::VerificationFailed)?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(message, &signature)
        .map_err(|_| UpdaterError::VerificationFailed)
}

/// Returns the canonical platform-arch key used in `SignedUpdateManifest::assets`.
pub const fn current_platform_key() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "windows-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-aarch64"
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

/// Parses a SemVer string such as `0.1.0`, `v0.2.1`, or `0.2.0-beta.1` and returns whether
/// `candidate` is strictly newer than `current`.
pub fn is_newer_semver(current: &str, candidate: &str) -> bool {
    let Some(cur) = ParsedSemver::parse(current) else {
        return false;
    };
    let Some(cand) = ParsedSemver::parse(candidate) else {
        return false;
    };
    cand > cur
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedSemver {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl ParsedSemver {
    fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match trimmed.split_once('-') {
            Some((c, p)) => (c, Some(p.to_owned())),
            None => (trimmed, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl Ord for ParsedSemver {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                // Stable release takes precedence over pre-release with the same numeric version.
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for ParsedSemver {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn semver_ordering_handles_stable_and_prerelease() {
        assert!(is_newer_semver("0.1.0", "v0.1.1"));
        assert!(is_newer_semver("0.1.0-beta.1", "0.1.0"));
        assert!(is_newer_semver("0.1.0-beta.1", "0.1.0-beta.2"));
        assert!(!is_newer_semver("0.2.0", "0.1.9"));
        assert!(!is_newer_semver("0.2.0", "0.2.0"));
        assert!(!is_newer_semver("invalid", "0.2.0"));
    }

    #[test]
    fn ed25519_signature_verification_rejects_tampering() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let msg = canonical_asset_signing_bytes("0.2.0", "windows-x86_64", sha);
        let sig_hex = hex::encode(signing_key.sign(&msg).to_bytes());

        assert!(verify_ed25519_hex(&verifying_hex, &msg, &sig_hex).is_ok());

        let tampered = canonical_asset_signing_bytes("0.2.1", "windows-x86_64", sha);
        assert!(matches!(
            verify_ed25519_hex(&verifying_hex, &tampered, &sig_hex),
            Err(UpdaterError::VerificationFailed)
        ));
    }
}
