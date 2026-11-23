use std::time::Duration;

use clipboardx_model::{UpdateChannel, UpdateClient, UpdateReleaseInfo, UpdaterError};
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::manifest::{
    DEFAULT_RELEASE_PUBLIC_KEY_HEX, RELEASE_MANIFEST_ASSET_NAME, SignedUpdateManifest,
    canonical_asset_signing_bytes, current_platform_key, is_newer_semver, verify_ed25519_hex,
};

const DEFAULT_GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/GXSZone/ClipboardX/releases";

#[derive(Debug, Deserialize)]
struct GitHubReleaseDto {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAssetDto>,
}

#[derive(Debug, Deserialize)]
struct GitHubAssetDto {
    name: String,
    browser_download_url: String,
}

/// Zero-telemetry GitHub Releases updater checking Ed25519-signed `clipboardx-manifest.json`.
#[derive(Debug, Clone)]
pub struct GitHubUpdateClient {
    current_version: String,
    platform_key: String,
    public_key_hex: String,
    releases_api_url: String,
}

impl GitHubUpdateClient {
    pub fn new(current_version: impl Into<String>) -> Self {
        Self {
            current_version: current_version.into(),
            platform_key: current_platform_key().to_owned(),
            public_key_hex: DEFAULT_RELEASE_PUBLIC_KEY_HEX.to_owned(),
            releases_api_url: DEFAULT_GITHUB_RELEASES_URL.to_owned(),
        }
    }

    pub fn with_public_key_hex(mut self, key_hex: impl Into<String>) -> Self {
        self.public_key_hex = key_hex.into();
        self
    }

    pub fn with_releases_api_url(mut self, url: impl Into<String>) -> Self {
        self.releases_api_url = url.into();
        self
    }

    pub fn with_platform_key(mut self, platform_key: impl Into<String>) -> Self {
        self.platform_key = platform_key.into();
        self
    }

    pub fn public_key_hex(&self) -> &str {
        &self.public_key_hex
    }

    pub fn platform_key(&self) -> &str {
        &self.platform_key
    }

    /// Evaluates a fetched signed manifest JSON against current version, channel, and public key.
    pub fn evaluate_manifest_json(
        &self,
        manifest_json: &str,
        channel: UpdateChannel,
    ) -> Result<Option<UpdateReleaseInfo>, UpdaterError> {
        let manifest: SignedUpdateManifest = serde_json::from_str(manifest_json)
            .map_err(|err| UpdaterError::Network(format!("invalid release manifest: {err}")))?;

        if manifest.schema_version != 1 {
            return Err(UpdaterError::VerificationFailed);
        }

        if channel == UpdateChannel::Stable && manifest.channel == UpdateChannel::Beta {
            return Ok(None);
        }

        if !is_newer_semver(&self.current_version, &manifest.version) {
            return Ok(None);
        }

        let Some(asset) = manifest.assets.get(&self.platform_key) else {
            return Ok(None);
        };

        if !asset
            .url
            .trim()
            .to_ascii_lowercase()
            .starts_with("https://")
        {
            return Err(UpdaterError::VerificationFailed);
        }

        let msg =
            canonical_asset_signing_bytes(&manifest.version, &self.platform_key, &asset.sha256_hex);
        verify_ed25519_hex(&self.public_key_hex, &msg, &asset.signature_hex)?;

        Ok(Some(UpdateReleaseInfo {
            version: manifest.version,
            notes: manifest.notes,
            download_url: asset.url.clone(),
            sha256_hex: asset.sha256_hex.clone(),
            signature_hex: asset.signature_hex.clone(),
        }))
    }
}

impl UpdateClient for GitHubUpdateClient {
    fn check_latest(
        &self,
        channel: UpdateChannel,
    ) -> Result<Option<UpdateReleaseInfo>, UpdaterError> {
        if !self.releases_api_url.starts_with("https://") {
            return Err(UpdaterError::Network(
                "update endpoint must use HTTPS".to_owned(),
            ));
        }

        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(format!("ClipboardX/{}", self.current_version))
            .build()
            .map_err(|err| UpdaterError::Network(err.to_string()))?;

        let releases: Vec<GitHubReleaseDto> = http
            .get(&self.releases_api_url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|err| UpdaterError::Network(err.to_string()))?
            .error_for_status()
            .map_err(|err| UpdaterError::Network(err.to_string()))?
            .json()
            .map_err(|err| UpdaterError::Network(err.to_string()))?;

        for rel in releases {
            if rel.draft {
                continue;
            }
            if channel == UpdateChannel::Stable && rel.prerelease {
                continue;
            }
            if !is_newer_semver(&self.current_version, &rel.tag_name) {
                continue;
            }
            let Some(manifest_asset) = rel
                .assets
                .iter()
                .find(|a| a.name == RELEASE_MANIFEST_ASSET_NAME)
            else {
                continue;
            };
            if !manifest_asset.browser_download_url.starts_with("https://") {
                continue;
            }
            let manifest_json = http
                .get(&manifest_asset.browser_download_url)
                .send()
                .map_err(|err| UpdaterError::Network(err.to_string()))?
                .error_for_status()
                .map_err(|err| UpdaterError::Network(err.to_string()))?
                .text()
                .map_err(|err| UpdaterError::Network(err.to_string()))?;

            if let Some(verified) = self.evaluate_manifest_json(&manifest_json, channel)? {
                return Ok(Some(verified));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signer, SigningKey};

    use super::*;
    use crate::manifest::ManifestAsset;

    #[test]
    fn evaluates_and_verifies_signed_manifest_respecting_channel() {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let pub_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sig_hex = hex::encode(
            signing_key
                .sign(&canonical_asset_signing_bytes(
                    "0.2.0",
                    "windows-x86_64",
                    sha,
                ))
                .to_bytes(),
        );

        let mut assets = BTreeMap::new();
        assets.insert(
            "windows-x86_64".to_owned(),
            ManifestAsset {
                url: "https://github.com/GXSZone/ClipboardX/releases/download/v0.2.0/clipboardx-windows-x86_64.zip".to_owned(),
                sha256_hex: sha.to_owned(),
                signature_hex: sig_hex,
                size_bytes: 4096,
            },
        );

        let manifest = SignedUpdateManifest {
            schema_version: 1,
            version: "0.2.0".to_owned(),
            channel: UpdateChannel::Beta,
            published_at: "2026-11-23T00:00:00Z".to_owned(),
            notes: "Beta fixes".to_owned(),
            assets,
        };
        let json = serde_json::to_string(&manifest).unwrap_or_default();

        let client = GitHubUpdateClient::new("0.1.0")
            .with_platform_key("windows-x86_64")
            .with_public_key_hex(pub_hex);

        // Stable channel skips beta manifest.
        assert!(matches!(
            client.evaluate_manifest_json(&json, UpdateChannel::Stable),
            Ok(None)
        ));

        // Beta channel verifies Ed25519 signature and returns release info.
        let found = client
            .evaluate_manifest_json(&json, UpdateChannel::Beta)
            .ok()
            .flatten();
        assert_eq!(found.as_ref().map(|r| r.version.as_str()), Some("0.2.0"));
    }
}
