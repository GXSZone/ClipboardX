use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use clipboardx_core::inspector::strip_tracking_params;
use clipboardx_core::service::format_bytes;
use clipboardx_model::{ClipItem, ClipItemId, ClipKind, RgbHex, SmartSubtype};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, FontWeight, HighlightStyle, Image, ImageSource, IntoElement, ObjectFit,
    SharedString, StyledText, Window, div, img, px,
};
use rust_i18n::t;

use crate::icons::kit_icon;
use crate::theme::{
    ThemePalette, contrasting_fg_for_hex, mono_font_family, parse_hex_str_hsla, ui_weight_bold,
};

/// Shared callback aliases for preview card interactive actions.
pub type WindowAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type AppAction = Rc<dyn Fn(&mut App)>;
pub type StringAction = Rc<dyn Fn(String)>;

/// Actions emitted from interactive buttons inside the right preview pane.
#[derive(Clone)]
pub struct PreviewCallbacks {
    pub on_copy: Rc<dyn Fn(ClipItemId, bool)>,
    pub on_copy_snippet: StringAction,
    pub on_open_url: StringAction,
    pub on_toggle_image_scale: AppAction,
    pub on_run_ocr: Rc<dyn Fn(ClipItemId)>,
    pub on_toggle_pin: Rc<dyn Fn(ClipItemId)>,
    pub on_edit_alias: WindowAction,
    pub on_edit_group: WindowAction,
    pub on_edit_content: WindowAction,
    pub on_delete: Rc<dyn Fn(ClipItemId)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxTokenKind {
    Muted,
    Purple,
    Green,
    Amber,
}

/// Precomputed heavy preview artifacts (JSON formatting, JWT decoding, URL parsing, color math,
/// lexical token spans, and filesystem `Path::exists()` checks) cached once when a clip item is
/// selected so VSync animations and rapid hover frames execute with zero disk I/O or redundant parsing.
#[derive(Clone)]
pub struct PreparedPreview {
    pub item_id: ClipItemId,
    pub char_count: usize,
    pub image_dims: (u64, u64),
    pub color_hex: String,
    pub color_formats: Option<ColorFormats>,
    pub file_entries: Vec<(String, bool)>,
    pub url_clean: String,
    pub url_parts: Option<UrlParts>,
    pub url_truncated: String,
    pub timestamp_formats: Option<TimestampFormats>,
    pub jwt_parts: Option<JwtParts>,
    pub jwt_header_compact: String,
    pub jwt_header_tokens: Vec<(Range<usize>, SyntaxTokenKind)>,
    pub jwt_payload_vis: String,
    pub jwt_payload_tokens: Vec<(Range<usize>, SyntaxTokenKind)>,
    pub code_formatted: String,
    pub code_visible: String,
    pub code_tokens: Vec<(Range<usize>, SyntaxTokenKind)>,
    pub code_total_lines: usize,
    pub code_line_numbers: String,
    pub plain_truncated: String,
    pub base64_decoded_truncated: String,
    pub base64_raw_truncated: String,
}

impl PreparedPreview {
    pub fn from_item(item: &ClipItem) -> Self {
        let meta: serde_json::Value =
            serde_json::from_str(&item.summary.meta_json).unwrap_or_default();
        let image_dims = (
            meta["width"].as_u64().unwrap_or(0),
            meta["height"].as_u64().unwrap_or(0),
        );
        let char_count = if matches!(item.summary.kind, ClipKind::Text | ClipKind::RichText)
            && item.summary.subtype != SmartSubtype::Color
        {
            item.full_text.chars().count()
        } else {
            0
        };

        let mut prep = Self {
            item_id: item.summary.id,
            char_count,
            image_dims,
            color_hex: String::new(),
            color_formats: None,
            file_entries: Vec::new(),
            url_clean: String::new(),
            url_parts: None,
            url_truncated: String::new(),
            timestamp_formats: None,
            jwt_parts: None,
            jwt_header_compact: String::new(),
            jwt_header_tokens: Vec::new(),
            jwt_payload_vis: String::new(),
            jwt_payload_tokens: Vec::new(),
            code_formatted: String::new(),
            code_visible: String::new(),
            code_tokens: Vec::new(),
            code_total_lines: 1,
            code_line_numbers: String::new(),
            plain_truncated: String::new(),
            base64_decoded_truncated: String::new(),
            base64_raw_truncated: String::new(),
        };

        match item.summary.kind {
            ClipKind::Image => {}
            ClipKind::Files => {
                let paths: Vec<String> = meta["paths"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_else(|| item.full_text.lines().map(str::to_owned).collect());
                prep.file_entries = paths
                    .into_iter()
                    .take(7)
                    .map(|p| {
                        let exists = Path::new(&p).exists();
                        (p, exists)
                    })
                    .collect();
            }
            ClipKind::Text | ClipKind::RichText => match item.summary.subtype {
                SmartSubtype::Color => {
                    let hex_str = meta["color_hex"]
                        .as_str()
                        .unwrap_or(item.full_text.trim())
                        .to_owned();
                    prep.color_formats = Some(compute_color_formats(&hex_str));
                    prep.color_hex = hex_str;
                }
                SmartSubtype::Url => {
                    let raw_url = item.full_text.trim();
                    prep.url_clean = strip_tracking_params(raw_url);
                    prep.url_parts = Some(parse_url_parts(raw_url));
                    prep.url_truncated = truncate_lines(raw_url, 4);
                }
                SmartSubtype::Jwt => {
                    let decoded = decode_jwt(item.full_text.trim());
                    prep.jwt_header_compact = decoded.header.replace('\n', " ");
                    prep.jwt_header_tokens = tokenize_syntax(&prep.jwt_header_compact, true);
                    prep.jwt_payload_vis = truncate_lines(&decoded.payload, 7);
                    prep.jwt_payload_tokens = tokenize_syntax(&prep.jwt_payload_vis, true);
                    prep.jwt_parts = Some(decoded);
                }
                SmartSubtype::Timestamp => {
                    prep.timestamp_formats = Some(format_unix_timestamp(item.full_text.trim()));
                }
                SmartSubtype::Base64 => {
                    let raw = item.full_text.trim();
                    let decoded = decode_base64_utf8(raw)
                        .unwrap_or_else(|| "<binary base64 payload>".to_owned());
                    prep.base64_decoded_truncated = truncate_lines(&decoded, 10);
                    prep.base64_raw_truncated = truncate_lines(raw, 2);
                }
                SmartSubtype::Json | SmartSubtype::Code => {
                    let is_json = item.summary.subtype == SmartSubtype::Json;
                    let formatted = if is_json && item.full_text.len() <= 64 * 1024 {
                        serde_json::from_str::<serde_json::Value>(item.full_text.trim())
                            .and_then(|v| serde_json::to_string_pretty(&v))
                            .unwrap_or_else(|_| item.full_text.clone())
                    } else {
                        item.full_text.clone()
                    };
                    let visible = truncate_lines(&formatted, 13);
                    let vis_lines = visible.lines().count().max(1);
                    prep.code_line_numbers = (1..=vis_lines)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    prep.code_total_lines = formatted.lines().count().max(1);
                    prep.code_tokens = tokenize_syntax(&visible, is_json);
                    prep.code_visible = visible;
                    prep.code_formatted = formatted;
                }
                SmartSubtype::Plain => {
                    prep.plain_truncated = truncate_lines(&item.full_text, 13);
                }
            },
        }
        prep
    }
}

/// Bundled render inputs keeping `render_preview_card` clean without excessive arguments.
pub struct PreviewRenderProps<'a> {
    pub item: Option<&'a ClipItem>,
    pub prepared: Option<&'a PreparedPreview>,
    pub has_items: bool,
    pub image_preview: Option<Arc<Image>>,
    pub image_actual_size: bool,
    pub group_name: Option<&'a str>,
    pub copied_badge: Option<&'a str>,
    pub native_ocr_tag: &'a str,
}

/// Renders the right-side preview surface (`bg/surface`, radius `12px`,
/// `16px` padding, top single-line meta, main preview body, and action footer bar).
pub fn render_preview_card(
    props: PreviewRenderProps<'_>,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    let Some(item) = props.item else {
        let (title_text, hint_text) = if props.has_items {
            (
                t!("preview.idle_title").to_string(),
                t!("preview.idle_hint").to_string(),
            )
        } else {
            (
                t!("search.empty_title").to_string(),
                t!("search.empty_hint").to_string(),
            )
        };
        return div()
            .size_full()
            .rounded(px(12.0))
            .bg(palette.bg_surface)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .px(px(24.0))
            .child(
                div()
                    .size(px(36.0))
                    .rounded(px(10.0))
                    .bg(palette.bg_elevated)
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(mono_font_family())
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.accent_blue)
                    .child("CX"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(ui_weight_bold())
                    .text_color(palette.text_primary)
                    .child(title_text),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(palette.text_muted)
                    .text_center()
                    .child(hint_text),
            )
            .into_any_element();
    };

    let fallback_prep;
    let prep = if let Some(p) = props.prepared.filter(|p| p.item_id == item.summary.id) {
        p
    } else {
        fallback_prep = PreparedPreview::from_item(item);
        &fallback_prep
    };

    let top_meta = render_clean_top_meta(
        item,
        prep,
        props.group_name,
        props.image_actual_size,
        palette,
        callbacks,
    );
    let main_body = render_main_section(
        item,
        prep,
        props.image_preview,
        props.image_actual_size,
        props.copied_badge,
        props.native_ocr_tag,
        palette,
        callbacks,
    );
    let bottom_actions = render_clean_action_bar(item, palette, callbacks);

    div()
        .size_full()
        .rounded(px(12.0))
        .bg(palette.bg_surface)
        .overflow_hidden()
        .pt(px(14.0))
        .px(px(16.0))
        .pb(px(12.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(top_meta)
        .child(div().flex_1().min_h_0().overflow_hidden().child(main_body))
        .child(bottom_actions)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_main_section(
    item: &ClipItem,
    prep: &PreparedPreview,
    image_preview: Option<Arc<Image>>,
    image_actual_size: bool,
    copied_badge: Option<&str>,
    native_ocr_tag: &str,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    match item.summary.kind {
        ClipKind::Image => render_image_main(
            item,
            prep,
            image_preview,
            image_actual_size,
            native_ocr_tag,
            palette,
            callbacks,
        ),
        ClipKind::Files => render_files_main(prep, palette, &callbacks.on_copy_snippet),
        ClipKind::Text | ClipKind::RichText => match item.summary.subtype {
            SmartSubtype::Color => {
                render_color_main(prep, copied_badge, palette, &callbacks.on_copy_snippet)
            }
            SmartSubtype::Url => render_url_main(
                item,
                prep,
                palette,
                &callbacks.on_copy_snippet,
                &callbacks.on_open_url,
            ),
            SmartSubtype::Jwt => render_jwt_main(prep, palette, &callbacks.on_copy_snippet),
            SmartSubtype::Timestamp => {
                render_timestamp_main(item, prep, palette, &callbacks.on_copy_snippet)
            }
            SmartSubtype::Base64 => render_base64_main(prep, image_preview, palette),
            SmartSubtype::Json | SmartSubtype::Code => {
                render_code_main(item, prep, palette, &callbacks.on_copy_snippet)
            }
            SmartSubtype::Plain => render_plain_text_main(item, prep, palette, callbacks),
        },
    }
}

/// Renders the color swatch hero block and click-to-copy color format rows.
fn render_color_main(
    prep: &PreparedPreview,
    copied_badge: Option<&str>,
    palette: &ThemePalette,
    on_copy_snippet: &StringAction,
) -> AnyElement {
    let hex_str = &prep.color_hex;
    let swatch_bg = parse_hex_str_hsla(hex_str).unwrap_or(palette.accent_blue);
    let swatch_fg = contrasting_fg_for_hex(hex_str);
    let info = prep
        .color_formats
        .clone()
        .unwrap_or_else(|| compute_color_formats(hex_str));

    let best_ratio = info.vs_white.max(info.vs_black);
    let wcag_grade = if best_ratio >= 7.0 {
        "WCAG AAA"
    } else if best_ratio >= 4.5 {
        "WCAG AA"
    } else {
        "AA Large"
    };
    let wcag_badge = format!("{wcag_grade} {:.2}:1", best_ratio);
    let srgb_sub = format!("sRGB {}, {}, {}", info.r, info.g, info.b);

    let rows = [
        ("HEX", info.hex.clone()),
        ("RGB", info.rgb),
        ("HSL", info.hsl),
        ("OKLCH", info.oklch),
    ];

    let mut formats = div().w_full().flex().flex_col().gap(px(3.0));
    for (idx, (key, val)) in rows.into_iter().enumerate() {
        let cb = Rc::clone(on_copy_snippet);
        let val_copy = val.clone();
        let is_first = idx == 0;
        let show_copied = copied_badge.is_some() && is_first;
        formats = formats.child(
            div()
                .id(SharedString::from(format!("color-row-{idx}")))
                .cursor_pointer()
                .w_full()
                .h(px(27.0))
                .rounded(px(7.0))
                .bg(if is_first {
                    palette.bg_elevated
                } else {
                    gpui_kit::transparent_black()
                })
                .hover(|s| s.bg(palette.bg_elevated))
                .px(px(10.0))
                .on_click(move |_, _, _| cb(val_copy.clone()))
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .w(px(40.0))
                        .flex_shrink_0()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .line_height(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(key),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(15.0))
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(val),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(if show_copied {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if show_copied {
                            palette.accent_blue
                        } else {
                            palette.text_muted
                        })
                        .child(if show_copied {
                            t!("preview.copied").to_string()
                        } else {
                            t!("preview.copy").to_string()
                        }),
                ),
        );
    }

    let badge_bg = gpui_kit::hsla(swatch_fg.h, swatch_fg.s, swatch_fg.l, 0.2);

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .w_full()
                .h(px(86.0))
                .rounded(px(10.0))
                .bg(swatch_bg)
                .px(px(14.0))
                .py(px(12.0))
                .flex()
                .flex_col()
                .justify_between()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(999.0))
                                .bg(badge_bg)
                                .font_family(mono_font_family())
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(swatch_fg)
                                .child(wcag_badge),
                        )
                        .child(
                            div()
                                .font_family(mono_font_family())
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .opacity(0.85)
                                .text_color(swatch_fg)
                                .child(srgb_sub),
                        ),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(20.0))
                        .line_height(px(24.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(swatch_fg)
                        .child(info.hex),
                ),
        )
        .child(formats)
        .into_any_element()
}

/// Renders the decoded JWT preview layout: status strip and three sunken code sections.
fn render_jwt_main(
    prep: &PreparedPreview,
    palette: &ThemePalette,
    on_copy_snippet: &StringAction,
) -> AnyElement {
    let Some(decoded) = prep.jwt_parts.as_ref() else {
        return div().into_any_element();
    };
    let copy_payload = decoded.payload.clone();
    let cb = Rc::clone(on_copy_snippet);

    let status_pill = match decoded.is_expired {
        Some(false) => div()
            .px(px(7.0))
            .py(px(2.0))
            .rounded(px(5.0))
            .bg(palette.accent_green_soft)
            .text_size(px(10.0))
            .line_height(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(palette.accent_green)
            .child("● 有效"),
        Some(true) => div()
            .px(px(7.0))
            .py(px(2.0))
            .rounded(px(5.0))
            .bg(palette.accent_amber_soft)
            .text_size(px(10.0))
            .line_height(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(palette.accent_red)
            .child("● 已过期"),
        None => div()
            .px(px(7.0))
            .py(px(2.0))
            .rounded(px(5.0))
            .bg(palette.bg_elevated)
            .text_size(px(10.0))
            .line_height(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(palette.text_secondary)
            .child("JWT Claims"),
    };

    let header_compact = prep.jwt_header_compact.clone();
    let header_hl = tokens_to_highlights(&prep.jwt_header_tokens, palette);
    let payload_vis = prep.jwt_payload_vis.clone();
    let payload_hl = tokens_to_highlights(&prep.jwt_payload_tokens, palette);

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .px(px(7.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(palette.accent_amber_soft)
                                .font_family(mono_font_family())
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(palette.accent_amber)
                                .child(format!("JWT · {}", decoded.alg)),
                        )
                        .child(status_pill),
                )
                .child(
                    div()
                        .id("jwt-copy-json")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(5.0))
                        .bg(palette.bg_elevated)
                        .hover(|s| s.bg(palette.bg_hover))
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_secondary)
                        .on_click(move |_, _, _| cb(copy_payload.clone()))
                        .child("复制 JSON"),
                ),
        )
        .child(
            div()
                .w_full()
                .rounded(px(8.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(7.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.text_muted)
                        .child("HEADER"),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(StyledText::new(header_compact).with_highlights(header_hl)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(px(8.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(7.0))
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.text_muted)
                        .child("PAYLOAD · 本地离线解码"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(palette.text_primary)
                        .child(StyledText::new(payload_vis).with_highlights(payload_hl)),
                ),
        )
        .child(
            div()
                .w_full()
                .rounded(px(8.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(7.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.text_muted)
                        .child("SIGNATURE"),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(palette.text_secondary)
                        .truncate()
                        .child(decoded.signature.clone()),
                ),
        )
        .into_any_element()
}

/// Renders the image preview canvas well and the secondary scrollable OCR card.
fn render_image_main(
    item: &ClipItem,
    prep: &PreparedPreview,
    image_preview: Option<Arc<Image>>,
    actual_size: bool,
    native_ocr_tag: &str,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    let (w, h) = prep.image_dims;
    let ocr_cb = Rc::clone(&callbacks.on_run_ocr);
    let copy_ocr_cb = Rc::clone(&callbacks.on_copy_snippet);
    let item_id = item.summary.id;
    let ocr_text = item.summary.ocr_text.trim().to_owned();
    let has_ocr = !ocr_text.is_empty();
    let ocr_char_count = ocr_text.chars().count();

    let canvas_caption = if w > 0 && h > 0 {
        format!("{w} × {h} · PNG")
    } else {
        format!("PNG · {}", format_bytes(item.summary.byte_size))
    };

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(10.0))
        // Upper sunken image well (bg/code, r=10, spacious breathing room)
        .child(
            div()
                .flex_1()
                .min_h(px(140.0))
                .w_full()
                .rounded(px(10.0))
                .bg(palette.bg_code)
                .pt(px(10.0))
                .px(px(12.0))
                .pb(px(8.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(if let Some(decoded) = image_preview {
                    let mut image_el = img(ImageSource::Image(decoded)).rounded(px(7.0));
                    if !actual_size {
                        if w > 0 && h > 0 {
                            let max_w = 340.0_f32;
                            let max_h = 134.0_f32;
                            let scale = (max_w / w as f32).min(max_h / h as f32).min(1.0);
                            let draw_w = (w as f32 * scale).round().max(16.0);
                            let draw_h = (h as f32 * scale).round().max(16.0);
                            image_el = image_el
                                .w(px(draw_w))
                                .h(px(draw_h))
                                .object_fit(ObjectFit::Contain);
                        } else {
                            image_el = image_el
                                .max_w_full()
                                .max_h(px(134.0))
                                .object_fit(ObjectFit::Contain);
                        }
                    }
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(image_el)
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .text_color(palette.text_muted)
                        .child(format!("PNG {w} × {h}"))
                        .into_any_element()
                })
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.text_muted)
                        .child(canvas_caption),
                ),
        )
        // Lower OCR well (bg/code, r=9): quiet secondary copy button so bottom blue CTA stays sole primary focus.
        .child(
            div()
                .w_full()
                .flex_shrink_0()
                .max_h(px(86.0))
                .rounded(px(9.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(8.0))
                .flex()
                .flex_col()
                .gap(px(5.0))
                .overflow_hidden()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .bg(palette.accent_purple_soft)
                                        .text_size(px(9.0))
                                        .line_height(px(12.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(palette.accent_purple)
                                        .child(format!("OCR · {native_ocr_tag}")),
                                )
                                .when(has_ocr, |el| {
                                    el.child(
                                        div()
                                            .text_size(px(9.0))
                                            .line_height(px(12.0))
                                            .text_color(palette.text_muted)
                                            .child(format!("{ocr_char_count} 字 · 可滚动")),
                                    )
                                }),
                        )
                        .child(if has_ocr {
                            let text_for_copy = ocr_text.clone();
                            div()
                                .id("img-copy-ocr-btn")
                                .cursor_pointer()
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(palette.bg_elevated)
                                .hover(|s| s.bg(palette.bg_hover))
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_secondary)
                                .on_click(move |_, _, _| copy_ocr_cb(text_for_copy.clone()))
                                .child(t!("preview.ocr_copy").to_string())
                                .into_any_element()
                        } else {
                            div()
                                .id("img-run-ocr-btn")
                                .cursor_pointer()
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(palette.bg_elevated)
                                .hover(|s| s.bg(palette.bg_hover))
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_secondary)
                                .on_click(move |_, _, _| ocr_cb(item_id))
                                .child(kit_icon(IconName::ScanText, 10.0, palette.text_secondary))
                                .child(t!("preview.ocr_run").to_string())
                                .into_any_element()
                        }),
                )
                .child(
                    div()
                        .id("img-ocr-text-scroll")
                        .w_full()
                        .max_h(px(48.0))
                        .overflow_y_scroll()
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .text_color(if has_ocr {
                            palette.text_primary
                        } else {
                            palette.text_muted
                        })
                        .child(if has_ocr {
                            ocr_text
                        } else {
                            t!("preview.ocr_empty").to_string()
                        }),
                ),
        )
        .into_any_element()
}

fn render_files_main(
    prep: &PreparedPreview,
    palette: &ThemePalette,
    on_copy_snippet: &StringAction,
) -> AnyElement {
    let mut list = div()
        .size_full()
        .rounded(px(8.0))
        .bg(palette.bg_code)
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(5.0))
        .overflow_hidden();

    for (idx, (path, exists)) in prep.file_entries.iter().enumerate() {
        let exists = *exists;
        let cb = Rc::clone(on_copy_snippet);
        let path_copy = path.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("file-row-{idx}")))
                .cursor_pointer()
                .w_full()
                .rounded(px(7.0))
                .bg(palette.bg_elevated)
                .hover(|s| s.bg(palette.bg_hover))
                .px(px(9.0))
                .py(px(5.0))
                .on_click(move |_, _, _| cb(path_copy.clone()))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(if exists {
                            palette.accent_green_soft
                        } else {
                            palette.bg_code
                        })
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if exists {
                            palette.accent_green
                        } else {
                            palette.text_muted
                        })
                        .child(if exists {
                            "FILE".to_owned()
                        } else {
                            t!("preview.missing_file").to_string()
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(15.0))
                        .text_color(if exists {
                            palette.text_primary
                        } else {
                            palette.text_muted
                        })
                        .truncate()
                        .child(path.clone()),
                ),
        );
    }

    list.into_any_element()
}

fn render_url_main(
    item: &ClipItem,
    prep: &PreparedPreview,
    palette: &ThemePalette,
    on_copy_snippet: &StringAction,
    _on_open_url: &StringAction,
) -> AnyElement {
    let raw_url = item.full_text.trim().to_owned();
    let parsed = prep
        .url_parts
        .clone()
        .unwrap_or_else(|| parse_url_parts(&raw_url));
    let mut query_list = div().w_full().flex().flex_col().gap(px(5.0));
    for (idx, (k, v)) in parsed.params.iter().take(4).enumerate() {
        let cb = Rc::clone(on_copy_snippet);
        let v_copy = v.clone();
        let is_tracking = k.starts_with("utm_")
            || matches!(
                k.to_ascii_lowercase().as_str(),
                "fbclid" | "gclid" | "spm" | "ref" | "igshid"
            );
        query_list = query_list.child(
            div()
                .id(SharedString::from(format!("url-param-{idx}")))
                .cursor_pointer()
                .w_full()
                .rounded(px(7.0))
                .bg(palette.bg_elevated)
                .hover(|s| s.bg(palette.bg_hover))
                .px(px(10.0))
                .py(px(6.0))
                .on_click(move |_, _, _| cb(v_copy.clone()))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .line_height(px(14.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if is_tracking {
                            palette.accent_amber
                        } else {
                            palette.accent_blue
                        })
                        .child(k.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .line_height(px(14.0))
                        .text_color(if is_tracking {
                            palette.text_muted
                        } else {
                            palette.text_primary
                        })
                        .truncate()
                        .text_right()
                        .child(if is_tracking {
                            format!("{v} · 可净化")
                        } else {
                            v.clone()
                        }),
                ),
        );
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .w_full()
                .rounded(px(9.0))
                .bg(palette.bg_code)
                .px(px(12.0))
                .py(px(10.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(palette.accent_blue_soft)
                                .font_family(mono_font_family())
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(palette.accent_blue)
                                .child(parsed.scheme.to_uppercase()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(mono_font_family())
                                .text_size(px(12.0))
                                .line_height(px(16.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_primary)
                                .truncate()
                                .child(parsed.host),
                        ),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .text_color(palette.text_secondary)
                        .child(prep.url_truncated.clone()),
                ),
        )
        .when(!parsed.params.is_empty(), |el| {
            el.child(
                div()
                    .w_full()
                    .px(px(2.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.text_muted)
                            .child("查询参数（单击任意项复制值）"),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_muted)
                            .child(format!("{} 项", parsed.params.len())),
                    ),
            )
            .child(query_list)
        })
        .into_any_element()
}

fn render_timestamp_main(
    item: &ClipItem,
    prep: &PreparedPreview,
    palette: &ThemePalette,
    on_copy_snippet: &StringAction,
) -> AnyElement {
    let info = prep
        .timestamp_formats
        .clone()
        .unwrap_or_else(|| format_unix_timestamp(item.full_text.trim()));
    let rows = [
        ("ISO 8601 (UTC)", info.iso_utc),
        ("Seconds", info.seconds),
        ("Milliseconds", info.millis),
    ];

    let mut list = div().size_full().flex().flex_col().gap(px(6.0));

    for (idx, (label, val)) in rows.into_iter().enumerate() {
        let cb = Rc::clone(on_copy_snippet);
        let val_copy = val.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("ts-row-{idx}")))
                .cursor_pointer()
                .w_full()
                .rounded(px(7.0))
                .bg(palette.bg_elevated)
                .hover(|s| s.bg(palette.bg_hover))
                .pl(px(10.0))
                .pr(px(6.0))
                .py(px(6.0))
                .on_click(move |_, _, _| cb(val_copy.clone()))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .font_family(mono_font_family())
                                .text_size(px(11.0))
                                .line_height(px(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_primary)
                                .child(val),
                        )
                        .child(
                            div()
                                .px(px(7.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(palette.bg_surface)
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .text_color(palette.text_secondary)
                                .child(t!("preview.copy").to_string()),
                        ),
                ),
        );
    }

    list.into_any_element()
}

fn render_base64_main(
    prep: &PreparedPreview,
    inline_image: Option<Arc<Image>>,
    palette: &ThemePalette,
) -> AnyElement {
    if let Some(decoded_img) = inline_image {
        return div()
            .size_full()
            .rounded(px(8.0))
            .bg(palette.bg_code)
            .p(px(10.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(ImageSource::Image(decoded_img))
                            .max_w_full()
                            .max_h(px(184.0))
                            .object_fit(ObjectFit::Contain)
                            .rounded(px(6.0)),
                    ),
            )
            .child(
                div()
                    .px(px(7.0))
                    .py(px(2.0))
                    .rounded(px(5.0))
                    .bg(palette.accent_purple_soft)
                    .text_size(px(9.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.accent_purple)
                    .child(t!("preview.inline_image").to_string()),
            )
            .into_any_element();
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .rounded(px(8.0))
                .bg(palette.bg_code)
                .p(px(9.0))
                .overflow_hidden()
                .font_family(mono_font_family())
                .text_size(px(11.0))
                .line_height(px(15.0))
                .text_color(palette.text_primary)
                .child(prep.base64_decoded_truncated.clone()),
        )
        .child(
            div()
                .rounded(px(8.0))
                .bg(palette.bg_elevated)
                .p(px(8.0))
                .font_family(mono_font_family())
                .text_size(px(10.0))
                .line_height(px(13.0))
                .text_color(palette.text_muted)
                .child(prep.base64_raw_truncated.clone()),
        )
        .into_any_element()
}

fn render_code_main(
    item: &ClipItem,
    prep: &PreparedPreview,
    palette: &ThemePalette,
    _on_copy_snippet: &StringAction,
) -> AnyElement {
    let lang = item
        .summary
        .detected_lang
        .as_deref()
        .unwrap_or("code")
        .to_uppercase();
    let is_json = item.summary.subtype == SmartSubtype::Json;
    let visible = prep.code_visible.clone();
    let line_numbers = prep.code_line_numbers.clone();
    let highlights = tokens_to_highlights(&prep.code_tokens, palette);
    let badge_text = if is_json {
        "JSON · UTF-8".to_owned()
    } else {
        lang
    };
    let footer_hint = if is_json {
        format!("{} 字符 · 已自动缩进", prep.char_count)
    } else {
        format!("{} 行 · {} 字符", prep.code_total_lines, prep.char_count)
    };

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(7.0))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(px(9.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(9.0))
                .overflow_hidden()
                .flex()
                .gap(px(9.0))
                .child(
                    div()
                        .w(px(18.0))
                        .flex_shrink_0()
                        .font_family(mono_font_family())
                        .text_size(px(10.0))
                        .line_height(px(16.0))
                        .text_color(palette.text_muted)
                        .text_right()
                        .child(line_numbers),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .text_color(palette.text_primary)
                        .child(StyledText::new(visible).with_highlights(highlights)),
                ),
        )
        .child(
            div()
                .w_full()
                .px(px(2.0))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(if is_json {
                            palette.accent_green_soft
                        } else {
                            palette.accent_purple_soft
                        })
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if is_json {
                            palette.accent_green
                        } else {
                            palette.accent_purple
                        })
                        .child(badge_text),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.text_muted)
                        .child(footer_hint),
                ),
        )
        .into_any_element()
}

fn render_plain_text_main(
    item: &ClipItem,
    prep: &PreparedPreview,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    let id = item.summary.id;
    let on_copy_plain = Rc::clone(&callbacks.on_copy);
    let is_rtf = item.summary.kind == ClipKind::RichText;

    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .when(is_rtf, |el| {
            el.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(5.0))
                            .bg(palette.accent_blue_soft)
                            .font_family(mono_font_family())
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(palette.accent_blue)
                            .child("RTF"),
                    )
                    .child(
                        div()
                            .id("plain-copy-btn")
                            .cursor_pointer()
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(5.0))
                            .bg(palette.bg_elevated)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(10.0))
                            .text_color(palette.text_secondary)
                            .on_click(move |_, _, _| on_copy_plain(id, true))
                            .child(t!("footer.paste_plain").to_string()),
                    ),
            )
        })
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(px(8.0))
                .bg(palette.bg_code)
                .px(px(10.0))
                .py(px(9.0))
                .overflow_hidden()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(palette.text_primary)
                .child(prep.plain_truncated.clone()),
        )
        .into_any_element()
}

/// Renders the top metadata header with type indicator, title/alias, group tag, and utility action.
fn render_clean_top_meta(
    item: &ClipItem,
    prep: &PreparedPreview,
    group_name: Option<&str>,
    image_actual_size: bool,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    let summary = &item.summary;
    let source = summary.source_app_name.as_deref().unwrap_or("System");
    let alias_trimmed = summary.alias.trim();
    let has_alias = !alias_trimmed.is_empty();

    let (kind_label, dot_color) = match summary.kind {
        ClipKind::Image => (t!("filter.image").to_string(), palette.accent_purple),
        ClipKind::Files => (t!("filter.files").to_string(), palette.text_secondary),
        ClipKind::Text | ClipKind::RichText => match summary.subtype {
            SmartSubtype::Color => (t!("filter.color").to_string(), palette.accent_blue),
            SmartSubtype::Url => ("链接".to_owned(), palette.accent_blue),
            SmartSubtype::Jwt => ("JWT 令牌".to_owned(), palette.accent_amber),
            SmartSubtype::Json => ("JSON".to_owned(), palette.accent_green),
            SmartSubtype::Code => (t!("filter.code").to_string(), palette.accent_purple),
            SmartSubtype::Timestamp => ("时间戳".to_owned(), palette.accent_amber),
            SmartSubtype::Base64 => ("Base64".to_owned(), palette.accent_purple),
            SmartSubtype::Plain => (t!("filter.text").to_string(), palette.accent_blue),
        },
    };
    let time_str = format_relative_time(summary.updated_at_ms);
    let size_metric = match summary.kind {
        ClipKind::Text | ClipKind::RichText
            if matches!(summary.subtype, SmartSubtype::Json | SmartSubtype::Code) =>
        {
            format!("{} 行", prep.code_total_lines)
        }
        ClipKind::Text | ClipKind::RichText if summary.subtype != SmartSubtype::Color => {
            format!("{} ch", prep.char_count)
        }
        _ => format_bytes(summary.byte_size),
    };
    let on_group = Rc::clone(&callbacks.on_edit_group);

    let utility_chip: Option<AnyElement> = match summary.kind {
        ClipKind::Image => {
            let toggle_cb = Rc::clone(&callbacks.on_toggle_image_scale);
            let label = if image_actual_size {
                t!("preview.actual_scale").to_string()
            } else {
                t!("preview.fit_scale").to_string()
            };
            Some(
                div()
                    .id("top-img-scale-chip")
                    .cursor_pointer()
                    .px(px(7.0))
                    .py(px(2.0))
                    .rounded(px(5.0))
                    .bg(palette.bg_elevated)
                    .hover(|s| s.bg(palette.bg_hover))
                    .text_size(px(9.0))
                    .line_height(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.text_secondary)
                    .on_click(move |_, _, cx| toggle_cb(cx))
                    .child(label)
                    .into_any_element(),
            )
        }
        ClipKind::Text | ClipKind::RichText => match summary.subtype {
            SmartSubtype::Json | SmartSubtype::Code => {
                let cb = Rc::clone(&callbacks.on_copy_snippet);
                let text = prep.code_formatted.clone();
                Some(
                    div()
                        .id("top-code-copy-chip")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(5.0))
                        .bg(palette.bg_elevated)
                        .hover(|s| s.bg(palette.bg_hover))
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_secondary)
                        .on_click(move |_, _, _| cb(text.clone()))
                        .child(if summary.subtype == SmartSubtype::Json {
                            "复制格式化".to_owned()
                        } else {
                            t!("preview.copy").to_string()
                        })
                        .into_any_element(),
                )
            }
            SmartSubtype::Url => {
                let cb = Rc::clone(&callbacks.on_copy_snippet);
                let clean = prep.url_clean.clone();
                Some(
                    div()
                        .id("top-url-clean-chip")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(5.0))
                        .bg(palette.bg_elevated)
                        .hover(|s| s.bg(palette.bg_hover))
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_secondary)
                        .on_click(move |_, _, _| cb(clean.clone()))
                        .child("去除追踪参数")
                        .into_any_element(),
                )
            }
            _ => None,
        },
        ClipKind::Files => None,
    };

    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .size(px(7.0))
                        .rounded(px(999.0))
                        .flex_shrink_0()
                        .bg(dot_color),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(px(16.0))
                        .font_weight(if has_alias {
                            ui_weight_bold()
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(if has_alias {
                            alias_trimmed.to_owned()
                        } else {
                            kind_label
                        }),
                )
                .when(!has_alias, |el| {
                    el.child(
                        div()
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .text_color(palette.text_muted)
                            .truncate()
                            .child(format!("· {source}")),
                    )
                })
                .when_some(group_name, |el, gname| {
                    let cb = Rc::clone(&on_group);
                    el.child(
                        div()
                            .id("meta-group-tag")
                            .cursor_pointer()
                            .flex_shrink_0()
                            .px(px(6.0))
                            .py(px(1.5))
                            .rounded(px(4.0))
                            .bg(palette.accent_blue_soft)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(9.0))
                            .line_height(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.accent_blue)
                            .on_click(move |_, win, cx| cb(win, cx))
                            .child(gname.to_owned()),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.text_muted)
                        .child(format!("{time_str} · {size_metric}")),
                )
                .when_some(utility_chip, |el, chip| el.child(chip)),
        )
        .into_any_element()
}

/// Renders the bottom action bar with secondary item operations and the primary paste action.
fn render_clean_action_bar(
    item: &ClipItem,
    palette: &ThemePalette,
    callbacks: &PreviewCallbacks,
) -> AnyElement {
    let id = item.summary.id;
    let is_pinned = item.summary.is_pinned;
    let has_alias = !item.summary.alias.trim().is_empty();
    let has_group = item.summary.group_id.is_some();
    let is_editable_text = matches!(item.summary.kind, ClipKind::Text | ClipKind::RichText);
    let is_url = is_editable_text && item.summary.subtype == SmartSubtype::Url;
    let show_plain_btn = item.summary.kind == ClipKind::RichText
        || matches!(
            item.summary.subtype,
            SmartSubtype::Json | SmartSubtype::Code
        );

    let on_pin = Rc::clone(&callbacks.on_toggle_pin);
    let on_alias = Rc::clone(&callbacks.on_edit_alias);
    let on_group = Rc::clone(&callbacks.on_edit_group);
    let on_edit = Rc::clone(&callbacks.on_edit_content);
    let on_del = Rc::clone(&callbacks.on_delete);
    let on_paste = Rc::clone(&callbacks.on_copy);
    let on_paste_plain = Rc::clone(&callbacks.on_copy);
    let on_open_url = Rc::clone(&callbacks.on_open_url);
    let raw_url = item.full_text.trim().to_owned();

    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .child(
                    div()
                        .id("preview-act-pin")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(palette.bg_elevated).text_color(palette.text_primary))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(if is_pinned {
                            palette.accent_blue
                        } else {
                            palette.text_muted
                        })
                        .on_click(move |_, _, _| on_pin(id))
                        .child(if is_pinned {
                            t!("footer.pinned_state").to_string()
                        } else {
                            t!("footer.pin").to_string()
                        }),
                )
                .child(
                    div()
                        .id("preview-act-alias")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(palette.bg_elevated).text_color(palette.text_primary))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(if has_alias {
                            palette.text_primary
                        } else {
                            palette.text_muted
                        })
                        .on_click(move |_, win, cx| on_alias(win, cx))
                        .child(t!("preview.alias").to_string()),
                )
                .child(
                    div()
                        .id("preview-act-group")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(palette.bg_elevated).text_color(palette.text_primary))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(if has_group {
                            palette.accent_blue
                        } else {
                            palette.text_muted
                        })
                        .on_click(move |_, win, cx| on_group(win, cx))
                        .child("分组"),
                )
                .when(is_editable_text, |el| {
                    el.child(
                        div()
                            .id("preview-act-edit")
                            .cursor_pointer()
                            .px(px(7.0))
                            .py(px(4.0))
                            .rounded(px(6.0))
                            .hover(|s| s.bg(palette.bg_elevated).text_color(palette.text_primary))
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .text_color(palette.text_muted)
                            .on_click(move |_, win, cx| on_edit(win, cx))
                            .child(t!("preview.edit").to_string()),
                    )
                })
                .child(
                    div()
                        .id("preview-act-del")
                        .cursor_pointer()
                        .px(px(7.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .hover(|s| s.bg(palette.bg_elevated).text_color(palette.accent_red))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(palette.text_muted)
                        .on_click(move |_, _, _| on_del(id))
                        .child(t!("preview.delete").to_string()),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .when(is_url, |el| {
                    let u = raw_url.clone();
                    el.child(
                        div()
                            .id("preview-act-open-url")
                            .cursor_pointer()
                            .px(px(10.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(palette.bg_elevated)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .text_color(palette.text_secondary)
                            .on_click(move |_, _, _| on_open_url(u.clone()))
                            .child(t!("url.open_browser").to_string()),
                    )
                })
                .when(!is_url && show_plain_btn, |el| {
                    el.child(
                        div()
                            .id("preview-act-paste-plain")
                            .cursor_pointer()
                            .px(px(10.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(palette.bg_elevated)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .text_color(palette.text_secondary)
                            .on_click(move |_, _, _| on_paste_plain(id, true))
                            .child("纯文本"),
                    )
                })
                .child(
                    div()
                        .id("preview-act-paste")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .px(px(13.0))
                        .py(px(5.0))
                        .rounded(px(7.0))
                        .bg(palette.accent_blue)
                        .text_size(px(11.0))
                        .line_height(px(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui_kit::rgb(0xFFFFFF))
                        .on_click(move |_, _, _| on_paste(id, false))
                        .child(t!("footer.paste_now").to_string()),
                ),
        )
        .into_any_element()
}

/// Formats relative elapsed timestamps concisely for list rows and metadata.
pub fn format_relative_time(timestamp_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(timestamp_ms);
    let diff_secs = ((now_ms - timestamp_ms) / 1000).max(0);
    if diff_secs < 45 {
        "刚刚".to_owned()
    } else if diff_secs < 3600 {
        format!("{}m", (diff_secs / 60).max(1))
    } else if diff_secs < 86_400 {
        format!("{}h", diff_secs / 3600)
    } else {
        format!("{}d", diff_secs / 86_400)
    }
}

fn truncate_lines(input: &str, max_lines: usize) -> String {
    let mut lines: Vec<&str> = input.lines().take(max_lines).collect();
    if input.lines().count() > max_lines {
        lines.push("...");
    }
    let out = lines.join("\n");
    if out.chars().count() > 1400 {
        let mut s: String = out.chars().take(1400).collect();
        s.push('…');
        return s;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorFormats {
    pub hex: String,
    pub rgb: String,
    pub hsl: String,
    pub oklch: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub vs_white: f64,
    pub vs_black: f64,
}

fn compute_color_formats(hex: &str) -> ColorFormats {
    let cleaned = hex.trim().strip_prefix('#').unwrap_or("3B82F6");
    let padded = if cleaned.len() >= 6 {
        &cleaned[..6]
    } else {
        "3B82F6"
    };
    let val = u32::from_str_radix(padded, 16).unwrap_or(0x3B82F6);
    let rgb_hex = RgbHex(val);
    let r = rgb_hex.r_u8();
    let g = rgb_hex.g_u8();
    let b = rgb_hex.b_u8();

    let rf = f64::from(r) / 255.0;
    let gf = f64::from(g) / 255.0;
    let bf = f64::from(b) / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let (h, s) = if (max - min).abs() < 1e-9 {
        (0.0, 0.0)
    } else {
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let mut h = if (max - rf).abs() < 1e-9 {
            (gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }
        } else if (max - gf).abs() < 1e-9 {
            (bf - rf) / d + 2.0
        } else {
            (rf - gf) / d + 4.0
        };
        h /= 6.0;
        (h * 360.0, s * 100.0)
    };

    // Standard sRGB to OKLab / OKLCH conversion (Bjorn Ottosson matrix).
    let lin = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let lr = lin(rf);
    let lg = lin(gf);
    let lb = lin(bf);
    let l_ = (0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb).cbrt();
    let m_ = (0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb).cbrt();
    let s_ = (0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb).cbrt();
    let ok_l = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
    let ok_a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let ok_b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;
    let ok_c = ok_a.hypot(ok_b);
    let mut ok_h = ok_b.atan2(ok_a).to_degrees();
    if ok_h < 0.0 {
        ok_h += 360.0;
    }

    let rel_lum = 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
    let vs_white = 1.05 / (rel_lum + 0.05);
    let vs_black = (rel_lum + 0.05) / 0.05;

    ColorFormats {
        hex: format!("#{padded}").to_uppercase(),
        rgb: format!("rgb({r}, {g}, {b})"),
        hsl: format!("hsl({:.0}, {:.0}%, {:.0}%)", h, s, l * 100.0),
        oklch: format!("oklch({:.3} {:.3} {:.1})", ok_l, ok_c, ok_h),
        r,
        g,
        b,
        vs_white,
        vs_black,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UrlParts {
    pub scheme: String,
    pub host: String,
    pub params: Vec<(String, String)>,
}

fn parse_url_parts(raw: &str) -> UrlParts {
    let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
    let (authority_and_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let host = authority_and_path
        .split('/')
        .next()
        .unwrap_or(authority_and_path)
        .to_owned();
    let params = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (k.to_owned(), v.to_owned())
        })
        .collect();
    UrlParts {
        scheme: scheme.to_owned(),
        host,
        params,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimestampFormats {
    pub iso_utc: String,
    pub seconds: String,
    pub millis: String,
}

fn format_unix_timestamp(raw: &str) -> TimestampFormats {
    let val = raw.parse::<i64>().unwrap_or(0);
    let (secs, ms) = if raw.len() >= 13 {
        (val.div_euclid(1000), val)
    } else {
        (val, val.saturating_mul(1000))
    };
    let (year, month, day, hour, min, sec) = civil_from_unix_seconds(secs);
    TimestampFormats {
        iso_utc: format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z"),
        seconds: secs.to_string(),
        millis: ms.to_string(),
    }
}

/// Howard Hinnant's civil-from-days algorithm in pure Rust without external date crates.
fn civil_from_unix_seconds(unix_secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix_secs.div_euclid(86_400);
    let rem = unix_secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u32;
    let min = ((rem % 3600) / 60) as u32;
    let sec = (rem % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32, hour, min, sec)
}

#[derive(Clone, Debug, PartialEq)]
pub struct JwtParts {
    pub alg: String,
    pub header: String,
    pub payload: String,
    pub signature: String,
    pub is_expired: Option<bool>,
}

fn decode_jwt(raw: &str) -> JwtParts {
    let mut segments = raw.split('.');
    let h_raw = segments.next().unwrap_or("");
    let p_raw = segments.next().unwrap_or("");
    let s_raw = segments.next().unwrap_or("");
    let header_text = decode_base64_utf8(h_raw).unwrap_or_default();
    let payload_text = decode_base64_utf8(p_raw).unwrap_or_default();

    let alg = serde_json::from_str::<serde_json::Value>(&header_text)
        .ok()
        .and_then(|v| v["alg"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "JWT".to_owned());

    let is_expired = serde_json::from_str::<serde_json::Value>(&payload_text)
        .ok()
        .and_then(|v| v["exp"].as_i64())
        .map(|exp| {
            let now_s = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            exp < now_s
        });

    JwtParts {
        alg,
        header: pretty_json_or_raw(&header_text),
        payload: pretty_json_or_raw(&payload_text),
        signature: s_raw.to_owned(),
        is_expired,
    }
}

fn pretty_json_or_raw(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| raw.to_owned())
}

fn decode_base64_utf8(input: &str) -> Option<String> {
    let bytes = decode_base64_bytes(input)?;
    String::from_utf8(bytes).ok()
}

/// Decodes raw Base64 or Base64URL payloads into owned bytes for UTF-8 text or inline image previews.
pub fn decode_base64_bytes(input: &str) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u8 = 0;

    for b in input.bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push(((acc >> bits) & 0xFF) as u8);
        }
    }

    Some(buf)
}

/// Maps precomputed lexical token spans to active theme `HighlightStyle` colors in O(tokens).
fn tokens_to_highlights(
    tokens: &[(Range<usize>, SyntaxTokenKind)],
    palette: &ThemePalette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    tokens
        .iter()
        .map(|(range, kind)| {
            let color = match kind {
                SyntaxTokenKind::Muted => palette.text_muted,
                SyntaxTokenKind::Purple => palette.accent_purple,
                SyntaxTokenKind::Green => palette.accent_green,
                SyntaxTokenKind::Amber => palette.accent_amber,
            };
            (
                range.clone(),
                HighlightStyle {
                    color: Some(color),
                    ..Default::default()
                },
            )
        })
        .collect()
}

/// Lightweight deterministic lexical tokenizer emitting strictly ordered UTF-8 byte ranges.
fn tokenize_syntax(text: &str, is_json: bool) -> Vec<(Range<usize>, SyntaxTokenKind)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        if !is_json && ((b == b'/' && i + 1 < len && bytes[i + 1] == b'/') || b == b'#') {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, SyntaxTokenKind::Muted));
            continue;
        }

        if b == b'"' || (!is_json && b == b'\'') {
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            let mut look = i;
            while look < len && bytes[look] == b' ' {
                look += 1;
            }
            let is_json_key = is_json && look < len && bytes[look] == b':';
            let kind = if is_json_key {
                SyntaxTokenKind::Purple
            } else {
                SyntaxTokenKind::Green
            };
            out.push((start..i, kind));
            continue;
        }

        if b.is_ascii_digit()
            && (i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_'))
        {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            out.push((start..i, SyntaxTokenKind::Amber));
            continue;
        }

        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let is_kw = matches!(
                word,
                "fn" | "let"
                    | "mut"
                    | "pub"
                    | "struct"
                    | "enum"
                    | "impl"
                    | "return"
                    | "async"
                    | "await"
                    | "const"
                    | "function"
                    | "import"
                    | "export"
                    | "def"
                    | "class"
                    | "SELECT"
                    | "FROM"
                    | "WHERE"
                    | "INSERT"
                    | "UPDATE"
                    | "true"
                    | "false"
                    | "null"
                    | "None"
                    | "Some"
                    | "Ok"
                    | "Err"
            );
            if is_kw {
                out.push((start..i, SyntaxTokenKind::Purple));
            }
            continue;
        }

        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }

    out
}

#[cfg(test)]
fn syntax_highlight_ranges(
    text: &str,
    is_json: bool,
    palette: &ThemePalette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    tokens_to_highlights(&tokenize_syntax(text, is_json), palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipboardx_model::ThemeId;

    #[test]
    fn computes_wcag_and_oklch_for_primary_blue() {
        let info = compute_color_formats("#3B82F6");
        assert_eq!(info.hex, "#3B82F6");
        assert_eq!(info.rgb, "rgb(59, 130, 246)");
        assert!(info.oklch.starts_with("oklch(0.62"));
    }

    #[test]
    fn formats_unix_epoch_and_modern_timestamp() {
        assert_eq!(format_unix_timestamp("0").iso_utc, "1970-01-01T00:00:00Z");
        assert_eq!(
            format_unix_timestamp("1700000000").iso_utc,
            "2023-11-14T22:13:20Z"
        );
    }

    #[test]
    fn decodes_base64_url_safe_jwt_segments() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.sig";
        let parts = decode_jwt(jwt);
        assert_eq!(parts.alg, "HS256");
        assert!(parts.header.contains("HS256"));
        assert!(parts.payload.contains("1234567890"));
        assert_eq!(parts.signature, "sig");
    }

    #[test]
    fn lexical_highlighter_respects_utf8_boundaries_and_non_overlapping_ranges() {
        let palette = ThemePalette::from_theme_id(ThemeId::ZedOneDark);
        let sample = "{\n  \"title\": \"剪贴板\",\n  \"count\": 42,\n  \"ok\": true\n}";
        let ranges = syntax_highlight_ranges(sample, true, &palette);
        assert!(!ranges.is_empty());
        let mut prev_end = 0;
        for (range, _) in ranges {
            assert!(range.start >= prev_end);
            assert!(sample.is_char_boundary(range.start));
            assert!(sample.is_char_boundary(range.end));
            prev_end = range.end;
        }
    }

    #[test]
    fn prepared_preview_caches_json_color_and_url_artifacts() {
        use clipboardx_model::ClipSummary;

        let json_item = ClipItem {
            summary: ClipSummary {
                id: ClipItemId(7),
                kind: ClipKind::Text,
                subtype: SmartSubtype::Json,
                content_hash: "h".to_owned(),
                preview_text: "{\"a\":1}".to_owned(),
                alias: String::new(),
                ocr_text: String::new(),
                source_app_name: None,
                byte_size: 7,
                is_pinned: false,
                group_id: None,
                created_at_ms: 0,
                updated_at_ms: 0,
                use_count: 1,
                thumb_ref: None,
                detected_lang: Some("json".to_owned()),
                meta_json: "{}".to_owned(),
            },
            full_text: "{\"a\":1,\"b\":2}".to_owned(),
            payload_inline: None,
            blob_ref: None,
        };
        let prep_json = PreparedPreview::from_item(&json_item);
        assert_eq!(prep_json.item_id, ClipItemId(7));
        assert_eq!(prep_json.char_count, 13);
        assert_eq!(prep_json.code_total_lines, 4);
        assert_eq!(prep_json.code_line_numbers, "1\n2\n3\n4");

        let mut url_item = json_item;
        url_item.summary.id = ClipItemId(8);
        url_item.summary.subtype = SmartSubtype::Url;
        url_item.full_text = "https://example.com/path?utm_source=x&key=val".to_owned();
        let prep_url = PreparedPreview::from_item(&url_item);
        assert_eq!(prep_url.url_clean, "https://example.com/path?key=val");
        assert_eq!(
            prep_url.url_parts.as_ref().map(|p| p.host.as_str()),
            Some("example.com")
        );
    }
}
