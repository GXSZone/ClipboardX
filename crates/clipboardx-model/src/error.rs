use thiserror::Error;

/// Top-level standardized error type for ClipboardX.
#[derive(Debug, Error)]
pub enum ClipboardXError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("platform bridge error: {0}")]
    Platform(#[from] PlatformError),

    #[error("OCR error: {0}")]
    Ocr(#[from] OcrError),

    #[error("archive error: {0}")]
    Archive(#[from] ArchiveError),

    #[error("updater error: {0}")]
    Updater(#[from] UpdaterError),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("validation error: {0}")]
    Validation(String),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("keyring access failed: {0}")]
    Keyring(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("encryption/decryption failed or authentication tag mismatch: {0}")]
    Crypto(String),

    #[error("blob I/O error: {0}")]
    BlobIo(String),

    #[error("account vault is locked")]
    VaultLocked,

    #[error("invalid master password")]
    InvalidMasterPassword,

    #[error("record not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("clipboard operation failed: {0}")]
    Clipboard(String),

    #[error("global hotkey registration failed: {0}")]
    Hotkey(String),

    #[error("system tray initialization failed: {0}")]
    Tray(String),

    #[error("window control or focus restoration failed: {0}")]
    Window(String),

    #[error("input simulation failed: {0}")]
    InputSimulation(String),

    #[error("operation unsupported in current desktop session: {0}")]
    Unsupported(String),
}

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("OCR provider unavailable: {0}")]
    Unavailable(String),

    #[error("OCR recognition failed: {0}")]
    Inference(String),

    #[error("invalid OCR plugin manifest or checksum mismatch: {0}")]
    PluginInvalid(String),
}

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("invalid archive format: {0}")]
    InvalidFormat(String),

    #[error("incorrect passphrase or corrupted archive payload")]
    BadPassphrase,

    #[error("archive I/O error: {0}")]
    Io(String),
}

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("update request failed: {0}")]
    Network(String),

    #[error("signature or SHA-256 verification failed")]
    VerificationFailed,

    #[error("failed to apply update: {0}")]
    Install(String),
}

pub type Result<T> = std::result::Result<T, ClipboardXError>;
