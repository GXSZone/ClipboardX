use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clipboardx_model::{OcrError, OcrOutput, OcrProvider};
use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_IDLE_RECLAIM: Duration = Duration::from_secs(60);
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Wire protocol used to communicate with an external OCR child process over stdio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrPluginProtocol {
    #[default]
    JsonRpc,
    RapidOcrJson,
}

/// Metadata descriptor (`plugin.json`) placed at the root of each installed OCR plugin folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrPluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub protocol: OcrPluginProtocol,
}

struct WarmWorker {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    last_used: Instant,
    next_id: u64,
}

impl Drop for WarmWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct TempPngGuard {
    path: PathBuf,
}

impl TempPngGuard {
    fn write_bytes(bytes: &[u8]) -> Result<Self, OcrError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("clipboardx_ocr_{}_{nonce}.png", std::process::id()));
        fs::write(&path, bytes)
            .map_err(|err| OcrError::Inference(format!("write temp OCR image failed: {err}")))?;
        Ok(Self { path })
    }
}

impl Drop for TempPngGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// External OCR engine communicating over newline-delimited JSON-RPC via stdio
/// with a warm worker pool and automatic 60-second idle reclamation.
pub struct JsonRpcOcrProvider {
    manifest: OcrPluginManifest,
    executable_path: PathBuf,
    working_dir: PathBuf,
    worker: Mutex<Option<WarmWorker>>,
}

impl JsonRpcOcrProvider {
    pub fn new(manifest: OcrPluginManifest, install_dir: PathBuf) -> Self {
        let executable_path = install_dir.join(&manifest.executable);
        Self {
            manifest,
            executable_path,
            working_dir: install_dir,
            worker: Mutex::new(None),
        }
    }

    /// Reaps the warm worker subprocess if it has been idle longer than `max_idle`.
    pub fn reclaim_if_idle(&self, max_idle: Duration) {
        if let Ok(mut guard) = self.worker.lock()
            && let Some(w) = guard.as_ref()
            && w.last_used.elapsed() >= max_idle
        {
            *guard = None;
        }
    }

    fn spawn_worker(&self) -> Result<WarmWorker, OcrError> {
        if !self.executable_path.exists() {
            return Err(OcrError::Unavailable(format!(
                "OCR plugin binary not found: {}",
                self.executable_path.display()
            )));
        }

        let mut cmd = Command::new(&self.executable_path);
        cmd.args(&self.manifest.args)
            .current_dir(&self.working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|err| OcrError::Unavailable(format!("spawn OCR plugin failed: {err}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| OcrError::Unavailable("plugin stdin pipe missing".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::Unavailable("plugin stdout pipe missing".to_owned()))?;

        let mut reader = BufReader::new(stdout);
        if self.manifest.protocol == OcrPluginProtocol::RapidOcrJson {
            wait_for_rapidocr_ready(&mut reader)?;
        }

        Ok(WarmWorker {
            child,
            stdin: BufWriter::new(stdin),
            stdout: reader,
            last_used: Instant::now(),
            next_id: 1,
        })
    }
}

impl OcrProvider for JsonRpcOcrProvider {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn display_name(&self) -> &str {
        &self.manifest.name
    }

    fn is_available(&self) -> bool {
        self.executable_path.exists()
    }

    fn recognize_png(
        &self,
        png_bytes: &[u8],
        lang_hint: Option<&str>,
    ) -> Result<OcrOutput, OcrError> {
        if png_bytes.is_empty() {
            return Err(OcrError::Inference("empty image buffer".to_owned()));
        }

        let temp_png = TempPngGuard::write_bytes(png_bytes)?;
        let langs: Vec<&str> = match lang_hint {
            Some(hint) if !hint.trim().is_empty() => vec![hint.trim()],
            _ if !self.manifest.languages.is_empty() => {
                self.manifest.languages.iter().map(String::as_str).collect()
            }
            _ => vec!["zh", "en"],
        };

        let mut guard = self
            .worker
            .lock()
            .map_err(|_| OcrError::Inference("OCR worker lock poisoned".to_owned()))?;

        // Reclaim stale or exited workers before issuing the next request.
        if let Some(existing) = guard.as_mut() {
            let exited = existing.child.try_wait().ok().flatten().is_some();
            if exited || existing.last_used.elapsed() >= DEFAULT_IDLE_RECLAIM {
                *guard = None;
            }
        }

        if guard.is_none() {
            *guard = Some(self.spawn_worker()?);
        }

        let rpc_res = {
            let Some(worker) = guard.as_mut() else {
                return Err(OcrError::Unavailable(
                    "failed to allocate OCR worker".to_owned(),
                ));
            };
            match self.manifest.protocol {
                OcrPluginProtocol::JsonRpc => {
                    send_rpc_request(worker, &self.manifest.id, &temp_png.path, &langs)
                }
                OcrPluginProtocol::RapidOcrJson => {
                    send_rapidocr_json_request(worker, &self.manifest.id, &temp_png.path)
                }
            }
        };

        if rpc_res.is_err() {
            // Drop potentially wedged worker so the next request spawns a fresh subprocess.
            *guard = None;
        }
        rpc_res
    }
}

fn send_rpc_request(
    worker: &mut WarmWorker,
    plugin_id: &str,
    image_path: &std::path::Path,
    langs: &[&str],
) -> Result<OcrOutput, OcrError> {
    let req_id = worker.next_id;
    worker.next_id = req_id.saturating_add(1);

    let payload = json!({
        "id": req_id,
        "method": "ocr.recognize",
        "params": {
            "image_path": image_path.to_string_lossy(),
            "languages": langs,
        }
    });

    let mut line = serde_json::to_string(&payload)
        .map_err(|err| OcrError::Inference(format!("serialize RPC request: {err}")))?;
    line.push('\n');

    worker
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| worker.stdin.flush())
        .map_err(|err| OcrError::Inference(format!("write RPC request failed: {err}")))?;

    let mut resp_line = String::new();
    worker
        .stdout
        .read_line(&mut resp_line)
        .map_err(|err| OcrError::Inference(format!("read RPC response failed: {err}")))?;
    if resp_line.trim().is_empty() {
        return Err(OcrError::Inference("OCR worker returned EOF".to_owned()));
    }

    let resp: serde_json::Value = serde_json::from_str(resp_line.trim())
        .map_err(|err| OcrError::Inference(format!("invalid RPC JSON response: {err}")))?;

    if resp["id"].as_u64() != Some(req_id) {
        return Err(OcrError::Inference("RPC response id mismatch".to_owned()));
    }

    if let Some(err_obj) = resp.get("error").filter(|v| !v.is_null()) {
        let msg = err_obj["message"]
            .as_str()
            .unwrap_or("unknown plugin error")
            .to_owned();
        return Err(OcrError::Inference(msg));
    }

    worker.last_used = Instant::now();
    let text = resp["result"]["text"]
        .as_str()
        .ok_or_else(|| OcrError::Inference("missing result.text field in OCR response".to_owned()))?
        .trim()
        .to_owned();

    Ok(OcrOutput {
        full_text: text,
        provider_id: plugin_id.to_owned(),
    })
}

/// Drains startup banner lines from `RapidOCR-json` until the engine signals readiness.
fn wait_for_rapidocr_ready(stdout: &mut BufReader<ChildStdout>) -> Result<(), OcrError> {
    let mut banner_lines = Vec::new();
    for _ in 0..64 {
        let mut line = String::new();
        let n = stdout
            .read_line(&mut line)
            .map_err(|err| OcrError::Unavailable(format!("read RapidOCR init failed: {err}")))?;
        if n == 0 {
            let detail = banner_lines.join(" | ");
            return Err(OcrError::Unavailable(format!(
                "RapidOCR exited before ready: {detail}"
            )));
        }
        let trimmed = line.trim();
        if trimmed.contains("OCR init completed") {
            return Ok(());
        }
        if !trimmed.is_empty() && banner_lines.len() < 4 {
            banner_lines.push(trimmed.to_owned());
        }
    }
    Err(OcrError::Unavailable(
        "RapidOCR did not emit ready banner within 64 lines".to_owned(),
    ))
}

/// Sends a single newline-delimited `{"image_path": "..."}` request to `RapidOCR-json`.
fn send_rapidocr_json_request(
    worker: &mut WarmWorker,
    plugin_id: &str,
    image_path: &std::path::Path,
) -> Result<OcrOutput, OcrError> {
    let normalized = image_path.to_string_lossy().replace('\\', "/");
    let payload = json!({ "image_path": normalized });
    let mut line = serde_json::to_string(&payload)
        .map_err(|err| OcrError::Inference(format!("serialize RapidOCR request: {err}")))?;
    line.push('\n');

    worker
        .stdin
        .write_all(line.as_bytes())
        .and_then(|()| worker.stdin.flush())
        .map_err(|err| OcrError::Inference(format!("write RapidOCR request failed: {err}")))?;

    let mut resp_line = String::new();
    for _ in 0..16 {
        resp_line.clear();
        let n = worker
            .stdout
            .read_line(&mut resp_line)
            .map_err(|err| OcrError::Inference(format!("read RapidOCR response failed: {err}")))?;
        if n == 0 {
            return Err(OcrError::Inference(
                "RapidOCR worker returned EOF".to_owned(),
            ));
        }
        if resp_line.trim_start().starts_with('{') {
            break;
        }
    }

    let trimmed = resp_line.trim();
    if trimmed.is_empty() {
        return Err(OcrError::Inference(
            "RapidOCR worker returned empty output".to_owned(),
        ));
    }

    let resp: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|err| OcrError::Inference(format!("invalid RapidOCR JSON response: {err}")))?;

    worker.last_used = Instant::now();
    let code = resp["code"].as_i64().unwrap_or(-1);
    match code {
        100 => {
            let mut lines = Vec::new();
            if let Some(items) = resp["data"].as_array() {
                for entry in items {
                    if let Some(t) = entry["text"].as_str() {
                        let clean = t.trim();
                        if !clean.is_empty() {
                            lines.push(clean);
                        }
                    }
                }
            }
            Ok(OcrOutput {
                full_text: lines.join("\n"),
                provider_id: plugin_id.to_owned(),
            })
        }
        101 => Ok(OcrOutput {
            full_text: String::new(),
            provider_id: plugin_id.to_owned(),
        }),
        other => {
            let msg = resp["data"]
                .as_str()
                .unwrap_or("RapidOCR recognition error")
                .to_owned();
            Err(OcrError::Inference(format!("RapidOCR code {other}: {msg}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_provider_reuses_warm_worker_and_reclaims_idle() {
        let dir = std::env::temp_dir().join(format!("clipboardx_rpc_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(fs::create_dir_all(&dir).is_ok());

        #[cfg(target_os = "windows")]
        let (exe_name, args) = {
            let script = dir.join("mock_ocr.cmd");
            let code = "@echo off\r\npowershell -NoProfile -Command \"while ($line = [Console]::In.ReadLine()) { $req = $line | ConvertFrom-Json; $id = $req.id; Write-Output ('{\\\"id\\\":' + $id + ',\\\"result\\\":{\\\"text\\\":\\\"hello-ocr-' + $id + '\\\"}}') }\"\r\n";
            assert!(fs::write(&script, code).is_ok());
            ("mock_ocr.cmd".to_owned(), Vec::new())
        };

        #[cfg(not(target_os = "windows"))]
        let (exe_name, args) = {
            let script = dir.join("mock_ocr.sh");
            let code = "#!/bin/sh\nwhile IFS= read -r line; do\n  id=$(echo \"$line\" | sed -n 's/.*\"id\":\\([0-9]*\\).*/\\1/p')\n  echo \"{\\\"id\\\":$id,\\\"result\\\":{\\\"text\\\":\\\"hello-ocr-$id\\\"}}\"\ndone\n";
            assert!(fs::write(&script, code).is_ok());
            ("/bin/sh".to_owned(), vec!["mock_ocr.sh".to_owned()])
        };

        let manifest = OcrPluginManifest {
            id: "mock.rpc.ocr".to_owned(),
            name: "Mock RPC OCR".to_owned(),
            version: "0.1.0".to_owned(),
            description: "Test worker".to_owned(),
            executable: exe_name,
            args,
            languages: vec!["en".to_owned()],
            protocol: OcrPluginProtocol::JsonRpc,
        };

        let provider = JsonRpcOcrProvider::new(manifest, dir.clone());
        #[cfg(target_os = "windows")]
        assert!(provider.is_available());

        let first = provider.recognize_png(b"fake-png", Some("en"));
        assert_eq!(
            first.as_ref().ok().map(|o| o.full_text.as_str()),
            Some("hello-ocr-1")
        );

        let second = provider.recognize_png(b"fake-png", None);
        assert_eq!(
            second.as_ref().ok().map(|o| o.full_text.as_str()),
            Some("hello-ocr-2")
        );

        provider.reclaim_if_idle(Duration::ZERO);
        let third = provider.recognize_png(b"fake-png", None);
        // After reclaiming idle worker, sequence restarts cleanly at id 1.
        assert_eq!(
            third.as_ref().ok().map(|o| o.full_text.as_str()),
            Some("hello-ocr-1")
        );

        drop(provider);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rapidocr_json_provider_parses_init_banner_and_codes() {
        let dir =
            std::env::temp_dir().join(format!("clipboardx_rapid_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(fs::create_dir_all(&dir).is_ok());

        #[cfg(target_os = "windows")]
        let (exe_name, args) = {
            let script = dir.join("mock_rapid.cmd");
            let code = "@echo off\r\npowershell -NoProfile -Command \"Write-Output 'RapidOCR-json v0.2.0'; Write-Output 'OCR init completed.'; $n = 0; while ($line = [Console]::In.ReadLine()) { $n++; if ($n -eq 1) { Write-Output '{\\\"code\\\":100,\\\"data\\\":[{\\\"text\\\":\\\"line-one\\\"},{\\\"text\\\":\\\"line-two\\\"}]}' } else { Write-Output '{\\\"code\\\":101,\\\"data\\\":\\\"No text found\\\"}' } }\"\r\n";
            assert!(fs::write(&script, code).is_ok());
            ("mock_rapid.cmd".to_owned(), Vec::new())
        };

        #[cfg(not(target_os = "windows"))]
        let (exe_name, args) = {
            let script = dir.join("mock_rapid.sh");
            let code = "#!/bin/sh\necho 'OCR init completed.'\nn=0\nwhile IFS= read -r line; do\n  n=$((n+1))\n  if [ \"$n\" -eq 1 ]; then\n    echo '{\"code\":100,\"data\":[{\"text\":\"line-one\"},{\"text\":\"line-two\"}]}'\n  else\n    echo '{\"code\":101,\"data\":\"No text found\"}'\n  fi\ndone\n";
            assert!(fs::write(&script, code).is_ok());
            ("/bin/sh".to_owned(), vec!["mock_rapid.sh".to_owned()])
        };

        let manifest = OcrPluginManifest {
            id: "rapidocr.onnx".to_owned(),
            name: "RapidOCR ONNX".to_owned(),
            version: "0.2.0".to_owned(),
            description: "RapidOCR-json test".to_owned(),
            executable: exe_name,
            args,
            languages: vec!["zh".to_owned(), "en".to_owned()],
            protocol: OcrPluginProtocol::RapidOcrJson,
        };

        let provider = JsonRpcOcrProvider::new(manifest, dir.clone());
        let hit = provider.recognize_png(b"fake-png", None);
        assert_eq!(
            hit.as_ref().ok().map(|o| o.full_text.as_str()),
            Some("line-one\nline-two")
        );

        let empty = provider.recognize_png(b"fake-png", None);
        assert_eq!(empty.as_ref().ok().map(|o| o.full_text.as_str()), Some(""));

        drop(provider);
        let _ = fs::remove_dir_all(&dir);
    }
}
