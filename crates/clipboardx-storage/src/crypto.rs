use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use clipboardx_model::StorageError;
use hkdf::Hkdf;
use image::ImageFormat;
use rand::{Rng, RngExt};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const BLOB_MAGIC: &[u8; 4] = b"CXB1";
const NONCE_LEN: usize = 24;
const THUMB_MAX_EDGE: u32 = 320;

/// Derived subkeys expanded from the 32-byte master key via HKDF-SHA256.
pub struct CryptoKeys {
    pub db_key_hex: Zeroizing<String>,
    pub blob_key: Zeroizing<[u8; 32]>,
}

impl CryptoKeys {
    /// Expands distinct domain-separated subkeys from a 32-byte master key.
    pub fn derive_from_master(master_key: &[u8]) -> Result<Self, StorageError> {
        let hk = Hkdf::<Sha256>::new(Some(b"clipboardx.hkdf.salt.v1"), master_key);

        let mut db_raw = Zeroizing::new([0u8; 32]);
        hk.expand(b"clipboardx.db.key.v1", db_raw.as_mut())
            .map_err(|err| StorageError::Crypto(format!("HKDF db key expand failed: {err}")))?;

        let mut blob_key = Zeroizing::new([0u8; 32]);
        hk.expand(b"clipboardx.blob.key.v1", blob_key.as_mut())
            .map_err(|err| StorageError::Crypto(format!("HKDF blob key expand failed: {err}")))?;

        let db_key_hex = Zeroizing::new(hex::encode(*db_raw));
        Ok(Self {
            db_key_hex,
            blob_key,
        })
    }
}

/// Computes lowercase hex-encoded SHA-256 digest of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Content-addressed encrypted blob store (`blobs/<2-hex-prefix>/<sha256>.enc`).
#[derive(Clone)]
pub struct BlobStore {
    root_dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl BlobStore {
    pub fn new(root_dir: PathBuf, blob_key: Zeroizing<[u8; 32]>) -> Result<Self, StorageError> {
        fs::create_dir_all(&root_dir)
            .map_err(|err| StorageError::BlobIo(format!("create blob root failed: {err}")))?;
        Ok(Self {
            root_dir,
            key: blob_key,
        })
    }

    /// Encrypts and writes content by its SHA-256 hash, returning the content hash.
    pub fn put_bytes(&self, plaintext: &[u8]) -> Result<String, StorageError> {
        let hash = sha256_hex(plaintext);
        let path = self.blob_path(&hash)?;
        if path.exists() {
            return Ok(hash);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| StorageError::BlobIo(format!("create shard dir failed: {err}")))?;
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);

        let cipher = XChaCha20Poly1305::new((&*self.key).into());
        let nonce = XNonce::from(nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: hash.as_bytes(),
                },
            )
            .map_err(|err| StorageError::Crypto(format!("blob encrypt failed: {err}")))?;

        let mut file_buf = Vec::with_capacity(BLOB_MAGIC.len() + NONCE_LEN + ciphertext.len());
        file_buf.extend_from_slice(BLOB_MAGIC);
        file_buf.extend_from_slice(&nonce_bytes);
        file_buf.extend_from_slice(&ciphertext);

        // Write atomically via temp file in the same directory to prevent torn blobs.
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &file_buf)
            .map_err(|err| StorageError::BlobIo(format!("write temp blob failed: {err}")))?;
        fs::rename(&tmp_path, &path)
            .map_err(|err| StorageError::BlobIo(format!("rename blob failed: {err}")))?;

        Ok(hash)
    }

    /// Reads, authenticates, and decrypts a blob referenced by its SHA-256 hash.
    pub fn get_bytes(&self, hash: &str) -> Result<Vec<u8>, StorageError> {
        let path = self.blob_path(hash)?;
        let raw = fs::read(&path)
            .map_err(|err| StorageError::BlobIo(format!("read blob {hash} failed: {err}")))?;

        let header_len = BLOB_MAGIC.len() + NONCE_LEN;
        if raw.len() <= header_len || &raw[..BLOB_MAGIC.len()] != BLOB_MAGIC {
            return Err(StorageError::Crypto("invalid blob header".to_owned()));
        }

        let nonce = XNonce::try_from(&raw[BLOB_MAGIC.len()..header_len])
            .map_err(|_| StorageError::Crypto("invalid blob nonce length".to_owned()))?;
        let ciphertext = &raw[header_len..];

        let cipher = XChaCha20Poly1305::new((&*self.key).into());
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: hash.as_bytes(),
                },
            )
            .map_err(|err| StorageError::Crypto(format!("blob decrypt failed: {err}")))
    }

    /// Generates and stores an encrypted PNG thumbnail (max edge 320px) for image payloads.
    pub fn put_image_thumbnail(&self, png_bytes: &[u8]) -> Result<String, StorageError> {
        let decoded = image::load_from_memory(png_bytes)
            .map_err(|err| StorageError::BlobIo(format!("decode image for thumb failed: {err}")))?;
        let thumb = decoded.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);
        let mut out = Cursor::new(Vec::new());
        thumb
            .write_to(&mut out, ImageFormat::Png)
            .map_err(|err| StorageError::BlobIo(format!("encode thumb failed: {err}")))?;
        self.put_bytes(out.get_ref())
    }

    /// Deletes a blob file if it exists on disk.
    pub fn remove_if_exists(&self, hash: &str) -> Result<(), StorageError> {
        let path = self.blob_path(hash)?;
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| StorageError::BlobIo(format!("remove blob failed: {err}")))?;
        }
        Ok(())
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    fn blob_path(&self, hash: &str) -> Result<PathBuf, StorageError> {
        if hash.len() < 4 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StorageError::BlobIo("malformed content hash".to_owned()));
        }
        Ok(self.root_dir.join(&hash[..2]).join(format!("{hash}.enc")))
    }
}

/// Generates a random password containing uppercase, lowercase, digit, and symbol characters.
pub fn generate_strong_password(length: usize) -> String {
    const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
    const DIGIT: &[u8] = b"23456789";
    const SYMBOL: &[u8] = b"!@#$%^&*-_+=?";
    const ALL: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_+=?";

    let target_len = length.max(12);
    let mut rng = rand::rng();
    let mut chars = Vec::with_capacity(target_len);
    chars.push(UPPER[rng.random_range(0..UPPER.len())] as char);
    chars.push(LOWER[rng.random_range(0..LOWER.len())] as char);
    chars.push(DIGIT[rng.random_range(0..DIGIT.len())] as char);
    chars.push(SYMBOL[rng.random_range(0..SYMBOL.len())] as char);
    while chars.len() < target_len {
        chars.push(ALL[rng.random_range(0..ALL.len())] as char);
    }
    for i in (1..chars.len()).rev() {
        let j = rng.random_range(0..=i);
        chars.swap(i, j);
    }
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clipboardx_test_{name}_{}", rand::random::<u64>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn blob_roundtrip_and_aad_tamper_detection() {
        let dir = temp_dir("blob");
        let master = [0x42u8; 32];
        let keys = CryptoKeys::derive_from_master(&master).unwrap();
        let store = BlobStore::new(dir.clone(), keys.blob_key).unwrap();

        let secret = b"super-secret-clipboard-payload-12345";
        let hash = store.put_bytes(secret).unwrap();
        let loaded = store.get_bytes(&hash).unwrap();
        assert_eq!(loaded, secret);

        // Disk file must not contain plaintext bytes.
        let on_disk = fs::read(dir.join(&hash[..2]).join(format!("{hash}.enc"))).unwrap();
        assert!(!on_disk.windows(secret.len()).any(|w| w == secret));

        // Corrupting ciphertext must fail AEAD verification.
        let enc_path = dir.join(&hash[..2]).join(format!("{hash}.enc"));
        let mut corrupted = on_disk;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        fs::write(&enc_path, corrupted).unwrap();
        assert!(store.get_bytes(&hash).is_err());

        let _ = fs::remove_dir_all(dir);
    }
}
