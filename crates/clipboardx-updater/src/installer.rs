use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clipboardx_model::{UpdateReleaseInfo, UpdaterError};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::manifest::{canonical_asset_signing_bytes, verify_ed25519_hex};

/// Downloads a release artifact over HTTPS, verifies SHA-256 and Ed25519 signature,
/// extracts the binary if packaged as `.zip`, and returns the verified staged binary path.
pub fn download_and_verify_release(
    info: &UpdateReleaseInfo,
    staging_dir: &Path,
    public_key_hex: &str,
    platform_key: &str,
    executable_name: &str,
) -> Result<PathBuf, UpdaterError> {
    if !info
        .download_url
        .trim()
        .to_ascii_lowercase()
        .starts_with("https://")
    {
        return Err(UpdaterError::Network(
            "release download URL must use https://".to_owned(),
        ));
    }

    fs::create_dir_all(staging_dir).map_err(|err| UpdaterError::Install(err.to_string()))?;

    // Verify the detached signature before downloading untrusted bytes.
    let canonical = canonical_asset_signing_bytes(&info.version, platform_key, &info.sha256_hex);
    verify_ed25519_hex(public_key_hex, &canonical, &info.signature_hex)?;

    let part_path = staging_dir.join(format!("release-{}.part", sanitize_tag(&info.version)));
    let _ = fs::remove_file(&part_path);

    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("ClipboardX-Updater")
        .build()
        .map_err(|err| UpdaterError::Network(err.to_string()))?;

    let mut response = client
        .get(&info.download_url)
        .send()
        .map_err(|err| UpdaterError::Network(err.to_string()))?
        .error_for_status()
        .map_err(|err| UpdaterError::Network(err.to_string()))?;

    let mut out = File::create(&part_path).map_err(|err| UpdaterError::Install(err.to_string()))?;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let read = response
            .read(&mut buf)
            .map_err(|err| UpdaterError::Network(err.to_string()))?;
        if read == 0 {
            break;
        }
        out.write_all(&buf[..read])
            .map_err(|err| UpdaterError::Install(err.to_string()))?;
    }
    out.flush()
        .map_err(|err| UpdaterError::Install(err.to_string()))?;
    drop(out);

    verify_staged_file(
        &part_path,
        info,
        staging_dir,
        public_key_hex,
        platform_key,
        executable_name,
    )
}

/// Verifies an already-downloaded local artifact against SHA-256 and Ed25519 signature,
/// extracting the binary payload from `.zip` packages if applicable.
pub fn verify_staged_file(
    artifact_path: &Path,
    info: &UpdateReleaseInfo,
    staging_dir: &Path,
    public_key_hex: &str,
    platform_key: &str,
    executable_name: &str,
) -> Result<PathBuf, UpdaterError> {
    let canonical = canonical_asset_signing_bytes(&info.version, platform_key, &info.sha256_hex);
    verify_ed25519_hex(public_key_hex, &canonical, &info.signature_hex)?;

    let actual_sha = compute_file_sha256(artifact_path)?;
    if !actual_sha.eq_ignore_ascii_case(info.sha256_hex.trim()) {
        let _ = fs::remove_file(artifact_path);
        return Err(UpdaterError::VerificationFailed);
    }

    fs::create_dir_all(staging_dir).map_err(|err| UpdaterError::Install(err.to_string()))?;
    let staged_binary = staging_dir.join(format!(
        "staged-{}-{executable_name}",
        sanitize_tag(&info.version)
    ));
    let _ = fs::remove_file(&staged_binary);

    if is_zip_archive(artifact_path)? {
        extract_binary_from_zip(artifact_path, executable_name, &staged_binary)?;
    } else {
        fs::copy(artifact_path, &staged_binary)
            .map_err(|err| UpdaterError::Install(err.to_string()))?;
    }

    ensure_executable_permissions(&staged_binary)?;
    Ok(staged_binary)
}

/// Atomically replaces `target_binary` with `staged_binary`, preserving a `.old` backup
/// and rolling back automatically if the replacement write fails.
pub fn apply_atomic_binary_replacement(
    staged_binary: &Path,
    target_binary: &Path,
) -> Result<PathBuf, UpdaterError> {
    let meta = fs::metadata(staged_binary)
        .map_err(|err| UpdaterError::Install(format!("staged binary inaccessible: {err}")))?;
    if meta.len() == 0 {
        return Err(UpdaterError::Install("staged binary is empty".to_owned()));
    }

    let backup_path = backup_path_for(target_binary);
    let _ = fs::remove_file(&backup_path);

    let had_target = target_binary.exists();
    if had_target {
        fs::rename(target_binary, &backup_path).map_err(|err| {
            UpdaterError::Install(format!(
                "cannot move current executable aside (check folder write permissions): {err}"
            ))
        })?;
    }

    // Attempt atomic rename first (succeeds on same volume); fall back to copy + sync.
    let write_res = match fs::rename(staged_binary, target_binary) {
        Ok(()) => Ok(()),
        Err(_) => fs::copy(staged_binary, target_binary).map(|_| ()),
    };

    if let Err(err) = write_res {
        if had_target && backup_path.exists() {
            let _ = fs::rename(&backup_path, target_binary);
        }
        return Err(UpdaterError::Install(format!(
            "failed to install updated binary: {err}"
        )));
    }

    ensure_executable_permissions(target_binary)?;
    Ok(backup_path)
}

/// Cleans up leftover `.old` executable backups from a prior successful restart update.
pub fn cleanup_stale_backup(current_exe: &Path) {
    let backup = backup_path_for(current_exe);
    if backup.exists() {
        let _ = fs::remove_file(backup);
    }
}

/// Checks whether the directory containing the current executable permits in-place updates.
pub fn can_self_update_directory(exe_path: &Path) -> bool {
    let Some(parent) = exe_path.parent() else {
        return false;
    };
    let probe = parent.join(".clipboardx-update-probe");
    match File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn backup_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clipboardx-app");
    target.with_file_name(format!("{name}.old"))
}

fn compute_file_sha256(path: &Path) -> Result<String, UpdaterError> {
    let mut file = File::open(path).map_err(|err| UpdaterError::Install(err.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|err| UpdaterError::Install(err.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn is_zip_archive(path: &Path) -> Result<bool, UpdaterError> {
    let mut file = File::open(path).map_err(|err| UpdaterError::Install(err.to_string()))?;
    let mut magic = [0u8; 4];
    let n = file
        .read(&mut magic)
        .map_err(|err| UpdaterError::Install(err.to_string()))?;
    Ok(n == 4 && &magic == b"PK\x03\x04")
}

fn extract_binary_from_zip(
    zip_path: &Path,
    executable_name: &str,
    dest_path: &Path,
) -> Result<(), UpdaterError> {
    let file = File::open(zip_path).map_err(|err| UpdaterError::Install(err.to_string()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|err| UpdaterError::Install(err.to_string()))?;

    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|err| UpdaterError::Install(err.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(safe_path) = entry.enclosed_name() else {
            continue;
        };
        let matches = safe_path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| name == executable_name);
        if !matches {
            continue;
        }

        let mut out =
            File::create(dest_path).map_err(|err| UpdaterError::Install(err.to_string()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|err| UpdaterError::Install(err.to_string()))?;
        return Ok(());
    }

    Err(UpdaterError::Install(format!(
        "executable '{executable_name}' not found inside release archive"
    )))
}

fn ensure_executable_permissions(path: &Path) -> Result<(), UpdaterError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms).map_err(|err| UpdaterError::Install(err.to_string()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn sanitize_tag(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn verifies_zip_release_rejects_bad_hash_and_replaces_atomically() {
        let root = std::env::temp_dir().join(format!(
            "clipboardx-updater-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).ok();

        let zip_path = root.join("release.zip");
        {
            let file = File::create(&zip_path).ok();
            assert!(file.is_some());
            if let Some(f) = file {
                let mut zw = zip::ZipWriter::new(f);
                let _ = zw.start_file("clipboardx-app.exe", SimpleFileOptions::default());
                let _ = zw.write_all(b"NEW-BINARY-PAYLOAD-V2");
                let _ = zw.finish();
            }
        }

        let sha = compute_file_sha256(&zip_path).unwrap_or_default();
        let signing_key = SigningKey::from_bytes(&[23u8; 32]);
        let pub_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let msg = canonical_asset_signing_bytes("0.2.0", "windows-x86_64", &sha);
        let sig_hex = hex::encode(signing_key.sign(&msg).to_bytes());

        let info = UpdateReleaseInfo {
            version: "0.2.0".to_owned(),
            notes: "Test release".to_owned(),
            download_url: "https://example.com/release.zip".to_owned(),
            sha256_hex: sha,
            signature_hex: sig_hex,
        };

        let staging = root.join("staging");
        let staged = verify_staged_file(
            &zip_path,
            &info,
            &staging,
            &pub_hex,
            "windows-x86_64",
            "clipboardx-app.exe",
        );
        assert!(staged.is_ok());

        if let Ok(staged_bin) = staged {
            let target_bin = root.join("clipboardx-app.exe");
            let _ = fs::write(&target_bin, b"OLD-BINARY-V1");

            let backup = apply_atomic_binary_replacement(&staged_bin, &target_bin);
            assert!(backup.is_ok());
            assert_eq!(
                fs::read(&target_bin).ok().as_deref(),
                Some(b"NEW-BINARY-PAYLOAD-V2".as_slice())
            );
            assert_eq!(
                backup
                    .as_ref()
                    .ok()
                    .and_then(|p| fs::read(p).ok())
                    .as_deref(),
                Some(b"OLD-BINARY-V1".as_slice())
            );

            cleanup_stale_backup(&target_bin);
            assert!(backup.as_ref().ok().is_some_and(|p| !p.exists()));
        }

        // Tampered SHA-256 must fail verification and delete untrusted artifact.
        let bad_path = root.join("tampered.zip");
        let _ = fs::write(&bad_path, b"corrupted");
        let bad_res = verify_staged_file(
            &bad_path,
            &info,
            &staging,
            &pub_hex,
            "windows-x86_64",
            "clipboardx-app.exe",
        );
        assert!(matches!(bad_res, Err(UpdaterError::VerificationFailed)));
        assert!(!bad_path.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
