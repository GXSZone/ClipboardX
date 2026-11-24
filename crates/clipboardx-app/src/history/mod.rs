pub mod preview;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clipboardx_core::inspector::parse_data_image_uri;
use clipboardx_model::{
    ClipItem, ClipItemId, ClipKind, ClipSummary, FilterTab, Group, GroupId, LayoutSpec,
    PlatformBridge, SmartSubtype, VaultEntry, VaultEntryId, VaultUnlockPolicy,
};
use clipboardx_platform::{AutofillAction, open_external_url};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Image, ImageFormat,
    ImageSource, KeyDownEvent, ListAlignment, ListState, MouseButton, ObjectFit, Pixels, Point,
    Render, SharedString, Subscription, Task, Window, div, img, list, px,
};
use rust_i18n::t;

use crate::history::preview::{
    PreparedPreview, PreviewCallbacks, PreviewRenderProps, decode_base64_bytes,
    format_relative_time, render_preview_card,
};
use crate::icons::{
    AppIcon, app_icon, input_shield, is_blind_dragging, kit_icon, reset_blind_drag_click_guard,
    was_blind_drag_consumed,
};
use crate::state::AppRuntime;
use crate::theme::{
    ThemePalette, extract_color_swatch_hsla, resolve_theme_id, sync_component_theme,
    ui_font_family, ui_weight_bold,
};
use crate::vault::{
    RowContextMenuCb, RowHoverCb, RowIndexCb, VaultCallbacks, VaultPaneProps, VaultRowActions,
    VaultUiState, render_vault_list_row, render_vault_right_pane,
};

const WARM_THUMB_BATCH: usize = 2;
const TAB_HOVER_DWELL_MS: u64 = 55;
const DUAL_LEFT_LIST_WIDTH: f32 = 256.0;

/// Window action requested by the panel view to the shell controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRequest {
    HideWindow,
    AfterPaste,
    OpenSettings,
    ToggleCompact,
    ToggleWindowPin,
    HoverImagePreview(Option<ClipItemId>),
}

#[derive(Debug, Clone)]
enum RowEntry {
    Clip(ClipSummary),
    Vault(VaultEntry),
}

#[derive(Debug, Clone)]
struct ContextMenuState {
    row_index: usize,
    position: Point<Pixels>,
}

/// Computes the next `(active_index, pointer_hovered_index)` on row hover enter/leave:
/// - Hovering into a row updates both `active_index` and `pointer_hovered_index`.
/// - Leaving a row clears only `pointer_hovered_index` (when it matches `idx`) while keeping
///   `active_index` pinned on the last hovered row so the single `bg_hover` highlight & preview stay put.
pub(crate) fn next_hover_state(
    active_index: Option<usize>,
    pointer_hovered_index: Option<usize>,
    idx: usize,
    entered: bool,
) -> (Option<usize>, Option<usize>) {
    if entered {
        (Some(idx), Some(idx))
    } else if pointer_hovered_index == Some(idx) {
        (active_index, None)
    } else {
        (active_index, pointer_hovered_index)
    }
}

/// Primary clipboard & account-vault panel rendering both the 680x420 dual-pane layout
/// and the 328x420 slim single-column mode with 180ms VSync-driven smooth collapse animation,
/// unified single hover-sticky highlight, hover-to-switch tabs, instant image preview, and context menus.
pub struct HistoryPanel {
    runtime: Rc<AppRuntime>,
    on_request: Rc<dyn Fn(PanelRequest)>,
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    alias_input: Entity<InputState>,
    alias_editing_id: Option<ClipItemId>,
    group_input: Entity<InputState>,
    group_editing_id: Option<ClipItemId>,
    content_input: Entity<TextareaState>,
    content_editing_id: Option<ClipItemId>,
    group_popover_open: bool,
    image_actual_size: bool,
    vault_ui: VaultUiState,
    _subscriptions: Vec<Subscription>,
    list_state: ListState,

    palette: ThemePalette,
    is_slim: bool,
    window_pinned: bool,

    active_tab: FilterTab,
    hover_tab_task: Option<Task<()>>,
    active_group: Option<GroupId>,
    groups: Vec<Group>,
    search_query: String,
    rows: Rc<[RowEntry]>,
    total_count: u64,
    active_index: Option<usize>,
    pointer_hovered_index: Option<usize>,
    context_menu: Option<ContextMenuState>,
    selected_item: Option<ClipItem>,
    prepared_preview: Option<PreparedPreview>,
    item_cache: HashMap<ClipItemId, (ClipItem, PreparedPreview)>,
    selected_vault: Option<VaultEntry>,
    vault_initialized: bool,
    vault_has_passphrase: bool,
    cached_image: Option<(ClipItemId, Arc<Image>)>,
    copied_feedback: Option<(String, Instant)>,
}

impl Focusable for HistoryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl HistoryPanel {
    /// Constructs a new panel bound to shared runtime state and window controls.
    pub fn new(
        runtime: Rc<AppRuntime>,
        on_request: Rc<dyn Fn(PanelRequest)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let cfg = runtime.snapshot_config();
        let theme_id = resolve_theme_id(&cfg, window.appearance());
        let palette = ThemePalette::from_theme_id(theme_id);
        sync_component_theme(&palette, Some(window), cx);

        let is_slim = cfg.is_compact_mode;

        let focus_handle = cx.focus_handle();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if is_slim {
                t!("search.placeholder_slim").to_string()
            } else {
                t!("search.placeholder").to_string()
            })
        });
        let alias_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("preview.alias_placeholder").to_string())
        });
        let group_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("输入新分组名称，回车新建并归类..."));
        let content_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("编辑剪贴板文本内容..."));

        let vault_ui = VaultUiState::new(window, cx, |panel, window, cx| {
            panel.submit_vault_unlock_passphrase(window, cx);
        });

        let search_sub = cx.subscribe_in(&search_input, window, |this, state, ev, _window, cx| {
            if matches!(ev, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                if text != this.search_query {
                    this.search_query = text;
                    this.active_index = None;
                    this.pointer_hovered_index = None;
                    this.refresh_items(cx);
                }
            }
        });

        let alias_sub = cx.subscribe_in(&alias_input, window, |this, _state, ev, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.commit_alias_edit(window, cx);
            }
        });

        let group_sub = cx.subscribe_in(&group_input, window, |this, _state, ev, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.commit_group_create(window, cx);
            }
        });

        let search_focus = search_input.read(cx).focus_handle(cx);
        window.focus(&search_focus, cx);

        let mut panel = Self {
            runtime,
            on_request,
            focus_handle,
            search_input,
            alias_input,
            alias_editing_id: None,
            group_input,
            group_editing_id: None,
            content_input,
            content_editing_id: None,
            group_popover_open: false,
            image_actual_size: false,
            vault_ui,
            _subscriptions: vec![search_sub, alias_sub, group_sub],
            list_state: ListState::new(0, ListAlignment::Top, px(240.0)),
            palette,
            is_slim,
            window_pinned: false,
            active_tab: FilterTab::All,
            hover_tab_task: None,
            active_group: None,
            groups: Vec::new(),
            search_query: String::new(),
            rows: Rc::from([]),
            total_count: 0,
            active_index: None,
            pointer_hovered_index: None,
            context_menu: None,
            selected_item: None,
            prepared_preview: None,
            item_cache: HashMap::new(),
            selected_vault: None,
            vault_initialized: false,
            vault_has_passphrase: false,
            cached_image: None,
            copied_feedback: None,
        };
        panel.refresh_items(cx);
        panel
    }

    pub fn is_slim(&self) -> bool {
        self.is_slim
    }

    pub fn set_window_pinned(&mut self, pinned: bool, cx: &mut Context<'_, Self>) {
        if self.window_pinned != pinned {
            self.window_pinned = pinned;
            cx.notify();
        }
    }

    pub fn is_editing_inline(&self) -> bool {
        self.vault_ui.editor_open
            || self.alias_editing_id.is_some()
            || self.group_editing_id.is_some()
            || self.content_editing_id.is_some()
            || self.group_popover_open
            || self.context_menu.is_some()
    }

    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        let focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Reloads history items, groups, unlocked vault entries, and image thumbnails.
    pub fn refresh_items(&mut self, cx: &mut Context<'_, Self>) {
        let cfg = self.runtime.snapshot_config();
        let tab = self.active_tab;
        let query = self.search_query.clone();
        let group = self.active_group;

        self.groups = self.runtime.service.list_groups().unwrap_or_default();
        self.pointer_hovered_index = None;

        if tab == FilterTab::Accounts
            && !self
                .runtime
                .vault
                .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes)
        {
            let _ = self
                .runtime
                .vault
                .try_unlock_trusted(cfg.vault_unlock_policy);
        }

        let vault_unlocked = self
            .runtime
            .vault
            .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes);
        self.vault_initialized = self.runtime.vault.exists();
        self.vault_has_passphrase = self.runtime.vault.has_passphrase_envelope();

        let mut combined = Vec::new();

        if tab == FilterTab::Accounts {
            if vault_unlocked {
                let entries = self
                    .runtime
                    .vault
                    .list_entries(&query, 200)
                    .unwrap_or_default();
                self.total_count = entries.len() as u64;
                for entry in entries {
                    combined.push(RowEntry::Vault(entry));
                }
            } else {
                self.total_count = 0;
            }
        } else {
            if tab == FilterTab::All
                && vault_unlocked
                && !query.trim().is_empty()
                && let Ok(vault_hits) = self.runtime.vault.list_entries(query.trim(), 5)
            {
                for entry in vault_hits {
                    combined.push(RowEntry::Vault(entry));
                }
            }
            let fetch_limit = if query.trim().is_empty() { 60 } else { 120 };
            let clip_hits = self
                .runtime
                .service
                .query_summaries(tab, group, &query, fetch_limit)
                .unwrap_or_default();
            for clip in clip_hits {
                combined.push(RowEntry::Clip(clip));
            }
            self.total_count = self
                .runtime
                .service
                .total_count()
                .unwrap_or(combined.len() as u64);
        }

        self.rows = Rc::from(combined);
        self.selected_item = None;
        self.prepared_preview = None;
        self.list_state.reset(self.rows.len());
        self.warm_visible_thumbnails();

        if self.rows.is_empty() {
            self.active_index = None;
            self.pointer_hovered_index = None;
            self.context_menu = None;
            self.selected_item = None;
            self.prepared_preview = None;
            self.selected_vault = None;
            self.cached_image = None;
        } else {
            if let Some(menu) = &self.context_menu
                && menu.row_index >= self.rows.len()
            {
                self.context_menu = None;
            }
            if let Some(idx) = self.active_index
                && idx >= self.rows.len()
            {
                self.active_index = Some(self.rows.len() - 1);
            }
            self.load_preview_entry();
        }
        cx.notify();
    }

    /// Loads encrypted image thumbnails into the shared runtime cache and precomputes the first
    /// visible `PreparedPreview` entries so rapid pointer sweeps across list rows hit 100% in-memory cache.
    fn warm_visible_thumbnails(&mut self) {
        let mut loaded_thumbs = 0usize;
        let mut loaded_previews = 0usize;
        for row in self.rows.iter() {
            if loaded_thumbs >= WARM_THUMB_BATCH && loaded_previews >= 2 {
                break;
            }
            let RowEntry::Clip(summary) = row else {
                continue;
            };
            if summary.kind == ClipKind::Image && loaded_thumbs < WARM_THUMB_BATCH {
                loaded_thumbs += 1;
                let _ = self.runtime.warm_image_thumb_from_summary(summary);
            }
            if loaded_previews < 2 {
                loaded_previews += 1;
                let valid_cached = self.item_cache.get(&summary.id).is_some_and(|(it, _)| {
                    it.summary.updated_at_ms == summary.updated_at_ms
                        && it.summary.is_pinned == summary.is_pinned
                        && it.summary.group_id == summary.group_id
                        && it.summary.alias == summary.alias
                        && it.summary.ocr_text == summary.ocr_text
                });
                if !valid_cached && let Ok(item) = self.runtime.service.get_item(summary.id) {
                    let prep = PreparedPreview::from_item(&item);
                    if self.item_cache.len() >= 16 {
                        self.item_cache.clear();
                    }
                    self.item_cache.insert(summary.id, (item, prep));
                }
            }
        }
    }

    /// Updates the rendered theme palette smoothly during the 280ms theme cross-fade.
    pub fn set_animated_palette(&mut self, palette: ThemePalette, cx: &mut Context<'_, Self>) {
        self.palette = palette;
        cx.notify();
    }

    /// Releases cached preview items and decoded image handles while resting in capsule/tray standby.
    pub fn reclaim_idle_caches(&mut self) -> Option<Arc<Image>> {
        self.item_cache.clear();
        self.item_cache.shrink_to_fit();
        self.prepared_preview = None;
        self.selected_item = None;
        self.selected_vault = None;
        self.cached_image.take().map(|(_, img)| img)
    }

    /// Updates localized input placeholders after settings/locale changes without disrupting theme fades.
    pub fn refresh_locale_placeholders(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let placeholder = if self.is_slim {
            t!("search.placeholder_slim").to_string()
        } else {
            t!("search.placeholder").to_string()
        };
        self.search_input
            .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        self.alias_input.update(cx, |s, cx| {
            s.set_placeholder(t!("preview.alias_placeholder").to_string(), window, cx)
        });
        cx.notify();
    }

    /// Applies the visual slim/dual state driven by the shell's zero-reflow morph animation.
    pub fn apply_slim_mode(
        &mut self,
        target_slim: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.is_slim = target_slim;
        self.context_menu = None;
        self.runtime.set_compact_mode(target_slim);

        if target_slim && !FilterTab::ALL_SLIM.contains(&self.active_tab) {
            self.active_tab = FilterTab::All;
            self.refresh_items(cx);
        } else if !target_slim {
            self.load_preview_entry();
        }

        let placeholder = if target_slim {
            t!("search.placeholder_slim").to_string()
        } else {
            t!("search.placeholder").to_string()
        };
        self.search_input.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    /// Requests the shell controller to perform a smooth `328 <-> 680` drawer toggle.
    pub fn toggle_compact(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) {
        self.context_menu = None;
        (self.on_request)(PanelRequest::HoverImagePreview(None));
        (self.on_request)(PanelRequest::ToggleCompact);
    }

    fn active_preview_index(&self) -> Option<usize> {
        self.active_index
    }

    fn switch_filter_tab(&mut self, tab: FilterTab, cx: &mut Context<'_, Self>) {
        if self.active_tab == tab {
            return;
        }
        self.active_tab = tab;
        self.vault_ui.editor_open = false;
        self.alias_editing_id = None;
        self.group_editing_id = None;
        self.context_menu = None;
        self.active_index = None;
        self.pointer_hovered_index = None;
        (self.on_request)(PanelRequest::HoverImagePreview(None));
        self.refresh_items(cx);
    }

    fn load_preview_entry(&mut self) {
        let target = self
            .active_preview_index()
            .and_then(|idx| self.rows.get(idx));
        match target {
            Some(RowEntry::Clip(summary)) => {
                self.selected_vault = None;
                let id = summary.id;
                if self
                    .selected_item
                    .as_ref()
                    .is_some_and(|it| it.summary.id == id)
                    && self
                        .prepared_preview
                        .as_ref()
                        .is_some_and(|p| p.item_id == id)
                {
                    return;
                }
                let resolved = if let Some(cached) = self.item_cache.get(&id).filter(|(it, _)| {
                    it.summary.updated_at_ms == summary.updated_at_ms
                        && it.summary.is_pinned == summary.is_pinned
                        && it.summary.group_id == summary.group_id
                        && it.summary.alias == summary.alias
                        && it.summary.ocr_text == summary.ocr_text
                }) {
                    Some(cached.clone())
                } else {
                    self.runtime.service.get_item(id).ok().map(|item| {
                        let prep = PreparedPreview::from_item(&item);
                        if self.item_cache.len() >= 16 {
                            self.item_cache.clear();
                        }
                        self.item_cache.insert(id, (item.clone(), prep.clone()));
                        (item, prep)
                    })
                };
                match resolved {
                    Some((item, prep)) => {
                        let same_cached = self
                            .cached_image
                            .as_ref()
                            .is_some_and(|(cached_id, _)| *cached_id == id);
                        if !same_cached {
                            self.cached_image = if item.summary.kind == ClipKind::Image {
                                self.runtime
                                    .get_or_load_image_thumb(id)
                                    .map(|entry| (id, entry.image))
                            } else if item.summary.subtype == SmartSubtype::Base64 {
                                load_data_uri_image(item.full_text.trim())
                                    .map(|decoded| (id, decoded))
                            } else {
                                None
                            };
                        }
                        self.selected_item = Some(item);
                        self.prepared_preview = Some(prep);
                    }
                    None => {
                        self.selected_item = None;
                        self.prepared_preview = None;
                        self.cached_image = None;
                    }
                }
            }
            Some(RowEntry::Vault(entry)) => {
                self.selected_item = None;
                self.prepared_preview = None;
                self.cached_image = None;
                self.selected_vault = Some(entry.clone());
            }
            None => {
                self.selected_item = None;
                self.prepared_preview = None;
                self.selected_vault = None;
                self.cached_image = None;
            }
        }
    }

    fn select_index(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if self.rows.is_empty() {
            return;
        }
        let clamped = index.min(self.rows.len() - 1);
        if self.active_index != Some(clamped)
            || (self.selected_item.is_none() && self.selected_vault.is_none())
        {
            self.active_index = Some(clamped);
            self.alias_editing_id = None;
            self.context_menu = None;
            self.list_state.scroll_to_reveal_item(clamped);
            self.load_preview_entry();
            cx.notify();
        }
    }

    fn activate_selected(&mut self, shift: bool) {
        let Some(idx) = self.active_index else {
            return;
        };
        let Some(row) = self.rows.get(idx).cloned() else {
            return;
        };
        self.activate_row_entry(row, shift);
    }

    fn activate_row_entry(&self, row: RowEntry, shift: bool) {
        match row {
            RowEntry::Clip(summary) => {
                if shift && summary.kind == ClipKind::Image && !summary.ocr_text.trim().is_empty() {
                    let payload = clipboardx_model::ClipboardPayload::Text {
                        plain: summary.ocr_text.trim().to_owned(),
                    };
                    let _ =
                        PlatformBridge::write_clipboard(self.runtime.platform.as_ref(), &payload);
                    (self.on_request)(PanelRequest::AfterPaste);
                    return;
                }
                if let Err(err) = self.runtime.execute_paste(summary.id, shift, false) {
                    tracing::warn!(error = %err, "paste execution failed");
                }
                (self.on_request)(PanelRequest::AfterPaste);
            }
            RowEntry::Vault(entry) => {
                let action = if shift {
                    AutofillAction::PasswordOnly
                } else {
                    AutofillAction::FullSequence
                };
                self.runtime.execute_vault_autofill(entry, action);
                (self.on_request)(PanelRequest::AfterPaste);
            }
        }
    }

    fn activate_slot(&mut self, one_based_slot: usize) {
        let Some(idx) = one_based_slot.checked_sub(1) else {
            return;
        };
        let Some(row) = self.rows.get(idx).cloned() else {
            return;
        };
        self.activate_row_entry(row, false);
    }

    fn submit_vault_unlock_passphrase(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let pass = self.vault_ui.passphrase_input.read(cx).value().to_string();
        if pass.len() < 4 {
            self.vault_ui.unlock_key_mismatch = false;
            self.vault_ui.unlock_error = Some(t!("vault.passphrase_too_short").to_string());
            cx.notify();
            return;
        }

        let cfg = self.runtime.snapshot_config();
        let cache_trusted = cfg.vault_unlock_policy == VaultUnlockPolicy::TrustedDeviceNoPrompt;

        let res = if !self.runtime.vault.exists() || !self.runtime.vault.has_passphrase_envelope() {
            if self
                .runtime
                .vault
                .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes)
            {
                self.runtime
                    .vault
                    .attach_passphrase_while_unlocked(pass.as_bytes())
            } else {
                self.runtime
                    .vault
                    .init_with_passphrase(pass.as_bytes(), cache_trusted)
            }
        } else {
            self.runtime
                .vault
                .unlock_with_passphrase(pass.as_bytes(), cache_trusted)
        };

        match res {
            Ok(()) => {
                self.vault_ui.unlock_error = None;
                self.vault_ui.unlock_key_mismatch = false;
                self.vault_ui
                    .passphrase_input
                    .update(cx, |s, cx| s.set_value("", window, cx));
                let search_focus = self.search_input.read(cx).focus_handle(cx);
                window.focus(&search_focus, cx);
                self.refresh_items(cx);
            }
            Err(err) => {
                let msg = err.to_string();
                tracing::warn!(error = %msg, "vault passphrase unlock failed");
                let is_mismatch = msg.contains("verify vault.db key failed")
                    || msg.contains("unlock encrypted vault.db failed");
                self.vault_ui.unlock_key_mismatch = is_mismatch;
                self.vault_ui.unlock_error = Some(if is_mismatch {
                    t!("vault.key_mismatch").to_string()
                } else {
                    t!("vault.wrong_passphrase").to_string()
                });
                cx.notify();
            }
        }
    }

    fn save_vault_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let draft = self.vault_ui.read_draft(cx);
        if draft.title.trim().is_empty() || draft.username.trim().is_empty() {
            self.vault_ui.editor_error = Some("请输入名称与账号".to_owned());
            cx.notify();
            return;
        }

        let res = if let Some(id) = self.vault_ui.editing_id {
            self.runtime.vault.update_entry(id, &draft)
        } else {
            self.runtime.vault.create_entry(&draft).map(|_| ())
        };

        match res {
            Ok(()) => {
                self.vault_ui.editor_open = false;
                self.vault_ui.editing_id = None;
                self.vault_ui.editor_error = None;
                let search_focus = self.search_input.read(cx).focus_handle(cx);
                window.focus(&search_focus, cx);
                self.refresh_items(cx);
            }
            Err(err) => {
                self.vault_ui.editor_error = Some(err.to_string());
                cx.notify();
            }
        }
    }

    fn begin_alias_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.open_note_and_group_editor(true, window, cx);
    }

    fn begin_group_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.open_note_and_group_editor(false, window, cx);
    }

    /// Opens the unified note and group editor for the currently selected clip item.
    fn open_note_and_group_editor(
        &mut self,
        focus_alias: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.is_slim {
            self.toggle_compact(window, cx);
        }
        let Some(item) = &self.selected_item else {
            return;
        };
        self.group_popover_open = false;
        self.content_editing_id = None;
        self.alias_editing_id = Some(item.summary.id);
        self.group_editing_id = Some(item.summary.id);
        self.alias_input.update(cx, |s, cx| {
            s.set_value(item.summary.alias.clone(), window, cx)
        });
        self.group_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let focus = if focus_alias {
            self.alias_input.read(cx).focus_handle(cx)
        } else {
            self.group_input.read(cx).focus_handle(cx)
        };
        window.focus(&focus, cx);
        cx.notify();
    }

    fn close_note_and_group_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.alias_editing_id = None;
        self.group_editing_id = None;
        let search_focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&search_focus, cx);
        cx.notify();
    }

    fn commit_alias_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(id) = self.alias_editing_id.or(self.group_editing_id) else {
            return;
        };
        let text = self.alias_input.read(cx).value().to_string();
        if let Err(err) = self.runtime.service.update_alias(id, &text) {
            tracing::warn!(error = %err, "update alias failed");
        }
        let pending_group = self.group_input.read(cx).value().trim().to_owned();
        if !pending_group.is_empty()
            && let Ok(gid) = self.runtime.service.create_group(&pending_group)
        {
            let _ = self.runtime.service.assign_group(id, Some(gid));
        }
        self.alias_editing_id = None;
        self.group_editing_id = None;
        let search_focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&search_focus, cx);
        self.refresh_items(cx);
    }

    /// Opens the inline multi-line editor for editable text clip items.
    fn begin_content_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.is_slim {
            self.toggle_compact(window, cx);
        }
        let Some(item) = &self.selected_item else {
            return;
        };
        if !matches!(item.summary.kind, ClipKind::Text | ClipKind::RichText) {
            self.open_note_and_group_editor(true, window, cx);
            return;
        }
        self.group_popover_open = false;
        self.alias_editing_id = None;
        self.group_editing_id = None;
        self.content_editing_id = Some(item.summary.id);
        let initial = item.full_text.clone();
        self.content_input
            .update(cx, |s, cx| s.set_value(initial, window, cx));
        let focus = self.content_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn format_content_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let raw = self.content_input.read(cx).value().to_string();
        let next = serde_json::from_str::<serde_json::Value>(raw.trim())
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_else(|_| raw.replace("\r\n", "\n"));
        self.content_input
            .update(cx, |s, cx| s.set_value(next, window, cx));
        cx.notify();
    }

    fn trim_content_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let trimmed = self.content_input.read(cx).value().trim().to_owned();
        self.content_input
            .update(cx, |s, cx| s.set_value(trimmed, window, cx));
        cx.notify();
    }

    /// Saves the edited text content either in place or as a new entry, optionally pasting immediately.
    fn save_content_edit(
        &mut self,
        as_new: bool,
        paste_after: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(source_id) = self.content_editing_id.take() else {
            return;
        };
        let next_text = self.content_input.read(cx).value().to_string();
        if next_text.trim().is_empty() {
            self.close_content_editor(window, cx);
            return;
        }

        let target_id = if as_new {
            let (alias, gid) = self
                .selected_item
                .as_ref()
                .map(|it| (it.summary.alias.clone(), it.summary.group_id))
                .unwrap_or_default();
            self.runtime
                .service
                .create_text_item(&next_text, &alias, gid)
        } else {
            self.runtime.service.update_item_text(source_id, &next_text)
        };

        let search_focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&search_focus, cx);
        self.item_cache.clear();
        self.refresh_items(cx);

        if let Ok(saved_id) = target_id
            && paste_after
        {
            let _ = self.runtime.execute_paste(saved_id, false, false);
            (self.on_request)(PanelRequest::AfterPaste);
        }
    }

    fn close_content_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.content_editing_id = None;
        let search_focus = self.search_input.read(cx).focus_handle(cx);
        window.focus(&search_focus, cx);
        cx.notify();
    }

    fn toggle_pin_selected(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(idx) = self.active_index
            && let Some(RowEntry::Clip(summary)) = self.rows.get(idx)
        {
            if let Err(err) = self.runtime.service.toggle_pin(summary.id) {
                tracing::warn!(error = %err, "toggle pin failed");
            } else {
                self.refresh_items(cx);
            }
        }
    }

    fn delete_selected(&mut self, cx: &mut Context<'_, Self>) {
        let Some(idx) = self.active_index else {
            return;
        };
        match self.rows.get(idx) {
            Some(RowEntry::Clip(summary))
                if self.runtime.service.delete_item(summary.id).is_ok() =>
            {
                self.refresh_items(cx);
            }
            Some(RowEntry::Vault(entry)) if self.runtime.vault.delete_entry(entry.id).is_ok() => {
                self.refresh_items(cx);
            }
            _ => {}
        }
    }

    fn assign_item_group(
        &mut self,
        item_id: ClipItemId,
        group_id: Option<GroupId>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Err(err) = self.runtime.service.assign_group(item_id, group_id) {
            tracing::warn!(error = %err, "assign group failed");
        }
        self.item_cache.remove(&item_id);
        self.refresh_items(cx);
    }

    fn commit_group_create(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(item_id) = self.group_editing_id.or(self.alias_editing_id) else {
            return;
        };
        let name = self.group_input.read(cx).value().trim().to_owned();
        if !name.is_empty()
            && let Ok(gid) = self.runtime.service.create_group(&name)
        {
            self.group_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.assign_item_group(item_id, Some(gid), cx);
        }
    }

    fn delete_custom_group(&mut self, gid: GroupId, cx: &mut Context<'_, Self>) {
        if self.runtime.service.delete_group(gid).is_ok() {
            if self.active_group == Some(gid) {
                self.active_group = None;
            }
            self.item_cache.clear();
            self.refresh_items(cx);
        }
    }

    fn is_subform_focused(&self, window: &Window, cx: &App) -> bool {
        if self.vault_ui.editor_open
            || self.alias_editing_id.is_some()
            || self.group_editing_id.is_some()
            || self.content_editing_id.is_some()
        {
            return true;
        }
        let pass_focus = self.vault_ui.passphrase_input.read(cx).focus_handle(cx);
        pass_focus.is_focused(window)
    }

    fn handle_capture_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        let mods = ev.keystroke.modifiers;
        let secondary = mods.secondary(cx);

        if key == "escape" {
            if self.group_popover_open {
                self.group_popover_open = false;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.context_menu.is_some() {
                self.context_menu = None;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.vault_ui.editor_open {
                self.vault_ui.editor_open = false;
                let search_focus = self.search_input.read(cx).focus_handle(cx);
                window.focus(&search_focus, cx);
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.content_editing_id.is_some() {
                self.close_content_editor(window, cx);
                cx.stop_propagation();
                return;
            }
            if self.alias_editing_id.is_some() || self.group_editing_id.is_some() {
                self.close_note_and_group_editor(window, cx);
                cx.stop_propagation();
                return;
            }
        }

        if key == "enter" && secondary && self.content_editing_id.is_some() {
            self.save_content_edit(false, true, window, cx);
            cx.stop_propagation();
            return;
        }

        if self.is_subform_focused(window, cx) && !secondary {
            return;
        }

        match key {
            "up" => {
                if !self.rows.is_empty() {
                    let next = match self.active_index {
                        Some(cur) => cur.saturating_sub(1),
                        None => self.rows.len() - 1,
                    };
                    self.select_index(next, cx);
                }
                cx.stop_propagation();
            }
            "down" => {
                if !self.rows.is_empty() {
                    let next = match self.active_index {
                        Some(cur) => (cur + 1).min(self.rows.len() - 1),
                        None => 0,
                    };
                    self.select_index(next, cx);
                }
                cx.stop_propagation();
            }
            "enter" => {
                self.activate_selected(mods.shift);
                cx.stop_propagation();
            }
            "escape" => {
                if self.active_group.is_some() {
                    self.active_group = None;
                    self.refresh_items(cx);
                } else if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.search_input
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    self.refresh_items(cx);
                } else {
                    (self.on_request)(PanelRequest::HideWindow);
                }
                cx.stop_propagation();
            }
            "\\" if secondary => {
                self.toggle_compact(window, cx);
                cx.stop_propagation();
            }
            "p" if secondary => {
                self.toggle_pin_selected(cx);
                cx.stop_propagation();
            }
            "e" if secondary => {
                self.begin_content_edit(window, cx);
                cx.stop_propagation();
            }
            "g" if secondary => {
                self.begin_group_edit(window, cx);
                cx.stop_propagation();
            }
            "n" if secondary => {
                self.active_tab = FilterTab::Accounts;
                let cfg = self.runtime.snapshot_config();
                if self
                    .runtime
                    .vault
                    .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes)
                {
                    if self.is_slim {
                        self.toggle_compact(window, cx);
                    }
                    self.vault_ui.open_new(window, cx);
                }
                self.refresh_items(cx);
                cx.stop_propagation();
            }
            "delete" if secondary => {
                self.delete_selected(cx);
                cx.stop_propagation();
            }
            "f" if secondary => {
                let focus = self.search_input.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
                cx.stop_propagation();
            }
            _ if secondary && key.len() == 1 => {
                if let Some(digit) = key.chars().next().and_then(|c| c.to_digit(10))
                    && (1..=9).contains(&digit)
                {
                    self.activate_slot(digit as usize);
                    cx.stop_propagation();
                }
            }
            _ => {}
        }
    }

    fn current_copied_label(&mut self) -> Option<String> {
        if let Some((label, at)) = &self.copied_feedback {
            if at.elapsed() < Duration::from_secs(2) {
                return Some(label.clone());
            }
            self.copied_feedback = None;
        }
        None
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let palette = self.palette.clone();
        let cfg = self.runtime.snapshot_config();
        let slim_visual = self.is_slim;
        let is_accounts = self.active_tab == FilterTab::Accounts;

        let copied_badge = self.current_copied_label();
        let is_paused = self.runtime.is_paused();
        let vault_unlocked = self
            .runtime
            .vault
            .is_unlocked(cfg.vault_unlock_policy, cfg.vault_timeout_minutes);

        // Unified 6-tab filter strip for both dual-pane and slim modes.
        let tabs = FilterTab::ALL_SLIM;

        let show_right = !self.is_slim;
        let body_gap = if show_right { 10.0 } else { 0.0 };

        let count_text = if is_accounts {
            t!("vault.count_entries", count = self.total_count).to_string()
        } else {
            t!("search.count_items", count = self.total_count).to_string()
        };

        let cached_image = self.selected_item.as_ref().and_then(|item| {
            self.cached_image
                .as_ref()
                .filter(|(cached_id, _)| *cached_id == item.summary.id)
                .map(|(_, img)| Arc::clone(img))
        });

        let active_index = self.active_index;
        let rows_snapshot = Rc::clone(&self.rows);
        let runtime_for_list = Rc::clone(&self.runtime);
        let groups_for_list = self.groups.clone();
        let list_palette = palette.clone();
        let list_weak = cx.weak_entity();
        let window_pinned = self.window_pinned;

        let right_pane = self.build_right_pane(
            vault_unlocked,
            &cfg,
            cached_image,
            copied_badge.as_deref(),
            &palette,
            cx,
        );

        let context_overlay = self.render_context_menu_overlay(slim_visual, &palette, cx);
        let group_popover_overlay = self.render_group_popover_overlay(slim_visual, &palette, cx);

        let footer_hint = if self.active_group.is_some() {
            "按分组筛选中 · Esc 清除筛选".to_owned()
        } else if is_accounts {
            t!("footer.vault_hint").to_string()
        } else if slim_visual {
            t!("footer.slim_hint").to_string()
        } else {
            t!("footer.dual_hint").to_string()
        };

        let status_dot_color = if is_paused || (is_accounts && !vault_unlocked) {
            palette.accent_amber
        } else {
            palette.accent_green
        };
        let status_label = if is_paused {
            t!("footer.paused").to_string()
        } else if is_accounts {
            t!("footer.vault_encrypted").to_string()
        } else if slim_visual {
            t!("footer.docked_right").to_string()
        } else {
            t!("footer.encrypted_local").to_string()
        };

        div()
            .id("clipboardx-history-root")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::handle_capture_key_down))
            .relative()
            .size_full()
            .font_family(ui_font_family())
            .text_color(palette.text_primary)
            .flex()
            .flex_col()
            .overflow_hidden()
            // 1. Topbar (full-width search input + borderless icon actions)
            .child(
                div()
                    .w_full()
                    .pt(px(10.0))
                    .pb(px(6.0))
                    .pl(px(if slim_visual { 10.0 } else { 14.0 }))
                    .pr(px(if slim_visual { 10.0 } else { 12.0 }))
                    .flex()
                    .items_center()
                    .gap(px(if slim_visual { 6.0 } else { 8.0 }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(26.0))
                            .rounded(px(8.0))
                            .bg(palette.bg_surface)
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(app_icon(
                                AppIcon::Search,
                                if slim_visual { 12.0 } else { 13.0 },
                                palette.text_muted,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.5))
                                    .line_height(px(15.0))
                                    .child(input_shield(
                                        "history-search-shield",
                                        Input::new(&self.search_input)
                                            .appearance(false)
                                            .bordered(false)
                                            .focus_bordered(false)
                                            .cleanable(true)
                                            .text_size(px(11.5))
                                            .into_any_element(),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(if slim_visual { 2.0 } else { 4.0 }))
                            .flex_shrink_0()
                            .when(is_accounts, |el| {
                                el.child(clean_topbar_icon_btn(
                                    "btn-new-account",
                                    AppIcon::Plus,
                                    palette.accent_blue,
                                    Some(palette.accent_blue_soft),
                                    &palette,
                                    slim_visual,
                                    cx.listener(|this, _, window, cx| {
                                        if this.is_slim {
                                            this.toggle_compact(window, cx);
                                        }
                                        if this.runtime.vault.is_unlocked(
                                            this.runtime.snapshot_config().vault_unlock_policy,
                                            this.runtime.snapshot_config().vault_timeout_minutes,
                                        ) {
                                            this.vault_ui.open_new(window, cx);
                                            cx.notify();
                                        } else {
                                            let focus = this
                                                .vault_ui
                                                .passphrase_input
                                                .read(cx)
                                                .focus_handle(cx);
                                            window.focus(&focus, cx);
                                        }
                                    }),
                                ))
                            })
                            .child(pin_window_topbar_btn(
                                "btn-pin-window",
                                window_pinned,
                                &palette,
                                slim_visual,
                                cx.listener(|this, _, _, _| {
                                    (this.on_request)(PanelRequest::ToggleWindowPin);
                                }),
                            ))
                            .child(clean_topbar_icon_btn(
                                "btn-compact",
                                if self.is_slim {
                                    AppIcon::ExpandSidebar
                                } else {
                                    AppIcon::CollapseSidebar
                                },
                                palette.text_secondary,
                                None,
                                &palette,
                                slim_visual,
                                cx.listener(|this, _, window, cx| {
                                    this.toggle_compact(window, cx);
                                }),
                            ))
                            .when(!(slim_visual && is_accounts), |el| {
                                el.child(clean_topbar_icon_btn(
                                    "btn-settings",
                                    AppIcon::Settings,
                                    palette.text_secondary,
                                    None,
                                    &palette,
                                    slim_visual,
                                    cx.listener(|this, _, _, _| {
                                        (this.on_request)(PanelRequest::OpenSettings);
                                    }),
                                ))
                            })
                            .child(clean_topbar_icon_btn(
                                "btn-close",
                                AppIcon::Close,
                                palette.text_secondary,
                                None,
                                &palette,
                                slim_visual,
                                cx.listener(|this, _, _, _| {
                                    (this.on_request)(PanelRequest::HideWindow);
                                }),
                            )),
                    ),
            )
            // 2. 6-Tab Filter Strip
            .child(
                div()
                    .w_full()
                    .pt(px(2.0))
                    .pb(px(if slim_visual { 6.0 } else { 8.0 }))
                    .px(px(if slim_visual { 10.0 } else { 14.0 }))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(if slim_visual { 2.0 } else { 4.0 }))
                            .children(tabs.iter().copied().map(|tab| {
                                let active = self.active_tab == tab;
                                let label = t!(tab.i18n_key()).to_string();

                                div()
                                    .id(SharedString::from(format!("tab-{}", tab.i18n_key())))
                                    .cursor_pointer()
                                    .px(px(if slim_visual { 8.0 } else { 9.0 }))
                                    .py(px(3.0))
                                    .rounded(px(7.0))
                                    .bg(if active {
                                        palette.bg_selected
                                    } else {
                                        gpui_kit::transparent_black()
                                    })
                                    .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                                    .flex()
                                    .items_center()
                                    .text_size(px(11.0))
                                    .line_height(px(14.0))
                                    .font_weight(if active {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if active {
                                        palette.accent_blue
                                    } else {
                                        palette.text_muted
                                    })
                                    .on_hover(cx.listener(move |this, entered: &bool, _, cx| {
                                        if !*entered {
                                            this.hover_tab_task = None;
                                            return;
                                        }
                                        if !is_blind_dragging()
                                            && this.active_tab != tab
                                            && !this.is_editing_inline()
                                        {
                                            this.hover_tab_task =
                                                Some(cx.spawn(async move |weak, cx| {
                                                    cx.background_executor()
                                                        .timer(Duration::from_millis(
                                                            TAB_HOVER_DWELL_MS,
                                                        ))
                                                        .await;
                                                    let _ = weak.update(cx, |panel, cx| {
                                                        if !is_blind_dragging()
                                                            && !panel.is_editing_inline()
                                                        {
                                                            panel.switch_filter_tab(tab, cx);
                                                        }
                                                    });
                                                }));
                                        }
                                    }))
                                    .on_mouse_down(MouseButton::Left, |_, _, _| {
                                        reset_blind_drag_click_guard();
                                    })
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            if !is_blind_dragging() && !was_blind_drag_consumed() {
                                                this.hover_tab_task = None;
                                                this.switch_filter_tab(tab, cx);
                                            }
                                        }),
                                    )
                                    .child(label)
                            })),
                    )
                    .child(div().id("filter-drag-spacer").flex_1().h_full())
                    .when(!slim_visual, |el| {
                        let active_group_name = self.active_group.and_then(|gid| {
                            self.groups
                                .iter()
                                .find(|g| g.id == gid)
                                .map(|g| g.name.clone())
                        });
                        let group_btn_label = match &active_group_name {
                            Some(name) => format!("分组 · {name} ▾"),
                            None => "分组 ▾".to_owned(),
                        };
                        let group_active = active_group_name.is_some() || self.group_popover_open;

                        el.child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .when(!is_accounts, |row| {
                                    row.child(
                                        div()
                                            .id("filter-group-dropdown-btn")
                                            .cursor_pointer()
                                            .px(px(8.0))
                                            .py(px(2.5))
                                            .rounded(px(6.0))
                                            .bg(if group_active {
                                                palette.accent_blue_soft
                                            } else {
                                                gpui_kit::transparent_black()
                                            })
                                            .hover(|s| {
                                                if group_active {
                                                    s
                                                } else {
                                                    s.bg(palette.bg_hover)
                                                }
                                            })
                                            .text_size(px(10.0))
                                            .line_height(px(13.0))
                                            .font_weight(if group_active {
                                                FontWeight::MEDIUM
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .text_color(if group_active {
                                                palette.accent_blue
                                            } else {
                                                palette.text_muted
                                            })
                                            .on_mouse_down(MouseButton::Left, |_, _, _| {
                                                reset_blind_drag_click_guard();
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if !was_blind_drag_consumed() {
                                                    this.context_menu = None;
                                                    this.group_popover_open =
                                                        !this.group_popover_open;
                                                    cx.notify();
                                                }
                                            }))
                                            .child(group_btn_label),
                                    )
                                })
                                .child(
                                    div()
                                        .id("filter-count-drag")
                                        .text_size(px(10.0))
                                        .line_height(px(13.0))
                                        .text_color(palette.text_muted)
                                        .child(count_text),
                                )
                                .when(is_accounts, |row| {
                                    row.child(
                                        div()
                                            .id("filter-new-account-pill")
                                            .cursor_pointer()
                                            .px(px(8.0))
                                            .py(px(3.0))
                                            .rounded(px(6.0))
                                            .bg(palette.accent_blue_soft)
                                            .hover(|s| s.bg(palette.bg_hover))
                                            .text_size(px(10.0))
                                            .line_height(px(13.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(palette.accent_blue)
                                            .on_mouse_down(MouseButton::Left, |_, _, _| {
                                                reset_blind_drag_click_guard();
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if was_blind_drag_consumed() {
                                                    return;
                                                }
                                                let cfg = this.runtime.snapshot_config();
                                                if this.runtime.vault.is_unlocked(
                                                    cfg.vault_unlock_policy,
                                                    cfg.vault_timeout_minutes,
                                                ) {
                                                    this.vault_ui.open_new(window, cx);
                                                    cx.notify();
                                                } else {
                                                    let focus = this
                                                        .vault_ui
                                                        .passphrase_input
                                                        .read(cx)
                                                        .focus_handle(cx);
                                                    window.focus(&focus, cx);
                                                }
                                            }))
                                            .child("+ 新建账号"),
                                    )
                                }),
                        )
                    }),
            )
            // 3. History List + Right Preview Pane
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .pb(px(6.0))
                    .px(px(if slim_visual { 8.0 } else { 10.0 }))
                    .flex()
                    .gap(px(body_gap))
                    .child(
                        div()
                            .when(show_right, |el| {
                                el.w(px(DUAL_LEFT_LIST_WIDTH)).flex_shrink_0()
                            })
                            .when(!show_right, |el| el.flex_1().min_w_0())
                            .h_full()
                            .overflow_hidden()
                            .child(
                                list(self.list_state.clone(), move |ix, _window, _cx| {
                                    let Some(entry) = rows_snapshot.get(ix) else {
                                        return div().into_any_element();
                                    };
                                    let is_active = active_index == Some(ix);

                                    let weak_act = list_weak.clone();
                                    let on_activate: RowIndexCb =
                                        Rc::new(move |idx, cx: &mut App| {
                                            let _ = weak_act.update(cx, |panel, cx| {
                                                panel.select_index(idx, cx);
                                                panel.activate_selected(false);
                                            });
                                        });

                                    let weak_hov = list_weak.clone();
                                    let on_hover: RowHoverCb =
                                        Rc::new(move |idx, entered, cx: &mut App| {
                                            let _ = weak_hov.update(cx, |panel, cx| {
                                                if panel.context_menu.is_some() {
                                                    return;
                                                }
                                                let (next_active, next_pointer) = next_hover_state(
                                                    panel.active_index,
                                                    panel.pointer_hovered_index,
                                                    idx,
                                                    entered,
                                                );
                                                let active_changed =
                                                    panel.active_index != next_active;
                                                let pointer_changed =
                                                    panel.pointer_hovered_index != next_pointer;
                                                if !active_changed && !pointer_changed {
                                                    return;
                                                }
                                                panel.active_index = next_active;
                                                panel.pointer_hovered_index = next_pointer;

                                                if !panel.is_slim && !panel.is_editing_inline() {
                                                    if active_changed {
                                                        panel.load_preview_entry();
                                                    }
                                                } else if panel.is_slim {
                                                    let hover_img_id =
                                                        next_pointer.and_then(|ix| {
                                                            match panel.rows.get(ix) {
                                                                Some(RowEntry::Clip(s))
                                                                    if s.kind
                                                                        == ClipKind::Image =>
                                                                {
                                                                    Some(s.id)
                                                                }
                                                                _ => None,
                                                            }
                                                        });
                                                    (panel.on_request)(
                                                        PanelRequest::HoverImagePreview(
                                                            hover_img_id,
                                                        ),
                                                    );
                                                }
                                                cx.notify();
                                            });
                                        });

                                    let weak_ctx = list_weak.clone();
                                    let on_context_menu: RowContextMenuCb =
                                        Rc::new(move |idx, pos, cx: &mut App| {
                                            let _ = weak_ctx.update(cx, |panel, cx| {
                                                (panel.on_request)(
                                                    PanelRequest::HoverImagePreview(None),
                                                );
                                                panel.select_index(idx, cx);
                                                panel.pointer_hovered_index = None;
                                                panel.context_menu = Some(ContextMenuState {
                                                    row_index: idx,
                                                    position: pos,
                                                });
                                                cx.notify();
                                            });
                                        });

                                    match entry {
                                        RowEntry::Clip(summary) => {
                                            let thumb = if summary.kind == ClipKind::Image {
                                                runtime_for_list
                                                    .cached_image_thumb(summary.id)
                                                    .map(|cached| cached.image)
                                            } else {
                                                None
                                            };
                                            let group_label = summary.group_id.and_then(|gid| {
                                                groups_for_list
                                                    .iter()
                                                    .find(|g| g.id == gid)
                                                    .map(|g| g.name.as_str())
                                            });
                                            let actions = ClipRowActions {
                                                on_activate,
                                                on_hover,
                                                on_context_menu,
                                            };
                                            render_clip_row(
                                                ClipRowProps {
                                                    ix,
                                                    summary,
                                                    thumb,
                                                    group_label,
                                                    is_active,
                                                    slim_visual,
                                                },
                                                &list_palette,
                                                actions,
                                            )
                                        }
                                        RowEntry::Vault(v_entry) => {
                                            let weak_del = list_weak.clone();
                                            render_vault_list_row(
                                                ix,
                                                v_entry,
                                                is_active,
                                                &list_palette,
                                                VaultRowActions {
                                                    on_activate,
                                                    on_hover,
                                                    on_context_menu,
                                                    on_quick_delete: Rc::new(move |id, cx| {
                                                        let _ = weak_del.update(cx, |panel, cx| {
                                                            let _ = panel
                                                                .runtime
                                                                .vault
                                                                .delete_entry(id);
                                                            panel.refresh_items(cx);
                                                        });
                                                    }),
                                                },
                                            )
                                        }
                                    }
                                })
                                .size_full(),
                            ),
                    )
                    .when(show_right, |el| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .overflow_hidden()
                                .child(right_pane),
                        )
                    }),
            )
            // 4. Compact Status Footer
            .child(
                div()
                    .id("history-footer-drag")
                    .w_full()
                    .pt(px(2.0))
                    .pb(px(8.0))
                    .px(px(if slim_visual { 12.0 } else { 14.0 }))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_muted)
                            .truncate()
                            .child(footer_hint),
                    )
                    .child(
                        div()
                            .id("footer-status-pill")
                            .when(is_paused, |el| el.cursor_pointer())
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_paused && !was_blind_drag_consumed() {
                                    let _ = this.runtime.toggle_pause();
                                    cx.notify();
                                }
                            }))
                            .child(div().size(px(6.0)).rounded(px(999.0)).bg(status_dot_color))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .text_color(palette.text_muted)
                                    .child(status_label),
                            ),
                    ),
            )
            .when_some(group_popover_overlay, |el, overlay| el.child(overlay))
            .when_some(context_overlay, |el, overlay| el.child(overlay))
    }
}

impl HistoryPanel {
    /// Renders the floating group filter popover anchored under the filter strip.
    fn render_group_popover_overlay(
        &self,
        slim_visual: bool,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        if !self.group_popover_open || slim_visual || self.active_tab == FilterTab::Accounts {
            return None;
        }

        let dot_colors = [
            palette.accent_blue,
            palette.accent_purple,
            palette.accent_green,
            palette.accent_amber,
            palette.text_muted,
        ];

        let mut card = div()
            .id("group-popover-card")
            .occlude()
            .absolute()
            .right(px(28.0))
            .top(px(68.0))
            .w(px(172.0))
            .rounded(px(10.0))
            .bg(palette.bg_surface)
            .shadow(palette.window_shadows())
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child({
                let active = self.active_group.is_none();
                div()
                    .id("group-pop-all")
                    .cursor_pointer()
                    .w_full()
                    .rounded(px(7.0))
                    .bg(if active {
                        palette.accent_blue_soft
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                    .px(px(8.0))
                    .py(px(5.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active_group = None;
                        this.group_popover_open = false;
                        this.refresh_items(cx);
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if active {
                                palette.accent_blue
                            } else {
                                palette.text_primary
                            })
                            .child(t!("groups.all").to_string()),
                    )
            });

        for (idx, g) in self.groups.iter().enumerate() {
            let gid = g.id;
            let active = self.active_group == Some(gid);
            let dot_color = dot_colors[idx % dot_colors.len()];
            card = card.child(
                div()
                    .id(SharedString::from(format!("group-pop-{}", gid.0)))
                    .cursor_pointer()
                    .w_full()
                    .rounded(px(7.0))
                    .bg(if active {
                        palette.accent_blue_soft
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                    .px(px(8.0))
                    .py(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_group = Some(gid);
                        this.group_popover_open = false;
                        this.refresh_items(cx);
                    }))
                    .child(div().size(px(6.0)).rounded(px(999.0)).bg(dot_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.0))
                            .line_height(px(14.0))
                            .font_weight(if active {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if active {
                                palette.accent_blue
                            } else {
                                palette.text_primary
                            })
                            .truncate()
                            .child(g.name.clone()),
                    ),
            );
        }

        if self.groups.is_empty() {
            card = card.child(
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(px(10.0))
                    .text_color(palette.text_muted)
                    .child("暂无自定义分组"),
            );
        }

        card = card.child(
            div()
                .id("group-pop-new")
                .cursor_pointer()
                .mt(px(3.0))
                .w_full()
                .rounded(px(7.0))
                .bg(palette.bg_elevated)
                .hover(|s| s.bg(palette.bg_hover))
                .px(px(8.0))
                .py(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .line_height(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.accent_blue)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.group_popover_open = false;
                    this.open_note_and_group_editor(false, window, cx);
                }))
                .child("+ 新建分组"),
        );

        Some(
            div()
                .id("group-popover-backdrop")
                .occlude()
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.group_popover_open = false;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.group_popover_open = false;
                        cx.notify();
                    }),
                )
                .child(card)
                .into_any_element(),
        )
    }

    /// Renders a borderless elevated context menu on right-click with automatic viewport clamping.
    fn render_context_menu_overlay(
        &self,
        slim_visual: bool,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let menu_state = self.context_menu.as_ref()?;
        let row = self.rows.get(menu_state.row_index)?.clone();

        let win_w = if slim_visual {
            LayoutSpec::WINDOW_SLIM_WIDTH
        } else {
            LayoutSpec::WINDOW_DUAL_WIDTH
        };
        let win_h = LayoutSpec::WINDOW_HEIGHT;
        let menu_w = 156.0_f32;
        let est_h = match &row {
            RowEntry::Clip(s) if s.kind == ClipKind::Image && !s.ocr_text.trim().is_empty() => {
                202.0
            }
            RowEntry::Clip(_) => 180.0,
            RowEntry::Vault(_) => 138.0,
        };

        // Convert window-space click coordinates into the visual shell coordinate space
        // by subtracting the outer transparent shadow margin so the menu aligns under the cursor.
        let shadow_margin = crate::shell::WINDOW_SHADOW_MARGIN;
        let raw_x: f32 = f32::from(menu_state.position.x) - shadow_margin;
        let raw_y: f32 = f32::from(menu_state.position.y) - shadow_margin;
        let left = raw_x.clamp(8.0, (win_w - menu_w - 8.0).max(8.0));
        let top = raw_y.clamp(8.0, (win_h - est_h - 8.0).max(8.0));

        let mut card = div()
            .id("context-menu-card")
            .occlude()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(menu_w))
            .rounded(px(9.0))
            .bg(palette.bg_surface)
            .shadow(palette.window_shadows())
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            });

        match row {
            RowEntry::Clip(summary) => {
                let id = summary.id;
                let is_pinned = summary.is_pinned;
                let ocr_text = summary.ocr_text.trim().to_owned();
                card = card
                    .child(ctx_menu_item(
                        "ctx-paste",
                        IconName::CornerDownLeft,
                        &t!("context_menu.paste"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, _| {
                            this.context_menu = None;
                            let _ = this.runtime.execute_paste(id, false, false);
                            (this.on_request)(PanelRequest::AfterPaste);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-paste-plain",
                        IconName::FileText,
                        &t!("context_menu.paste_plain"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, _| {
                            this.context_menu = None;
                            let _ = this.runtime.execute_paste(id, true, false);
                            (this.on_request)(PanelRequest::AfterPaste);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-copy-only",
                        IconName::Copy,
                        &t!("context_menu.copy_only"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            let _ = this.runtime.execute_paste(id, false, true);
                            cx.notify();
                        }),
                    ))
                    .when(!ocr_text.is_empty(), |el| {
                        let ocr_copy = ocr_text.clone();
                        el.child(ctx_menu_item(
                            "ctx-copy-ocr",
                            IconName::ScanText,
                            &t!("context_menu.copy_ocr"),
                            palette.accent_purple,
                            palette,
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                let payload = clipboardx_model::ClipboardPayload::Text {
                                    plain: ocr_copy.clone(),
                                };
                                let _ = PlatformBridge::write_clipboard(
                                    this.runtime.platform.as_ref(),
                                    &payload,
                                );
                                cx.notify();
                            }),
                        ))
                    })
                    .child(ctx_menu_item(
                        "ctx-pin",
                        IconName::Pin,
                        &if is_pinned {
                            t!("context_menu.unpin")
                        } else {
                            t!("context_menu.pin")
                        },
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            let _ = this.runtime.service.toggle_pin(id);
                            this.refresh_items(cx);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-alias",
                        IconName::Pencil,
                        &t!("context_menu.edit_alias"),
                        palette.text_primary,
                        palette,
                        cx.listener(|this, _, win, cx| {
                            this.context_menu = None;
                            this.begin_alias_edit(win, cx);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-group",
                        IconName::Folder,
                        &t!("context_menu.assign_group"),
                        palette.text_primary,
                        palette,
                        cx.listener(|this, _, win, cx| {
                            this.context_menu = None;
                            this.begin_group_edit(win, cx);
                        }),
                    ))
                    .when(
                        matches!(summary.kind, ClipKind::Text | ClipKind::RichText),
                        |el| {
                            el.child(ctx_menu_item(
                                "ctx-edit-content",
                                IconName::FileCode,
                                "编辑内容",
                                palette.text_primary,
                                palette,
                                cx.listener(|this, _, win, cx| {
                                    this.context_menu = None;
                                    this.begin_content_edit(win, cx);
                                }),
                            ))
                        },
                    )
                    .child(ctx_menu_item(
                        "ctx-del",
                        IconName::Trash,
                        &t!("context_menu.delete"),
                        palette.accent_red,
                        palette,
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            let _ = this.runtime.service.delete_item(id);
                            this.refresh_items(cx);
                        }),
                    ));
            }
            RowEntry::Vault(entry) => {
                let e_full = entry.clone();
                let e_user = entry.clone();
                let e_pass = entry.clone();
                let e_edit = entry.clone();
                let eid = entry.id;
                card = card
                    .child(ctx_menu_item(
                        "ctx-v-autofill",
                        IconName::Zap,
                        &t!("context_menu.autofill"),
                        palette.accent_blue,
                        palette,
                        cx.listener(move |this, _, _, _| {
                            this.context_menu = None;
                            this.runtime.execute_vault_autofill(
                                e_full.clone(),
                                AutofillAction::FullSequence,
                            );
                            (this.on_request)(PanelRequest::AfterPaste);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-v-user",
                        IconName::User,
                        &t!("context_menu.paste_user"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, _| {
                            this.context_menu = None;
                            this.runtime.execute_vault_autofill(
                                e_user.clone(),
                                AutofillAction::UsernameOnly,
                            );
                            (this.on_request)(PanelRequest::AfterPaste);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-v-pass",
                        IconName::KeyRound,
                        &t!("context_menu.paste_pass"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, _, _| {
                            this.context_menu = None;
                            this.runtime.execute_vault_autofill(
                                e_pass.clone(),
                                AutofillAction::PasswordOnly,
                            );
                            (this.on_request)(PanelRequest::AfterPaste);
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-v-edit",
                        IconName::Pencil,
                        &t!("context_menu.edit_vault"),
                        palette.text_primary,
                        palette,
                        cx.listener(move |this, _, win, cx| {
                            this.context_menu = None;
                            if this.is_slim {
                                this.toggle_compact(win, cx);
                            }
                            this.vault_ui.open_edit(&e_edit, win, cx);
                            cx.notify();
                        }),
                    ))
                    .child(ctx_menu_item(
                        "ctx-v-del",
                        IconName::Trash,
                        &t!("context_menu.delete"),
                        palette.accent_red,
                        palette,
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            let _ = this.runtime.vault.delete_entry(eid);
                            this.refresh_items(cx);
                        }),
                    ));
            }
        }

        Some(
            div()
                .id("context-menu-backdrop")
                .occlude()
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }),
                )
                .child(card)
                .into_any_element(),
        )
    }

    fn build_right_pane(
        &self,
        vault_unlocked: bool,
        cfg: &clipboardx_model::AppConfig,
        cached_image: Option<Arc<Image>>,
        copied_badge: Option<&str>,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        // Inline multi-line content editor for text/code clip items.
        if self.content_editing_id.is_some() {
            return self.render_content_editor_card(palette, cx);
        }

        // Unified note (alias) and group assignment card.
        if let Some(target_item_id) = self.alias_editing_id.or(self.group_editing_id) {
            return self.render_note_and_group_card(target_item_id, palette, cx);
        }

        let is_vault_context = self.active_tab == FilterTab::Accounts
            || self.vault_ui.editor_open
            || self.selected_vault.is_some();

        if is_vault_context {
            let weak_unlock = cx.weak_entity();
            let weak_trusted = cx.weak_entity();
            let weak_reset = cx.weak_entity();
            let weak_lock = cx.weak_entity();
            let weak_reveal = cx.weak_entity();
            let weak_submit = cx.weak_entity();
            let weak_edit = cx.weak_entity();
            let weak_del = cx.weak_entity();
            let weak_save = cx.weak_entity();
            let weak_cancel = cx.weak_entity();
            let weak_ed_sub = cx.weak_entity();
            let rt_autofill = Rc::clone(&self.runtime);
            let req_hide = Rc::clone(&self.on_request);

            let callbacks = VaultCallbacks {
                on_unlock_passphrase: Rc::new(move |window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_unlock.update(cx, |p, cx| {
                        p.submit_vault_unlock_passphrase(window, cx);
                    });
                }),
                on_unlock_trusted: Rc::new(move |window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_trusted.update(cx, |p, cx| {
                        p.runtime.update_config(|c| {
                            c.vault_unlock_policy = VaultUnlockPolicy::TrustedDeviceNoPrompt
                        });
                        match p
                            .runtime
                            .vault
                            .try_unlock_trusted(VaultUnlockPolicy::TrustedDeviceNoPrompt)
                        {
                            Ok(true) => {
                                p.vault_ui.unlock_error = None;
                                p.vault_ui.unlock_key_mismatch = false;
                                let sf = p.search_input.read(cx).focus_handle(cx);
                                window.focus(&sf, cx);
                                p.refresh_items(cx);
                            }
                            Ok(false) => {
                                p.vault_has_passphrase = p.runtime.vault.has_passphrase_envelope();
                                p.vault_ui.unlock_key_mismatch = !p.vault_has_passphrase;
                                p.vault_ui.unlock_error = Some(if p.vault_ui.unlock_key_mismatch {
                                    t!("vault.key_mismatch").to_string()
                                } else {
                                    t!("vault.unlock_hint").to_string()
                                });
                                cx.notify();
                            }
                            Err(err) => {
                                let msg = err.to_string();
                                tracing::warn!(error = %msg, "trusted vault unlock failed");
                                p.vault_ui.unlock_key_mismatch = true;
                                p.vault_ui.unlock_error =
                                    Some(t!("vault.key_mismatch").to_string());
                                cx.notify();
                            }
                        }
                    });
                }),
                on_reset_vault: Rc::new(move |window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_reset.update(cx, |p, cx| {
                        if let Err(err) = p.runtime.vault.reset_vault() {
                            tracing::warn!(error = %err, "vault reset failed");
                            p.vault_ui.unlock_error = Some(err.to_string());
                            cx.notify();
                            return;
                        }
                        p.vault_ui.unlock_error = None;
                        p.vault_ui.unlock_key_mismatch = false;
                        p.vault_ui
                            .passphrase_input
                            .update(cx, |s, cx| s.set_value("", window, cx));
                        p.refresh_items(cx);
                    });
                }),
                on_lock: Rc::new(move |cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_lock.update(cx, |p, cx| {
                        p.runtime.vault.lock();
                        p.refresh_items(cx);
                    });
                }),
                on_autofill: Rc::new(move |entry, action| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    rt_autofill.execute_vault_autofill(entry, action);
                    req_hide(PanelRequest::AfterPaste);
                }),
                on_toggle_reveal: Rc::new(move |cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_reveal.update(cx, |p, cx| {
                        p.vault_ui.reveal_password = !p.vault_ui.reveal_password;
                        cx.notify();
                    });
                }),
                on_toggle_entry_submit: Rc::new(move |id: VaultEntryId, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_submit.update(cx, |p, cx| {
                        let _ = p.runtime.vault.toggle_auto_submit(id);
                        p.refresh_items(cx);
                    });
                }),
                on_edit_entry: Rc::new(move |entry, window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_edit.update(cx, |p, cx| {
                        p.vault_ui.open_edit(&entry, window, cx);
                        cx.notify();
                    });
                }),
                on_delete_entry: Rc::new(move |id, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_del.update(cx, |p, cx| {
                        let _ = p.runtime.vault.delete_entry(id);
                        p.refresh_items(cx);
                    });
                }),
                on_save_editor: Rc::new(move |window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_save.update(cx, |p, cx| {
                        p.save_vault_editor(window, cx);
                    });
                }),
                on_cancel_editor: Rc::new(move |cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_cancel.update(cx, |p, cx| {
                        p.vault_ui.editor_open = false;
                        cx.notify();
                    });
                }),
                on_toggle_editor_submit: Rc::new(move |cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_ed_sub.update(cx, |p, cx| {
                        p.vault_ui.editor_auto_submit = !p.vault_ui.editor_auto_submit;
                        cx.notify();
                    });
                }),
                on_toggle_passphrase_mask: Rc::new({
                    let weak_pm = cx.weak_entity();
                    move |window, cx| {
                        if was_blind_drag_consumed() {
                            return;
                        }
                        let _ = weak_pm.update(cx, |p, cx| {
                            p.vault_ui.toggle_passphrase_mask(window, cx);
                            cx.notify();
                        });
                    }
                }),
                on_toggle_editor_pass_mask: Rc::new({
                    let weak_em = cx.weak_entity();
                    move |window, cx| {
                        if was_blind_drag_consumed() {
                            return;
                        }
                        let _ = weak_em.update(cx, |p, cx| {
                            p.vault_ui.toggle_editor_pass_mask(window, cx);
                            cx.notify();
                        });
                    }
                }),
                on_generate_password: Rc::new({
                    let weak_gen = cx.weak_entity();
                    move |window, cx| {
                        if was_blind_drag_consumed() {
                            return;
                        }
                        let _ = weak_gen.update(cx, |p, cx| {
                            p.vault_ui.fill_generated_password(window, cx);
                            cx.notify();
                        });
                    }
                }),
            };

            let editor_password_snapshot = self.vault_ui.pass_input.read(cx).value().to_string();
            return render_vault_right_pane(
                VaultPaneProps {
                    vault_unlocked,
                    vault_initialized: self.vault_initialized,
                    has_passphrase: self.vault_has_passphrase,
                    policy: cfg.vault_unlock_policy,
                    selected_entry: self.selected_vault.as_ref(),
                    editor_password_value: &editor_password_snapshot,
                },
                &self.vault_ui,
                palette,
                &callbacks,
            );
        }

        let runtime_copy = Rc::clone(&self.runtime);
        let runtime_snippet = Rc::clone(&self.runtime);
        let runtime_ocr = Rc::clone(&self.runtime);
        let runtime_pin = Rc::clone(&self.runtime);
        let runtime_del = Rc::clone(&self.runtime);
        let weak_alias = cx.weak_entity();
        let weak_group = cx.weak_entity();
        let weak_scale = cx.weak_entity();

        let group_name = self.selected_item.as_ref().and_then(|item| {
            item.summary.group_id.and_then(|gid| {
                self.groups
                    .iter()
                    .find(|g| g.id == gid)
                    .map(|g| g.name.as_str())
            })
        });

        let callbacks = PreviewCallbacks {
            on_copy: Rc::new(move |id, plain| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = runtime_copy.execute_paste(id, plain, true);
                runtime_copy.notify_ui(crate::state::UiEvent::HistoryUpdated);
            }),
            on_copy_snippet: Rc::new(move |text| {
                if was_blind_drag_consumed() {
                    return;
                }
                let payload = clipboardx_model::ClipboardPayload::Text { plain: text };
                let _ =
                    PlatformBridge::write_clipboard(runtime_snippet.platform.as_ref(), &payload);
                runtime_snippet.notify_ui(crate::state::UiEvent::HistoryUpdated);
            }),
            on_open_url: Rc::new(|url| {
                if was_blind_drag_consumed() {
                    return;
                }
                if let Err(err) = open_external_url(&url) {
                    tracing::warn!(error = %err, "open external URL failed");
                }
            }),
            on_toggle_image_scale: Rc::new(move |cx| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = weak_scale.update(cx, |p, cx| {
                    p.image_actual_size = !p.image_actual_size;
                    cx.notify();
                });
            }),
            on_run_ocr: Rc::new(move |id| {
                if was_blind_drag_consumed() {
                    return;
                }
                runtime_ocr.trigger_ocr_now(id);
            }),
            on_toggle_pin: Rc::new(move |id| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = runtime_pin.service.toggle_pin(id);
                runtime_pin.notify_ui(crate::state::UiEvent::HistoryUpdated);
            }),
            on_edit_alias: Rc::new(move |window, cx| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = weak_alias.update(cx, |p, cx| {
                    p.begin_alias_edit(window, cx);
                });
            }),
            on_edit_group: Rc::new(move |window, cx| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = weak_group.update(cx, |p, cx| {
                    p.begin_group_edit(window, cx);
                });
            }),
            on_edit_content: Rc::new({
                let weak_content = cx.weak_entity();
                move |window, cx| {
                    if was_blind_drag_consumed() {
                        return;
                    }
                    let _ = weak_content.update(cx, |p, cx| {
                        p.begin_content_edit(window, cx);
                    });
                }
            }),
            on_delete: Rc::new(move |id| {
                if was_blind_drag_consumed() {
                    return;
                }
                let _ = runtime_del.service.delete_item(id);
                runtime_del.notify_ui(crate::state::UiEvent::HistoryUpdated);
            }),
        };

        render_preview_card(
            PreviewRenderProps {
                item: self.selected_item.as_ref(),
                prepared: self.prepared_preview.as_ref(),
                has_items: !self.rows.is_empty(),
                image_preview: cached_image,
                image_actual_size: self.image_actual_size,
                group_name,
                copied_badge,
                native_ocr_tag: self.runtime.native_ocr_short_name(),
            },
            palette,
            &callbacks,
        )
    }

    /// Renders the unified note (alias) and group management card in the right pane.
    fn render_note_and_group_card(
        &self,
        target_item_id: ClipItemId,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let current_gid = self
            .selected_item
            .as_ref()
            .and_then(|it| it.summary.group_id);
        let (ctx_tag, ctx_bg, ctx_fg, ctx_preview) = self
            .selected_item
            .as_ref()
            .map(|it| {
                let (tag, bg, fg) = match it.summary.kind {
                    ClipKind::Image => ("IMG", palette.accent_purple_soft, palette.accent_purple),
                    ClipKind::Files => ("FILE", palette.bg_elevated, palette.text_secondary),
                    ClipKind::Text | ClipKind::RichText => match it.summary.subtype {
                        SmartSubtype::Json => {
                            ("JSON", palette.accent_green_soft, palette.accent_green)
                        }
                        SmartSubtype::Code => {
                            ("CODE", palette.accent_purple_soft, palette.accent_purple)
                        }
                        SmartSubtype::Url => ("URL", palette.accent_blue_soft, palette.accent_blue),
                        SmartSubtype::Jwt => {
                            ("JWT", palette.accent_amber_soft, palette.accent_amber)
                        }
                        SmartSubtype::Color => {
                            ("CLR", palette.accent_blue_soft, palette.accent_blue)
                        }
                        _ => ("TXT", palette.bg_elevated, palette.text_secondary),
                    },
                };
                (tag, bg, fg, it.summary.preview_text.clone())
            })
            .unwrap_or((
                "TXT",
                palette.bg_elevated,
                palette.text_secondary,
                String::new(),
            ));

        let alias_len = self.alias_input.read(cx).value().chars().count();
        let group_count = self.groups.len();

        let mut group_chips = div().w_full().flex().flex_wrap().gap(px(6.0)).child({
            let active = current_gid.is_none();
            div()
                .id("group-chip-none")
                .cursor_pointer()
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(7.0))
                .bg(if active {
                    palette.accent_blue_soft
                } else {
                    palette.bg_elevated
                })
                .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                .text_size(px(11.0))
                .line_height(px(14.0))
                .font_weight(if active {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if active {
                    palette.accent_blue
                } else {
                    palette.text_secondary
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !was_blind_drag_consumed() {
                        this.assign_item_group(target_item_id, None, cx);
                    }
                }))
                .child("未分组")
        });

        for g in &self.groups {
            let gid = g.id;
            let active = current_gid == Some(gid);
            group_chips = group_chips.child(
                div()
                    .id(SharedString::from(format!("group-chip-{}", gid.0)))
                    .cursor_pointer()
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(7.0))
                    .bg(if active {
                        palette.accent_blue_soft
                    } else {
                        palette.bg_elevated
                    })
                    .hover(|s| if active { s } else { s.bg(palette.bg_hover) })
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .text_size(px(11.0))
                    .line_height(px(14.0))
                    .font_weight(if active {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        palette.accent_blue
                    } else {
                        palette.text_primary
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !was_blind_drag_consumed() {
                            this.assign_item_group(target_item_id, Some(gid), cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.delete_custom_group(gid, cx);
                        }),
                    )
                    .when(active, |el| {
                        el.child(
                            div()
                                .size(px(6.0))
                                .rounded(px(999.0))
                                .bg(palette.accent_blue),
                        )
                    })
                    .child(g.name.clone()),
            );
        }

        div()
            .size_full()
            .rounded(px(12.0))
            .bg(palette.bg_surface)
            .pt(px(14.0))
            .px(px(16.0))
            .pb(px(12.0))
            .flex()
            .flex_col()
            .gap(px(9.0))
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .size(px(7.0))
                                    .rounded(px(999.0))
                                    .bg(palette.accent_blue),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(17.0))
                                    .font_weight(ui_weight_bold())
                                    .text_color(palette.text_primary)
                                    .child("备注与分组"),
                            ),
                    )
                    .child(
                        div()
                            .id("note-group-esc-btn")
                            .cursor_pointer()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_muted)
                            .hover(|s| s.text_color(palette.text_primary))
                            .on_click(cx.listener(|this, _, win, cx| {
                                if !was_blind_drag_consumed() {
                                    this.close_note_and_group_editor(win, cx);
                                }
                            }))
                            .child("Esc 返回预览"),
                    ),
            )
            // Target clip context preview strip
            .child(
                div()
                    .w_full()
                    .rounded(px(8.0))
                    .bg(palette.bg_code)
                    .px(px(10.0))
                    .py(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(ctx_bg)
                            .font_family(crate::theme::mono_font_family())
                            .text_size(px(9.0))
                            .line_height(px(12.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ctx_fg)
                            .child(ctx_tag),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.0))
                            .line_height(px(14.0))
                            .text_color(palette.text_secondary)
                            .truncate()
                            .child(ctx_preview),
                    ),
            )
            // Section 1: Alias input
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text_secondary)
                                    .child("自定义备注 / 别名（优先显示于列表首行）"),
                            )
                            .child(
                                div()
                                    .id("alias-clear-btn")
                                    .cursor_pointer()
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .text_color(palette.text_muted)
                                    .hover(|s| s.text_color(palette.text_primary))
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        this.alias_input
                                            .update(cx, |s, cx| s.set_value("", win, cx));
                                        cx.notify();
                                    }))
                                    .child("清空"),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(32.0))
                            .rounded(px(8.0))
                            .bg(palette.bg_code)
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div().flex_1().min_w_0().child(input_shield(
                                    "alias-input-shield",
                                    Input::new(&self.alias_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .into_any_element(),
                                )),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(crate::theme::mono_font_family())
                                    .text_size(px(9.0))
                                    .text_color(palette.text_muted)
                                    .child(format!("{alias_len}/40")),
                            ),
                    ),
            )
            // Section 2: Group chips + inline create
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text_secondary)
                                    .child("所属分组（单击切换，右键删除）"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .text_color(palette.text_muted)
                                    .child(format!("{group_count} 个分组")),
                            ),
                    )
                    .child(
                        div()
                            .id("group-chips-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(group_chips),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(30.0))
                            .rounded(px(8.0))
                            .bg(palette.bg_code)
                            .pl(px(10.0))
                            .pr(px(5.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div().flex_1().min_w_0().child(input_shield(
                                    "group-input-shield",
                                    Input::new(&self.group_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .into_any_element(),
                                )),
                            )
                            .child(
                                div()
                                    .id("group-create-btn")
                                    .cursor_pointer()
                                    .flex_shrink_0()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded(px(5.0))
                                    .bg(palette.bg_elevated)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.commit_group_create(win, cx);
                                        }
                                    }))
                                    .child("+ 新建"),
                            ),
                    ),
            )
            // Footer actions
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_muted)
                            .child("右键分组标签可删除分组"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("alias-cancel-btn")
                                    .cursor_pointer()
                                    .px(px(11.0))
                                    .py(px(5.0))
                                    .rounded(px(7.0))
                                    .bg(palette.bg_elevated)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(11.0))
                                    .line_height(px(15.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.close_note_and_group_editor(win, cx);
                                        }
                                    }))
                                    .child(t!("vault.cancel_btn").to_string()),
                            )
                            .child(
                                div()
                                    .id("alias-save-btn")
                                    .cursor_pointer()
                                    .px(px(13.0))
                                    .py(px(5.0))
                                    .rounded(px(7.0))
                                    .bg(palette.accent_blue)
                                    .text_size(px(11.0))
                                    .line_height(px(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(gpui_kit::rgb(0xFFFFFF))
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.commit_alias_edit(win, cx);
                                        }
                                    }))
                                    .child("保存更改"),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Renders the inline multi-line text/code content editor in the right pane.
    fn render_content_editor_card(
        &self,
        palette: &ThemePalette,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let current_val = self.content_input.read(cx).value();
        let line_count = current_val.lines().count().max(1);
        let char_count = current_val.chars().count();
        let (kind_badge, badge_bg, badge_fg) = self
            .selected_item
            .as_ref()
            .map(|it| match it.summary.subtype {
                SmartSubtype::Json => ("JSON", palette.accent_green_soft, palette.accent_green),
                SmartSubtype::Code => ("CODE", palette.accent_purple_soft, palette.accent_purple),
                SmartSubtype::Url => ("URL", palette.accent_blue_soft, palette.accent_blue),
                _ => ("TEXT", palette.bg_elevated, palette.text_secondary),
            })
            .unwrap_or(("TEXT", palette.bg_elevated, palette.text_secondary));

        div()
            .size_full()
            .rounded(px(12.0))
            .bg(palette.bg_surface)
            .pt(px(14.0))
            .px(px(16.0))
            .pb(px(12.0))
            .flex()
            .flex_col()
            .gap(px(9.0))
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
                                    .size(px(7.0))
                                    .rounded(px(999.0))
                                    .bg(palette.accent_green),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(17.0))
                                    .font_weight(ui_weight_bold())
                                    .text_color(palette.text_primary)
                                    .child("编辑内容"),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(badge_bg)
                                    .font_family(crate::theme::mono_font_family())
                                    .text_size(px(9.0))
                                    .line_height(px(12.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(badge_fg)
                                    .child(kind_badge),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .id("content-edit-format-btn")
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
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.format_content_editor(win, cx);
                                        }
                                    }))
                                    .child("格式化"),
                            )
                            .child(
                                div()
                                    .id("content-edit-trim-btn")
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
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.trim_content_editor(win, cx);
                                        }
                                    }))
                                    .child("去首尾空格"),
                            ),
                    ),
            )
            // Sunken multi-line editor well with top accent focus strip
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .rounded(px(10.0))
                    .bg(palette.bg_code)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(div().w_full().h(px(2.0)).bg(palette.accent_blue))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .px(px(10.0))
                            .py(px(8.0))
                            .font_family(crate::theme::mono_font_family())
                            .text_size(px(11.0))
                            .child(input_shield(
                                "content-editor-shield",
                                Textarea::new(&self.content_input)
                                    .appearance(false)
                                    .bordered(false)
                                    .h_full()
                                    .into_any_element(),
                            )),
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
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.accent_blue)
                            .child(format!("编辑中 · {line_count} 行 · {char_count} 字符")),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_muted)
                            .child("Esc 放弃 · Ctrl+Enter 保存"),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .id("content-edit-cancel")
                                    .cursor_pointer()
                                    .px(px(10.0))
                                    .py(px(5.0))
                                    .rounded(px(7.0))
                                    .bg(palette.bg_elevated)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(11.0))
                                    .line_height(px(15.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.close_content_editor(win, cx);
                                        }
                                    }))
                                    .child(t!("vault.cancel_btn").to_string()),
                            )
                            .child(
                                div()
                                    .id("content-edit-save-new")
                                    .cursor_pointer()
                                    .px(px(10.0))
                                    .py(px(5.0))
                                    .rounded(px(7.0))
                                    .bg(palette.bg_elevated)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(11.0))
                                    .line_height(px(15.0))
                                    .text_color(palette.text_secondary)
                                    .on_click(cx.listener(|this, _, win, cx| {
                                        if !was_blind_drag_consumed() {
                                            this.save_content_edit(true, false, win, cx);
                                        }
                                    }))
                                    .child("存为新条"),
                            ),
                    )
                    .child(
                        div()
                            .id("content-edit-save-paste")
                            .cursor_pointer()
                            .px(px(13.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(palette.accent_blue)
                            .text_size(px(11.0))
                            .line_height(px(15.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(gpui_kit::rgb(0xFFFFFF))
                            .on_click(cx.listener(|this, _, win, cx| {
                                if !was_blind_drag_consumed() {
                                    this.save_content_edit(false, true, win, cx);
                                }
                            }))
                            .child("保存并粘贴"),
                    ),
            )
            .into_any_element()
    }
}

struct ClipRowActions {
    on_activate: RowIndexCb,
    on_hover: RowHoverCb,
    on_context_menu: RowContextMenuCb,
}

/// Renders a `44px` history list row:
/// aligned `22x22` left slot (color dot / micro-thumb / monochrome hairline icon),
/// unified single `bg_hover` highlight resting on the last hovered row,
/// crisp title + subtle source line, and quiet right-aligned timestamp (or `Paste` in slim mode).
struct ClipRowProps<'a> {
    ix: usize,
    summary: &'a ClipSummary,
    thumb: Option<Arc<Image>>,
    group_label: Option<&'a str>,
    is_active: bool,
    slim_visual: bool,
}

/// Renders a single clipboard history row inside the virtualized list.
fn render_clip_row(
    props: ClipRowProps<'_>,
    palette: &ThemePalette,
    actions: ClipRowActions,
) -> AnyElement {
    let ClipRowProps {
        ix,
        summary,
        thumb,
        group_label,
        is_active,
        slim_visual,
    } = props;
    let row_bg = if is_active {
        palette.bg_hover
    } else {
        gpui_kit::transparent_black()
    };

    let alias_trimmed = summary.alias.trim();
    let has_alias = !alias_trimmed.is_empty();
    let is_color = summary.subtype == SmartSubtype::Color;
    let is_mono_title = !has_alias
        && (is_color
            || matches!(
                summary.subtype,
                SmartSubtype::Code | SmartSubtype::Json | SmartSubtype::Jwt
            ));
    let source = summary.source_app_name.as_deref().unwrap_or("System");
    let rel_time = format_relative_time(summary.updated_at_ms);

    let subtitle = if let Some(gname) = group_label {
        format!("{source} · {gname}")
    } else if summary.kind == ClipKind::Image && !summary.ocr_text.trim().is_empty() {
        format!("{source} · 已识字")
    } else {
        source.to_owned()
    };
    let display_title = if has_alias {
        alias_trimmed.to_owned()
    } else {
        summary.preview_text.clone()
    };

    let show_paste_action = is_active && slim_visual;
    let right_tag = if show_paste_action {
        t!("footer.paste").to_string()
    } else {
        rel_time
    };

    let left_slot: AnyElement = if is_color {
        let swatch = extract_color_swatch_hsla(&summary.preview_text, &summary.meta_json)
            .unwrap_or(palette.accent_blue);
        div()
            .size(px(22.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(16.0)).rounded(px(5.0)).bg(swatch))
            .into_any_element()
    } else if summary.kind == ClipKind::Image {
        div()
            .size(px(22.0))
            .rounded(px(5.0))
            .bg(palette.bg_elevated)
            .overflow_hidden()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(if let Some(decoded) = thumb {
                img(ImageSource::Image(decoded))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            } else {
                kit_icon(IconName::Image, 12.0, palette.text_muted)
            })
            .into_any_element()
    } else {
        let ic = match summary.kind {
            ClipKind::Files => AppIcon::File,
            _ => match summary.subtype {
                SmartSubtype::Code | SmartSubtype::Json | SmartSubtype::Jwt => AppIcon::Code,
                SmartSubtype::Url => AppIcon::Link,
                _ => AppIcon::Text,
            },
        };
        let fg = if is_active {
            palette.text_primary
        } else {
            palette.text_muted
        };
        div()
            .size(px(22.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(app_icon(ic, 14.0, fg))
            .into_any_element()
    };

    let id = summary.id;
    let is_pinned = summary.is_pinned;
    let on_activate = Rc::clone(&actions.on_activate);
    let on_hover = Rc::clone(&actions.on_hover);
    let on_ctx = Rc::clone(&actions.on_context_menu);

    let title_div = div()
        .flex_1()
        .min_w_0()
        .when(is_mono_title, |el| {
            el.font_family(crate::theme::mono_font_family())
        })
        .text_size(px(12.0))
        .line_height(px(15.0))
        .font_weight(if is_active {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        })
        .text_color(palette.text_primary)
        .truncate()
        .child(display_title);

    div()
        .id(SharedString::from(format!("clip-row-{}", id.0)))
        .w_full()
        .h(px(44.0))
        .pb(px(2.0))
        .on_hover(move |hovered, _, cx| {
            if !is_blind_dragging() {
                on_hover(ix, *hovered, cx);
            }
        })
        .on_mouse_down(MouseButton::Left, |_, _, _| {
            reset_blind_drag_click_guard();
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            if !is_blind_dragging() && !was_blind_drag_consumed() {
                on_activate(ix, cx);
            }
        })
        .on_mouse_down(MouseButton::Right, move |ev, _, cx| {
            cx.stop_propagation();
            on_ctx(ix, ev.position, cx);
        })
        .child(
            div()
                .size_full()
                .rounded(px(9.0))
                .bg(row_bg)
                .px(px(10.0))
                .py(px(6.0))
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(left_slot)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(2.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .min_w_0()
                                .when(is_pinned, |el| {
                                    el.child(kit_icon(IconName::Pin, 9.5, palette.accent_blue))
                                })
                                .child(title_div),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .line_height(px(13.0))
                                .text_color(palette.text_muted)
                                .truncate()
                                .child(subtitle),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .font_weight(if show_paste_action {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if show_paste_action {
                            palette.accent_blue
                        } else {
                            palette.text_muted
                        })
                        .child(right_tag),
                ),
        )
        .into_any_element()
}

fn pin_window_topbar_btn(
    id: &'static str,
    pinned: bool,
    palette: &ThemePalette,
    slim: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let box_sz = if slim { 24.0 } else { 26.0 };
    let ic_sz = if slim { 12.0 } else { 13.0 };
    let fg = if pinned {
        palette.accent_blue
    } else {
        palette.text_secondary
    };
    let bg = if pinned {
        palette.accent_blue_soft
    } else {
        gpui_kit::transparent_black()
    };
    div()
        .id(id)
        .cursor_pointer()
        .size(px(box_sz))
        .flex_shrink_0()
        .rounded(px(7.0))
        .bg(bg)
        .hover(|s| s.bg(palette.bg_hover))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, _| {
            reset_blind_drag_click_guard();
        })
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .child(kit_icon(IconName::Pin, ic_sz, fg))
        .into_any_element()
}

fn clean_topbar_icon_btn(
    id: &'static str,
    icon: AppIcon,
    fg: gpui_kit::Hsla,
    bg_opt: Option<gpui_kit::Hsla>,
    palette: &ThemePalette,
    slim: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let box_sz = if slim { 24.0 } else { 26.0 };
    let ic_sz = if slim { 12.0 } else { 13.0 };
    div()
        .id(id)
        .cursor_pointer()
        .size(px(box_sz))
        .flex_shrink_0()
        .rounded(px(7.0))
        .bg(bg_opt.unwrap_or_else(gpui_kit::transparent_black))
        .hover(|s| s.bg(palette.bg_hover))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, _| {
            reset_blind_drag_click_guard();
        })
        .on_click(move |ev, win, cx| {
            if !was_blind_drag_consumed() {
                on_click(ev, win, cx);
            }
        })
        .child(app_icon(icon, ic_sz, fg))
        .into_any_element()
}

fn ctx_menu_item(
    id: &'static str,
    icon: IconName,
    label: &str,
    fg: gpui_kit::Hsla,
    palette: &ThemePalette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .w_full()
        .h(px(24.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .hover(|s| s.bg(palette.bg_hover))
        .flex()
        .items_center()
        .gap(px(7.0))
        .text_size(px(11.0))
        .line_height(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .on_click(on_click)
        .child(kit_icon(icon, 11.0, fg))
        .child(label.to_owned())
        .into_any_element()
}

fn load_data_uri_image(raw: &str) -> Option<Arc<Image>> {
    let (mime_sub, b64) = parse_data_image_uri(raw)?;
    let bytes = decode_base64_bytes(b64)?;
    let fmt = match mime_sub {
        "png" => ImageFormat::Png,
        "jpeg" | "jpg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        _ => return None,
    };
    Some(Arc::new(Image::from_bytes(fmt, bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_state_stays_pinned_on_last_hovered_row_when_pointer_leaves() {
        // Initially idle
        let (active, ptr) = (None, None);
        // Enter row 2
        let (active, ptr) = next_hover_state(active, ptr, 2, true);
        assert_eq!((active, ptr), (Some(2), Some(2)));
        // Leave row 2 (e.g. moving pointer to right preview pane): active stays on 2, pointer clears
        let (active, ptr) = next_hover_state(active, ptr, 2, false);
        assert_eq!((active, ptr), (Some(2), None));
        // Enter row 4
        let (active, ptr) = next_hover_state(active, ptr, 4, true);
        assert_eq!((active, ptr), (Some(4), Some(4)));
        // Out-of-order stale leave from row 2 must NOT disturb row 4
        let (active, ptr) = next_hover_state(active, ptr, 2, false);
        assert_eq!((active, ptr), (Some(4), Some(4)));
        // Leave row 4: active remains pinned on 4
        let (active, ptr) = next_hover_state(active, ptr, 4, false);
        assert_eq!((active, ptr), (Some(4), None));
    }
}
