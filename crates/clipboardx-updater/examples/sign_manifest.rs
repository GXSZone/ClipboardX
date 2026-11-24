use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use clipboardx_model::UpdateChannel;
use clipboardx_updater::{
    DEFAULT_RELEASE_PUBLIC_KEY_HEX, ManifestAsset, RELEASE_MANIFEST_ASSET_NAME,
    SignedUpdateManifest, canonical_asset_signing_bytes, verify_ed25519_hex,
};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const PLATFORM_ASSETS: &[(&str, &str)] = &[
    ("windows-x86_64", "clipboardx-windows-x86_64-portable.zip"),
    ("macos-aarch64", "clipboardx-macos-aarch64.zip"),
    ("linux-x86_64", "clipboardx-linux-x86_64.AppImage"),
];

fn sha256_file_hex(path: &Path) -> Result<(String, u64), Box<dyn Error>> {
    let mut file = File::open(path)?;
    let size_bytes = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok((hex::encode(hasher.finalize()), size_bytes))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 5 {
        return Err(
            "usage: sign_manifest <tag> <owner/repo> <artifacts_dir> <published_at> [notes]".into(),
        );
    }

    let tag = args[1].trim();
    let repo = args[2].trim();
    let artifacts_dir = Path::new(&args[3]);
    let published_at = args[4].trim().to_owned();
    let notes = args
        .get(5)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("ClipboardX {tag}"));

    let key_hex = env::var("CLIPBOARDX_RELEASE_SIGNING_KEY")?;
    let seed_bytes = hex::decode(key_hex.trim())?;
    let seed_array: [u8; 32] = seed_bytes
        .try_into()
        .map_err(|_| "CLIPBOARDX_RELEASE_SIGNING_KEY must be 64 hex chars (32 bytes)")?;
    let signing_key = SigningKey::from_bytes(&seed_array);

    let derived_pub = hex::encode(signing_key.verifying_key().to_bytes());
    if !derived_pub.eq_ignore_ascii_case(DEFAULT_RELEASE_PUBLIC_KEY_HEX) {
        return Err(format!(
            "signing key public half ({derived_pub}) does not match DEFAULT_RELEASE_PUBLIC_KEY_HEX ({DEFAULT_RELEASE_PUBLIC_KEY_HEX})"
        )
        .into());
    }

    let version = tag.trim_start_matches(['v', 'V']).to_owned();
    let lower_ver = version.to_ascii_lowercase();
    let channel =
        if lower_ver.contains("-beta") || lower_ver.contains("-rc") || lower_ver.contains("-alpha")
        {
            UpdateChannel::Beta
        } else {
            UpdateChannel::Stable
        };

    let mut assets = BTreeMap::new();
    for (platform_key, filename) in PLATFORM_ASSETS {
        let asset_path = artifacts_dir.join(filename);
        if !asset_path.is_file() {
            continue;
        }
        let (sha256_hex, size_bytes) = sha256_file_hex(&asset_path)?;
        let signing_msg = canonical_asset_signing_bytes(&version, platform_key, &sha256_hex);
        let signature_hex = hex::encode(signing_key.sign(&signing_msg).to_bytes());
        verify_ed25519_hex(DEFAULT_RELEASE_PUBLIC_KEY_HEX, &signing_msg, &signature_hex)?;

        assets.insert(
            (*platform_key).to_owned(),
            ManifestAsset {
                url: format!("https://github.com/{repo}/releases/download/{tag}/{filename}"),
                sha256_hex,
                signature_hex,
                size_bytes,
            },
        );
    }

    if assets.is_empty() {
        return Err(format!(
            "no supported assets found under {}",
            artifacts_dir.display()
        )
        .into());
    }

    let manifest = SignedUpdateManifest {
        schema_version: 1,
        version,
        channel,
        published_at,
        notes,
        assets,
    };

    let out_path = artifacts_dir.join(RELEASE_MANIFEST_ASSET_NAME);
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&out_path, json)?;
    println!("Signed update manifest generated at {}", out_path.display());
    Ok(())
}
