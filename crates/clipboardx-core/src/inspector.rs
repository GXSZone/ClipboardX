use clipboardx_model::SmartSubtype;

const MAX_INSPECT_BYTES: usize = 512 * 1024;

/// Output of lightweight deterministic content inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionResult {
    pub subtype: SmartSubtype,
    pub detected_lang: Option<String>,
    pub color_hex: Option<String>,
}

/// Inspects plain text offline without heavy parser dependencies.
pub fn inspect_text(raw: &str) -> InspectionResult {
    if raw.len() > MAX_INSPECT_BYTES {
        return InspectionResult {
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            color_hex: None,
        };
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return InspectionResult {
            subtype: SmartSubtype::Plain,
            detected_lang: None,
            color_hex: None,
        };
    }

    if let Some(hex) = parse_hex_color_normalized(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Color,
            detected_lang: None,
            color_hex: Some(hex),
        };
    }

    if is_jwt(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Jwt,
            detected_lang: Some("json".to_owned()),
            color_hex: None,
        };
    }

    if is_unix_timestamp(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Timestamp,
            detected_lang: None,
            color_hex: None,
        };
    }

    if is_url(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Url,
            detected_lang: None,
            color_hex: None,
        };
    }

    if is_json(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Json,
            detected_lang: Some("json".to_owned()),
            color_hex: None,
        };
    }

    if let Some(lang) = detect_code_language(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Code,
            detected_lang: Some(lang.to_owned()),
            color_hex: None,
        };
    }

    if is_base64_candidate(trimmed) {
        return InspectionResult {
            subtype: SmartSubtype::Base64,
            detected_lang: None,
            color_hex: None,
        };
    }

    InspectionResult {
        subtype: SmartSubtype::Plain,
        detected_lang: None,
        color_hex: None,
    }
}

/// Parses `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `rgb(...)`, or `oklch(...)` header prefixes.
/// Returns normalized `#RRGGBB` hex for direct swatch rendering when hex/rgb is recognized.
pub fn parse_hex_color_normalized(s: &str) -> Option<String> {
    if let Some(hex) = s.strip_prefix('#')
        && matches!(hex.len(), 3 | 6 | 8)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        if hex.len() == 3 {
            let mut chars = hex.chars();
            let r = chars.next()?;
            let g = chars.next()?;
            let b = chars.next()?;
            return Some(format!("#{r}{r}{g}{g}{b}{b}").to_uppercase());
        }
        return Some(format!("#{}", &hex[..6]).to_uppercase());
    }

    let lower = s.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
        && let Some(inner) = inner.strip_suffix(')')
    {
        let parts: Vec<&str> = inner
            .split([',', ' ', '/'])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 3 {
            let r: u8 = parts[0].parse().ok()?;
            let g: u8 = parts[1].parse().ok()?;
            let b: u8 = parts[2].parse().ok()?;
            return Some(format!("#{r:02X}{g:02X}{b:02X}"));
        }
    }

    None
}

fn is_jwt(s: &str) -> bool {
    if s.contains(char::is_whitespace) || s.len() < 24 {
        return false;
    }
    let mut parts = s.split('.');
    let Some(h) = parts.next() else {
        return false;
    };
    let Some(p) = parts.next() else {
        return false;
    };
    let Some(sig) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    h.starts_with("eyJ")
        && p.starts_with("eyJ")
        && !sig.is_empty()
        && [h, p, sig].into_iter().all(|seg| {
            seg.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn is_unix_timestamp(s: &str) -> bool {
    if !(s.len() == 10 || s.len() == 13) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Ok(val) = s.parse::<i64>() else {
        return false;
    };
    if s.len() == 10 {
        (1_000_000_000..=4_102_444_800).contains(&val)
    } else {
        (1_000_000_000_000..=4_102_444_800_000).contains(&val)
    }
}

fn is_url(s: &str) -> bool {
    if s.contains(char::is_whitespace) || s.len() > 2048 {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && s.len() > 10
}

fn is_json(s: &str) -> bool {
    ((s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')))
        && serde_json::from_str::<serde_json::Value>(s).is_ok()
}

fn detect_code_language(s: &str) -> Option<&'static str> {
    let lines = s.lines().count();
    if lines < 2
        && !s.contains("fn ")
        && !s.contains("SELECT ")
        && !s.contains("const ")
        && !s.contains("import ")
    {
        return None;
    }
    if s.contains("fn ") && (s.contains("->") || s.contains("let ")) {
        return Some("rust");
    }
    if s.contains("SELECT ") && s.contains(" FROM ") {
        return Some("sql");
    }
    if s.contains("function ") || s.contains("const ") || s.contains("=>") {
        return Some("typescript");
    }
    if s.contains("def ") && s.contains(':') {
        return Some("python");
    }
    if s.contains("#!/bin/") || s.starts_with("cargo ") || s.starts_with("git ") {
        return Some("shell");
    }
    None
}

fn is_base64_candidate(s: &str) -> bool {
    if let Some((_, payload)) = parse_data_image_uri(s) {
        return !payload.is_empty();
    }
    if s.len() < 16 || !s.len().is_multiple_of(4) || s.contains(char::is_whitespace) {
        return false;
    }
    let has_padding = s.ends_with('=');
    let valid_chars = s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=');
    valid_chars && (has_padding || s.len() >= 24)
}

/// Extracts `(mime_subtype, base64_body)` from a `data:image/...;base64,...` URI if valid.
pub fn parse_data_image_uri(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("data:image/")?;
    let (mime_sub, payload) = rest.split_once(";base64,")?;
    if matches!(mime_sub, "png" | "jpeg" | "jpg" | "webp" | "gif") && !payload.is_empty() {
        Some((mime_sub, payload))
    } else {
        None
    }
}

/// Strips common tracking query parameters (`utm_*`, `fbclid`, `gclid`, `spm`) from a URL.
pub fn strip_tracking_params(raw_url: &str) -> String {
    let Some((base, query)) = raw_url.split_once('?') else {
        return raw_url.to_owned();
    };
    let (query_part, fragment) = match query.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (query, None),
    };
    let kept: Vec<&str> = query_part
        .split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or("").to_ascii_lowercase();
            !key.is_empty()
                && !key.starts_with("utm_")
                && !matches!(key.as_str(), "fbclid" | "gclid" | "spm" | "igshid" | "ref")
        })
        .collect();

    let mut out = if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    if let Some(frag) = fragment {
        out.push('#');
        out.push_str(frag);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_all_smart_subtypes() {
        assert_eq!(inspect_text("#3b82f6").subtype, SmartSubtype::Color);
        assert_eq!(
            inspect_text("#3b82f6").color_hex.as_deref(),
            Some("#3B82F6")
        );
        assert_eq!(
            inspect_text("rgb(59, 130, 246)").color_hex.as_deref(),
            Some("#3B82F6")
        );
        assert_eq!(
            inspect_text("https://clipboardx.dev/docs?a=1").subtype,
            SmartSubtype::Url
        );
        assert_eq!(inspect_text("{\"ok\":true}").subtype, SmartSubtype::Json);
        assert_eq!(
            inspect_text("1763831400000").subtype,
            SmartSubtype::Timestamp
        );
        assert_eq!(
            inspect_text("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.sig_abc").subtype,
            SmartSubtype::Jwt
        );
        assert_eq!(
            inspect_text("fn main() { let x = 1; }").subtype,
            SmartSubtype::Code
        );
    }
}
