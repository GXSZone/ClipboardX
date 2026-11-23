use clipboardx_model::{ClipKind, ClipSummary, FilterTab, SmartSubtype};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use regex::Regex;

/// Structured search query extracted from raw user search bar input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub type_filter: Option<FilterTab>,
    pub app_filter: Option<String>,
    pub regex_pattern: Option<String>,
    pub free_text: String,
}

/// Parses optional search prefixes (`type:image`, `app:editor`, `/regex/`) out of `raw`.
pub fn parse_query(raw: &str) -> ParsedQuery {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('/') && trimmed.ends_with('/') {
        return ParsedQuery {
            type_filter: None,
            app_filter: None,
            regex_pattern: Some(trimmed[1..trimmed.len() - 1].to_owned()),
            free_text: String::new(),
        };
    }

    let mut type_filter = None;
    let mut app_filter = None;
    let mut free_tokens = Vec::new();

    for token in trimmed.split_whitespace() {
        if let Some(val) = token.strip_prefix("type:") {
            match val.to_ascii_lowercase().as_str() {
                "text" => type_filter = Some(FilterTab::Text),
                "image" | "img" => type_filter = Some(FilterTab::Image),
                "code" | "json" | "jwt" => type_filter = Some(FilterTab::Code),
                "color" | "hex" => type_filter = Some(FilterTab::Color),
                "files" | "file" | "dir" => type_filter = Some(FilterTab::Files),
                "pinned" | "pin" => type_filter = Some(FilterTab::Pinned),
                "account" | "accounts" | "vault" => type_filter = Some(FilterTab::Accounts),
                _ => free_tokens.push(token),
            }
        } else if let Some(val) = token.strip_prefix("app:")
            && !val.is_empty()
        {
            app_filter = Some(val.to_ascii_lowercase());
        } else {
            free_tokens.push(token);
        }
    }

    ParsedQuery {
        type_filter,
        app_filter,
        regex_pattern: None,
        free_text: free_tokens.join(" "),
    }
}

/// Applies secondary in-memory filtering and fuzzy relevance scoring (`nucleo-matcher`).
pub fn filter_and_rank_summaries(
    items: Vec<ClipSummary>,
    parsed: &ParsedQuery,
) -> Vec<ClipSummary> {
    let regex = parsed
        .regex_pattern
        .as_deref()
        .and_then(|pat| Regex::new(pat).ok());

    let mut filtered: Vec<ClipSummary> = items
        .into_iter()
        .filter(|item| {
            if let Some(tab) = parsed.type_filter
                && !matches_filter_tab(item, tab)
            {
                return false;
            }
            if let Some(app_sub) = &parsed.app_filter {
                let matches_app = item
                    .source_app_name
                    .as_deref()
                    .is_some_and(|name| name.to_ascii_lowercase().contains(app_sub));
                if !matches_app {
                    return false;
                }
            }
            if let Some(re) = &regex {
                return re.is_match(&item.preview_text)
                    || re.is_match(&item.alias)
                    || re.is_match(&item.ocr_text);
            }
            true
        })
        .collect();

    if parsed.free_text.is_empty() {
        return filtered;
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(
        &parsed.free_text,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );

    let mut buf = Vec::new();
    let mut scored: Vec<(i64, ClipSummary)> = filtered
        .drain(..)
        .filter_map(|item| {
            let haystack = format!(
                "{} {} {} {}",
                item.alias,
                item.preview_text,
                item.ocr_text,
                item.source_app_name.as_deref().unwrap_or("")
            );
            let needle_lower = parsed.free_text.to_ascii_lowercase();
            let fuzzy_score = pattern
                .score(Utf32Str::new(&haystack, &mut buf), &mut matcher)
                .map(i64::from);

            let alias_hit =
                !item.alias.is_empty() && item.alias.to_ascii_lowercase().contains(&needle_lower);
            let direct_hit = item
                .preview_text
                .to_ascii_lowercase()
                .contains(&needle_lower)
                || item.ocr_text.to_ascii_lowercase().contains(&needle_lower);

            let base = fuzzy_score.or_else(|| (alias_hit || direct_hit).then_some(50))?;
            let pin_boost = if item.is_pinned { 10_000 } else { 0 };
            let alias_boost = if alias_hit { 2_000 } else { 0 };
            Some((pin_boost + alias_boost + base, item))
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.updated_at_ms.cmp(&a.1.updated_at_ms))
    });
    scored.into_iter().map(|(_, item)| item).collect()
}

fn matches_filter_tab(item: &ClipSummary, tab: FilterTab) -> bool {
    match tab {
        FilterTab::All | FilterTab::Accounts => true,
        FilterTab::Text => matches!(item.kind, ClipKind::Text | ClipKind::RichText),
        FilterTab::Image => item.kind == ClipKind::Image,
        FilterTab::Code => matches!(
            item.subtype,
            SmartSubtype::Code | SmartSubtype::Json | SmartSubtype::Jwt
        ),
        FilterTab::Color => item.subtype == SmartSubtype::Color,
        FilterTab::Files => item.kind == ClipKind::Files,
        FilterTab::Pinned => item.is_pinned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_type_app_and_regex_queries() {
        let q1 = parse_query("type:image app:browser banner");
        assert_eq!(q1.type_filter, Some(FilterTab::Image));
        assert_eq!(q1.app_filter.as_deref(), Some("browser"));
        assert_eq!(q1.free_text, "banner");

        let q2 = parse_query("/^#[0-9a-fA-F]{6}$/");
        assert_eq!(q2.regex_pattern.as_deref(), Some("^#[0-9a-fA-F]{6}$"));
        assert!(q2.free_text.is_empty());
    }
}
