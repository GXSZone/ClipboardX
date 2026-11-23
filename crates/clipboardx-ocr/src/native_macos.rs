use std::fs;
use std::process::Command;

use clipboardx_model::{OcrError, OcrOutput, OcrProvider};

/// Native macOS OCR provider invoking the system Vision framework via `/usr/bin/swift`
/// without requiring Objective-C FFI bindings.
#[derive(Debug, Default)]
pub struct MacosNativeOcrProvider;

impl MacosNativeOcrProvider {
    pub const fn new() -> Self {
        Self
    }
}

const VISION_SWIFT_SCRIPT: &str = r#"
import Foundation
import Vision
import AppKit

guard CommandLine.arguments.count > 1 else { exit(2) }
let path = CommandLine.arguments[1]
guard let img = NSImage(contentsOfFile: path),
      let tiff = img.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff),
      let cgImg = rep.cgImage else { exit(3) }

let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]

let handler = VNImageRequestHandler(cgImage: cgImg, options: [:])
try handler.perform([request])
let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
print(lines.joined(separator: "\n"))
"#;

impl OcrProvider for MacosNativeOcrProvider {
    fn id(&self) -> &str {
        "native.macos.vision"
    }

    fn display_name(&self) -> &str {
        "macOS Vision OCR"
    }

    fn is_available(&self) -> bool {
        std::path::Path::new("/usr/bin/swift").exists()
    }

    fn recognize_png(
        &self,
        png_bytes: &[u8],
        _lang_hint: Option<&str>,
    ) -> Result<OcrOutput, OcrError> {
        if png_bytes.is_empty() {
            return Err(OcrError::Inference("empty image buffer".to_owned()));
        }
        let temp_path = std::env::temp_dir().join(format!(
            "clipboardx_vision_{}_{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::write(&temp_path, png_bytes)
            .map_err(|err| OcrError::Inference(format!("write temp PNG failed: {err}")))?;

        let output = Command::new("/usr/bin/swift")
            .args(["-e", VISION_SWIFT_SCRIPT])
            .arg(&temp_path)
            .output();
        let _ = fs::remove_file(&temp_path);

        let output = output.map_err(|err| {
            OcrError::Unavailable(format!("spawn swift Vision OCR failed: {err}"))
        })?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(OcrError::Inference(format!(
                "Vision OCR exited with error: {}",
                err_msg.trim()
            )));
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok(OcrOutput {
            full_text: text,
            provider_id: self.id().to_owned(),
        })
    }
}
