use pinyin::ToPinyin;

/// Builds a space-delimited search index containing full pinyin and initials
/// for all CJK characters in `text`, enabling queries such as `jtb` -> "剪贴板".
pub fn build_pinyin_index(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // Cap input scanned for pinyin generation to keep large pastes sub-millisecond.
    let mut full = String::new();
    let mut initials = String::new();
    let mut han_count = 0usize;

    for ch in text.chars().take(1024) {
        if let Some(py) = ch.to_pinyin() {
            let plain = py.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
            han_count += 1;
        }
    }

    if han_count == 0 {
        return String::new();
    }

    format!("{initials} {full}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_initials_and_full_pinyin() {
        let idx = build_pinyin_index("剪贴板截图");
        assert!(idx.contains("jtbjt"));
        assert!(idx.contains("jiantiebanjietu"));
    }

    #[test]
    fn pure_ascii_produces_empty_index() {
        assert_eq!(build_pinyin_index("cargo clippy --workspace"), "");
    }
}
