use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clipboardx_model::OcrError;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::plugin::{JsonRpcOcrProvider, OcrPluginManifest, OcrPluginProtocol};

const RAPIDOCR_PLUGIN_ID: &str = "rapidocr.onnx";
const RAPIDOCR_RELEASE_VERSION: &str = "0.2.0";
const RAPIDOCR_UPSTREAM_WIN_URL: &str =
    "https://github.com/hiroi-sora/RapidOCR-json/releases/download/v0.2.0/RapidOCR-json_v0.2.0.7z";
const RAPIDOCR_MIRROR_URLS: &[&str] = &[
    RAPIDOCR_UPSTREAM_WIN_URL,
    "https://gh-proxy.org/https://github.com/hiroi-sora/RapidOCR-json/releases/download/v0.2.0/RapidOCR-json_v0.2.0.7z",
    "https://ghfast.top/https://github.com/hiroi-sora/RapidOCR-json/releases/download/v0.2.0/RapidOCR-json_v0.2.0.7z",
];
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Locally installed OCR plugin metadata resolved from `<plugins_dir>/<id>/plugin.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledOcrPlugin {
    pub manifest: OcrPluginManifest,
    pub install_dir: PathBuf,
}

/// Manages local ZIP import, authenticated HTTPS plugin downloads,
/// SHA-256 verification, hot loading, and clean uninstallation.
#[derive(Debug, Clone)]
pub struct OcrPluginManager {
    plugins_dir: PathBuf,
}

impl OcrPluginManager {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub fn plugins_dir(&self) -> &Path {
        &self.plugins_dir
    }

    /// Scans `<plugins_dir>` and returns all valid installed OCR plugins.
    pub fn list_installed(&self) -> Result<Vec<InstalledOcrPlugin>, OcrError> {
        if !self.plugins_dir.exists() {
            return Ok(Vec::new());
        }

        let entries = fs::read_dir(&self.plugins_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("read plugins dir failed: {err}")))?;

        let mut list = Vec::new();
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join("plugin.json");
            if !manifest_path.exists() {
                continue;
            }
            if let Ok(raw) = fs::read(&manifest_path)
                && let Ok(manifest) = serde_json::from_slice::<OcrPluginManifest>(&raw)
            {
                list.push(InstalledOcrPlugin {
                    manifest,
                    install_dir: dir,
                });
            }
        }
        list.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(list)
    }

    /// Instantiates a JSON-RPC worker provider for an installed plugin ID.
    pub fn load_provider(&self, plugin_id: &str) -> Result<JsonRpcOcrProvider, OcrError> {
        let installed = self
            .list_installed()?
            .into_iter()
            .find(|p| p.manifest.id == plugin_id)
            .ok_or_else(|| {
                OcrError::Unavailable(format!("OCR plugin not installed: {plugin_id}"))
            })?;
        Ok(JsonRpcOcrProvider::new(
            installed.manifest,
            installed.install_dir,
        ))
    }

    /// Installs a plugin from a local `.zip` or `.7z` archive with optional SHA-256 verification.
    pub fn install_from_local_zip(
        &self,
        zip_path: &Path,
        expected_sha256: Option<&str>,
    ) -> Result<InstalledOcrPlugin, OcrError> {
        if !zip_path.exists() {
            return Err(OcrError::PluginInvalid(format!(
                "plugin archive not found: {}",
                zip_path.display()
            )));
        }

        if let Some(expected) = expected_sha256.filter(|s| !s.trim().is_empty()) {
            verify_file_sha256(zip_path, expected)?;
        }

        if is_7z_archive(zip_path)? {
            return self.extract_rapidocr_7z_archive(zip_path);
        }

        self.extract_verified_zip(zip_path)
    }

    /// Downloads an OCR plugin package over HTTPS, enforces strict SHA-256 checksum
    /// verification before extraction, and installs it under `<plugins_dir>`.
    pub fn install_from_url(
        &self,
        url: &str,
        expected_sha256: &str,
    ) -> Result<InstalledOcrPlugin, OcrError> {
        self.install_from_url_with_progress(url, expected_sha256, |_, _, _| {})
    }

    /// Downloads an OCR plugin package over HTTPS while reporting streaming progress
    /// `(downloaded_bytes, total_bytes, stage)` before verifying SHA-256 and extracting.
    pub fn install_from_url_with_progress(
        &self,
        url: &str,
        expected_sha256: &str,
        mut on_progress: impl FnMut(u64, Option<u64>, &'static str),
    ) -> Result<InstalledOcrPlugin, OcrError> {
        let trimmed_url = url.trim();
        if !trimmed_url.to_ascii_lowercase().starts_with("https://") {
            return Err(OcrError::PluginInvalid(
                "remote OCR plugin download requires https:// URL".to_owned(),
            ));
        }

        let expected = expected_sha256.trim();
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(OcrError::PluginInvalid(
                "remote plugin installation requires a valid 64-char SHA-256 checksum".to_owned(),
            ));
        }

        fs::create_dir_all(&self.plugins_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("create plugins dir failed: {err}")))?;

        let tmp_zip = self
            .plugins_dir
            .join(format!(".download_{}.zip.tmp", std::process::id()));

        on_progress(0, None, "connecting");

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|err| OcrError::PluginInvalid(format!("build HTTP client failed: {err}")))?;

        let mut resp = client
            .get(trimmed_url)
            .send()
            .map_err(|err| OcrError::PluginInvalid(format!("download plugin failed: {err}")))?;
        if !resp.status().is_success() {
            return Err(OcrError::PluginInvalid(format!(
                "plugin download HTTP status {}",
                resp.status()
            )));
        }

        let total_bytes = resp.content_length();
        let mut downloaded = 0u64;
        {
            use std::io::Write;
            let mut out = File::create(&tmp_zip)
                .map_err(|err| OcrError::PluginInvalid(format!("create temp zip failed: {err}")))?;
            let mut buf = [0u8; 16 * 1024];
            loop {
                let n = resp.read(&mut buf).map_err(|err| {
                    OcrError::PluginInvalid(format!("read download stream failed: {err}"))
                })?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|err| {
                    OcrError::PluginInvalid(format!("write download stream failed: {err}"))
                })?;
                downloaded = downloaded.saturating_add(n as u64);
                on_progress(downloaded, total_bytes, "downloading");
            }
        }

        on_progress(downloaded, total_bytes.or(Some(downloaded)), "verifying");
        let verify_res = verify_file_sha256(&tmp_zip, expected);
        if let Err(err) = verify_res {
            let _ = fs::remove_file(&tmp_zip);
            return Err(err);
        }

        on_progress(downloaded, total_bytes.or(Some(downloaded)), "extracting");
        let install_res = self.extract_verified_zip(&tmp_zip);
        let _ = fs::remove_file(&tmp_zip);
        install_res
    }

    /// Downloads and installs the official upstream `RapidOCR-json` offline engine,
    /// falling back across mirror URLs, verifying archive format, extracting, and generating `plugin.json`.
    pub fn install_official_rapidocr_with_progress(
        &self,
        mut on_progress: impl FnMut(u64, Option<u64>, &'static str),
    ) -> Result<InstalledOcrPlugin, OcrError> {
        // Optional custom manifest override for self-hosted deployments.
        if let Ok(manifest_url) = std::env::var("CLIPBOARDX_RAPIDOCR_MANIFEST_URL")
            && !manifest_url.trim().is_empty()
        {
            return self.install_from_manifest_url(manifest_url.trim(), on_progress);
        }

        if !cfg!(target_os = "windows") {
            return Err(OcrError::PluginInvalid(
                "automatic RapidOCR-json download is currently provided for Windows; use local archive import on this OS".to_owned(),
            ));
        }

        let mut candidate_urls: Vec<String> = Vec::new();
        if let Ok(custom_url) = std::env::var("CLIPBOARDX_RAPIDOCR_URL")
            && !custom_url.trim().is_empty()
        {
            candidate_urls.push(custom_url.trim().to_owned());
        }
        for url in RAPIDOCR_MIRROR_URLS {
            candidate_urls.push((*url).to_owned());
        }

        fs::create_dir_all(&self.plugins_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("create plugins dir failed: {err}")))?;

        let tmp_archive = self
            .plugins_dir
            .join(format!(".rapidocr_download_{}.tmp", std::process::id()));

        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(300))
            .user_agent("ClipboardX-OCR-Installer/0.1")
            .build()
            .map_err(|err| OcrError::PluginInvalid(format!("build HTTP client failed: {err}")))?;

        let mut last_err = String::new();
        let mut downloaded_ok = false;

        for url in &candidate_urls {
            on_progress(0, None, "connecting");
            match stream_download_to_file(&client, url, &tmp_archive, &mut on_progress) {
                Ok(total) => {
                    on_progress(total, Some(total), "verifying");
                    if is_7z_archive(&tmp_archive)? || is_zip_archive(&tmp_archive)? {
                        downloaded_ok = true;
                        break;
                    }
                    let _ = fs::remove_file(&tmp_archive);
                    last_err = format!("unexpected non-archive response from {url}");
                }
                Err(err) => {
                    let _ = fs::remove_file(&tmp_archive);
                    last_err = err;
                }
            }
        }

        if !downloaded_ok {
            return Err(OcrError::PluginInvalid(format!(
                "RapidOCR download failed across all mirrors ({last_err}); you can also import a local .7z/.zip package below"
            )));
        }

        on_progress(1, Some(1), "extracting");
        let install_res = if is_7z_archive(&tmp_archive)? {
            self.extract_rapidocr_7z_archive(&tmp_archive)
        } else {
            self.extract_verified_zip(&tmp_archive)
        };
        let _ = fs::remove_file(&tmp_archive);
        install_res
    }

    fn install_from_manifest_url(
        &self,
        manifest_url: &str,
        mut on_progress: impl FnMut(u64, Option<u64>, &'static str),
    ) -> Result<InstalledOcrPlugin, OcrError> {
        on_progress(0, None, "connecting");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|err| OcrError::PluginInvalid(format!("build HTTP client failed: {err}")))?;

        let resp = client.get(manifest_url).send().map_err(|err| {
            OcrError::PluginInvalid(format!("rapidocr manifest request failed: {err}"))
        })?;
        if !resp.status().is_success() {
            return Err(OcrError::PluginInvalid(format!(
                "rapidocr manifest unavailable (HTTP {})",
                resp.status().as_u16()
            )));
        }

        let manifest_json: serde_json::Value = resp.json().map_err(|err| {
            OcrError::PluginInvalid(format!("invalid rapidocr manifest JSON: {err}"))
        })?;
        let target_key = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let asset = manifest_json
            .get("platforms")
            .and_then(|p| p.get(&target_key))
            .ok_or_else(|| {
                OcrError::PluginInvalid(format!("no rapidocr package for platform {target_key}"))
            })?;
        let url = asset
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OcrError::PluginInvalid("manifest missing asset url".to_owned()))?;
        let sha256 = asset
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OcrError::PluginInvalid("manifest missing asset sha256".to_owned()))?;

        self.install_from_url_with_progress(url, sha256, on_progress)
    }

    /// Extracts an upstream `RapidOCR-json` `.7z` package, locates `RapidOCR-json.exe`,
    /// writes a valid `plugin.json` descriptor, and installs it atomically into `<plugins_dir>`.
    fn extract_rapidocr_7z_archive(
        &self,
        archive_path: &Path,
    ) -> Result<InstalledOcrPlugin, OcrError> {
        fs::create_dir_all(&self.plugins_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("create plugins dir failed: {err}")))?;

        let staging_root = self
            .plugins_dir
            .join(format!(".staging_7z_{}", std::process::id()));
        if staging_root.exists() {
            let _ = fs::remove_dir_all(&staging_root);
        }
        fs::create_dir_all(&staging_root)
            .map_err(|err| OcrError::PluginInvalid(format!("create staging dir failed: {err}")))?;

        if let Err(err) = extract_7z_via_system_tool(archive_path, &staging_root) {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(err);
        }

        let Some(exe_path) = find_file_recursive(&staging_root, "RapidOCR-json.exe", 3) else {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(OcrError::PluginInvalid(
                "extracted archive does not contain RapidOCR-json.exe".to_owned(),
            ));
        };
        let Some(package_root) = exe_path.parent().map(Path::to_path_buf) else {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(OcrError::PluginInvalid(
                "failed to resolve RapidOCR-json directory".to_owned(),
            ));
        };

        let manifest = OcrPluginManifest {
            id: RAPIDOCR_PLUGIN_ID.to_owned(),
            name: "RapidOCR ONNX".to_owned(),
            version: RAPIDOCR_RELEASE_VERSION.to_owned(),
            description: "PP-OCRv4 offline recognition via RapidOCR-json".to_owned(),
            executable: "RapidOCR-json.exe".to_owned(),
            args: vec!["--ensureAscii=1".to_owned()],
            languages: vec!["zh".to_owned(), "en".to_owned()],
            protocol: OcrPluginProtocol::RapidOcrJson,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|err| OcrError::PluginInvalid(format!("serialize plugin.json: {err}")))?;
        fs::write(package_root.join("plugin.json"), manifest_bytes)
            .map_err(|err| OcrError::PluginInvalid(format!("write plugin.json: {err}")))?;

        let target_dir = self.plugins_dir.join(RAPIDOCR_PLUGIN_ID);
        if target_dir.exists() {
            fs::remove_dir_all(&target_dir).map_err(|err| {
                OcrError::PluginInvalid(format!("replace existing rapidocr plugin: {err}"))
            })?;
        }
        fs::rename(&package_root, &target_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("finalize rapidocr install: {err}")))?;
        let _ = fs::remove_dir_all(&staging_root);

        Ok(InstalledOcrPlugin {
            manifest,
            install_dir: target_dir,
        })
    }

    /// Removes an installed plugin directory cleanly.
    pub fn uninstall(&self, plugin_id: &str) -> Result<(), OcrError> {
        validate_plugin_id(plugin_id)?;
        let target = self.plugins_dir.join(plugin_id);
        if target.exists() {
            fs::remove_dir_all(&target)
                .map_err(|err| OcrError::PluginInvalid(format!("remove plugin failed: {err}")))?;
        }
        Ok(())
    }

    fn extract_verified_zip(&self, zip_path: &Path) -> Result<InstalledOcrPlugin, OcrError> {
        fs::create_dir_all(&self.plugins_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("create plugins dir failed: {err}")))?;

        let file = File::open(zip_path)
            .map_err(|err| OcrError::PluginInvalid(format!("open plugin zip failed: {err}")))?;
        let mut zip = ZipArchive::new(file)
            .map_err(|err| OcrError::PluginInvalid(format!("invalid plugin zip: {err}")))?;

        let manifest: OcrPluginManifest = {
            let mut mf = zip.by_name("plugin.json").map_err(|_| {
                OcrError::PluginInvalid("plugin package missing root plugin.json".to_owned())
            })?;
            let mut raw = Vec::new();
            mf.read_to_end(&mut raw).map_err(|err| {
                OcrError::PluginInvalid(format!("read plugin.json failed: {err}"))
            })?;
            serde_json::from_slice(&raw)
                .map_err(|err| OcrError::PluginInvalid(format!("invalid plugin.json: {err}")))?
        };

        validate_plugin_id(&manifest.id)?;

        let target_dir = self.plugins_dir.join(&manifest.id);
        let staging_dir = self.plugins_dir.join(format!(".staging_{}", manifest.id));
        if staging_dir.exists() {
            let _ = fs::remove_dir_all(&staging_dir);
        }
        fs::create_dir_all(&staging_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("create staging dir failed: {err}")))?;

        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|err| OcrError::PluginInvalid(format!("read zip entry failed: {err}")))?;
            let Some(rel_path) = entry.enclosed_name() else {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(OcrError::PluginInvalid(
                    "unsafe path traversal in plugin archive".to_owned(),
                ));
            };
            let dest = staging_dir.join(&rel_path);
            if entry.is_dir() {
                fs::create_dir_all(&dest)
                    .map_err(|err| OcrError::PluginInvalid(format!("mkdir failed: {err}")))?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|err| OcrError::PluginInvalid(format!("mkdir failed: {err}")))?;
                }
                let mut out = File::create(&dest)
                    .map_err(|err| OcrError::PluginInvalid(format!("create file failed: {err}")))?;
                io::copy(&mut entry, &mut out).map_err(|err| {
                    OcrError::PluginInvalid(format!("extract file failed: {err}"))
                })?;
            }
        }

        if target_dir.exists() {
            fs::remove_dir_all(&target_dir).map_err(|err| {
                OcrError::PluginInvalid(format!("replace existing plugin: {err}"))
            })?;
        }
        fs::rename(&staging_dir, &target_dir)
            .map_err(|err| OcrError::PluginInvalid(format!("finalize plugin install: {err}")))?;

        Ok(InstalledOcrPlugin {
            manifest,
            install_dir: target_dir,
        })
    }
}

fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<(), OcrError> {
    let mut file = File::open(path)
        .map_err(|err| OcrError::PluginInvalid(format!("open file for sha256 failed: {err}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|err| {
            OcrError::PluginInvalid(format!("read file for sha256 failed: {err}"))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_hex.trim()) {
        return Err(OcrError::PluginInvalid(format!(
            "SHA-256 mismatch: expected {}, got {actual}",
            expected_hex.trim()
        )));
    }
    Ok(())
}

fn validate_plugin_id(id: &str) -> Result<(), OcrError> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !valid {
        return Err(OcrError::PluginInvalid(format!("invalid plugin id: {id}")));
    }
    Ok(())
}

fn stream_download_to_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    on_progress: &mut impl FnMut(u64, Option<u64>, &'static str),
) -> Result<u64, String> {
    use std::io::Write;
    let mut resp = client
        .get(url)
        .send()
        .map_err(|err| format!("request {url} failed: {err}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} from {url}", resp.status().as_u16()));
    }
    let total_bytes = resp.content_length();
    let mut out = File::create(dest).map_err(|err| format!("create temp archive: {err}"))?;
    let mut buf = [0u8; 32 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|err| format!("read stream from {url}: {err}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|err| format!("write temp archive: {err}"))?;
        downloaded = downloaded.saturating_add(n as u64);
        on_progress(downloaded, total_bytes, "downloading");
    }
    Ok(downloaded)
}

fn read_magic_header(path: &Path) -> Result<[u8; 6], OcrError> {
    let mut f = File::open(path)
        .map_err(|err| OcrError::PluginInvalid(format!("open archive header failed: {err}")))?;
    let mut header = [0u8; 6];
    let n = f
        .read(&mut header)
        .map_err(|err| OcrError::PluginInvalid(format!("read archive header failed: {err}")))?;
    if n < 4 {
        return Ok([0u8; 6]);
    }
    Ok(header)
}

fn is_7z_archive(path: &Path) -> Result<bool, OcrError> {
    Ok(read_magic_header(path)? == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C])
}

fn is_zip_archive(path: &Path) -> Result<bool, OcrError> {
    let h = read_magic_header(path)?;
    Ok(h[0] == b'P' && h[1] == b'K')
}

/// Extracts a `.7z` archive using Windows built-in `tar` (libarchive) or `7z` if available.
fn extract_7z_via_system_tool(archive_path: &Path, dest_dir: &Path) -> Result<(), OcrError> {
    let mut tar_cmd = Command::new("tar");
    tar_cmd
        .arg("-xf")
        .arg(archive_path)
        .arg("-C")
        .arg(dest_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        tar_cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Ok(output) = tar_cmd.output()
        && output.status.success()
    {
        return Ok(());
    }

    let candidates: &[&str] = &[
        "7z",
        "7za",
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe",
    ];
    for bin in candidates {
        let mut cmd = Command::new(bin);
        cmd.arg("x")
            .arg("-y")
            .arg(format!("-o{}", dest_dir.display()))
            .arg(archive_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(status) = cmd.status()
            && status.success()
        {
            return Ok(());
        }
    }

    Err(OcrError::PluginInvalid(
        "failed to extract .7z archive: ensure Windows tar or 7-Zip is available".to_owned(),
    ))
}

fn find_file_recursive(dir: &Path, file_name: &str, max_depth: usize) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(file_name))
        {
            return Some(path);
        } else if path.is_dir() && max_depth > 0 {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_file_recursive(&sub, file_name, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    fn build_test_plugin_zip(
        path: &Path,
        id: &str,
        traversal_entry: bool,
    ) -> Result<String, String> {
        let file = File::create(path).map_err(|e| e.to_string())?;
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default();

        let manifest = serde_json::json!({
            "id": id,
            "name": "Test RapidOCR",
            "version": "1.0.0",
            "description": "Local OCR test plugin",
            "executable": "runner.bin",
            "args": [],
            "languages": ["zh", "en"]
        });
        zip.start_file("plugin.json", opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(manifest.to_string().as_bytes())
            .map_err(|e| e.to_string())?;

        zip.start_file("runner.bin", opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(b"binary-content")
            .map_err(|e| e.to_string())?;

        if traversal_entry {
            zip.start_file("../escape.txt", opts)
                .map_err(|e| e.to_string())?;
            zip.write_all(b"bad").map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;

        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    #[test]
    fn plugin_installer_verifies_sha256_prevents_traversal_and_uninstalls() {
        let dir =
            std::env::temp_dir().join(format!("clipboardx_plugin_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(fs::create_dir_all(&dir).is_ok());

        let mgr = OcrPluginManager::new(dir.join("plugins"));
        let good_zip = dir.join("good.zip");
        let sha = match build_test_plugin_zip(&good_zip, "rapidocr.onnx", false) {
            Ok(s) => s,
            Err(err) => panic!("build test zip failed: {err}"),
        };

        // Wrong checksum must be rejected.
        assert!(
            mgr.install_from_local_zip(&good_zip, Some(&"0".repeat(64)))
                .is_err()
        );

        // Matching checksum installs cleanly and lists plugin.
        let installed = mgr.install_from_local_zip(&good_zip, Some(&sha));
        assert!(installed.is_ok());
        assert_eq!(mgr.list_installed().map(|v| v.len()).unwrap_or(0), 1);

        // Traversal entry inside zip must be blocked.
        let evil_zip = dir.join("evil.zip");
        assert!(build_test_plugin_zip(&evil_zip, "evil.ocr", true).is_ok());
        assert!(mgr.install_from_local_zip(&evil_zip, None).is_err());

        assert!(mgr.uninstall("rapidocr.onnx").is_ok());
        assert_eq!(mgr.list_installed().map(|v| v.len()).unwrap_or(99), 0);

        let _ = fs::remove_dir_all(&dir);
    }
}
