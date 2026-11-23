use clipboardx_model::{OcrError, OcrOutput, OcrProvider};
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

/// Native Windows 10/11 OCR engine backed by safe WinRT `Windows.Media.Ocr`.
#[derive(Debug, Default)]
pub struct WindowsNativeOcrProvider;

impl WindowsNativeOcrProvider {
    pub const fn new() -> Self {
        Self
    }
}

impl OcrProvider for WindowsNativeOcrProvider {
    fn id(&self) -> &str {
        "native.windows.winrt"
    }

    fn display_name(&self) -> &str {
        "Windows WinRT OCR"
    }

    fn is_available(&self) -> bool {
        OcrEngine::TryCreateFromUserProfileLanguages().is_ok()
    }

    fn recognize_png(
        &self,
        png_bytes: &[u8],
        _lang_hint: Option<&str>,
    ) -> Result<OcrOutput, OcrError> {
        if png_bytes.is_empty() {
            return Err(OcrError::Inference("empty image buffer".to_owned()));
        }

        let stream = InMemoryRandomAccessStream::new().map_err(|err| {
            OcrError::Inference(format!("create WinRT memory stream failed: {err}"))
        })?;
        let writer = DataWriter::CreateDataWriter(&stream)
            .map_err(|err| OcrError::Inference(format!("create DataWriter failed: {err}")))?;
        writer
            .WriteBytes(png_bytes)
            .map_err(|err| OcrError::Inference(format!("write image bytes failed: {err}")))?;
        writer
            .StoreAsync()
            .map_err(|err| OcrError::Inference(format!("store WinRT buffer failed: {err}")))?
            .join()
            .map_err(|err| OcrError::Inference(format!("await store buffer failed: {err}")))?;
        let _ = writer.DetachStream();
        stream
            .Seek(0)
            .map_err(|err| OcrError::Inference(format!("seek stream failed: {err}")))?;

        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|err| OcrError::Inference(format!("create BitmapDecoder failed: {err}")))?
            .join()
            .map_err(|err| OcrError::Inference(format!("await BitmapDecoder failed: {err}")))?;

        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|err| OcrError::Inference(format!("decode SoftwareBitmap failed: {err}")))?
            .join()
            .map_err(|err| OcrError::Inference(format!("await SoftwareBitmap failed: {err}")))?;

        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|err| OcrError::Unavailable(format!("init WinRT OcrEngine failed: {err}")))?;

        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|err| OcrError::Inference(format!("start RecognizeAsync failed: {err}")))?
            .join()
            .map_err(|err| OcrError::Inference(format!("await RecognizeAsync failed: {err}")))?;

        let text = result
            .Text()
            .map_err(|err| OcrError::Inference(format!("extract OCR text failed: {err}")))?
            .to_string_lossy();

        Ok(OcrOutput {
            full_text: text.trim().to_owned(),
            provider_id: self.id().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winrt_ocr_provider_initializes_without_panic_on_windows() {
        let provider = WindowsNativeOcrProvider::new();
        assert_eq!(provider.id(), "native.windows.winrt");
        // Engine availability depends on installed Windows language packs; either true/false is valid.
        let _available = provider.is_available();
    }
}
