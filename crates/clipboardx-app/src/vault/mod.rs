use std::rc::Rc;

use clipboardx_model::{VaultEntry, VaultEntryDraft, VaultEntryId, VaultUnlockPolicy};
use clipboardx_platform::AutofillAction;
use clipboardx_storage::crypto::generate_strong_password;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, Hsla, SharedString, Subscription,
    Window, div, px,
};
use rust_i18n::t;

use crate::icons::{
    input_shield, is_blind_dragging, reset_blind_drag_click_guard, was_blind_drag_consumed,
};
use crate::theme::{ThemePalette, mono_font_family, ui_weight_bold};

/// Local UI state managing vault unlock prompts, inline credential editor,
/// and password masking visibility.
pub struct VaultUiState {
    pub passphrase_input: Entity<InputState>,
    pub title_input: Entity<InputState>,
    pub user_input: Entity<InputState>,
    pub pass_input: Entity<InputState>,
    pub note_input: Entity<InputState>,
    pub editor_open: bool,
    pub editing_id: Option<VaultEntryId>,
    pub editor_auto_submit: bool,
    pub editor_error: Option<String>,
    pub unlock_error: Option<String>,
    pub unlock_key_mismatch: bool,
    pub reveal_password: bool,
    pub reveal_unlock_passphrase: bool,
    pub reveal_editor_password: bool,
    pub _subscriptions: Vec<Subscription>,
}

impl VaultUiState {
    /// Initializes masked passphrase and credential editor `InputState` entities.
    pub fn new<V: 'static>(
        window: &mut Window,
        cx: &mut Context<'_, V>,
        on_unlock_submit: impl Fn(&mut V, &mut Window, &mut Context<'_, V>) + 'static,
    ) -> Self {
        let passphrase_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("vault.passphrase_placeholder").to_string())
        });
        let title_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("vault.title_label").to_string()));
        let user_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("vault.username_label").to_string())
        });
        let pass_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("vault.password_label").to_string())
        });
        let note_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("vault.note_label").to_string()));

        let sub = cx.subscribe_in(&passphrase_input, window, move |view, _st, ev, win, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                on_unlock_submit(view, win, cx);
            }
        });

        Self {
            passphrase_input,
            title_input,
            user_input,
            pass_input,
            note_input,
            editor_open: false,
            editing_id: None,
            editor_auto_submit: true,
            editor_error: None,
            unlock_error: None,
            unlock_key_mismatch: false,
            reveal_password: false,
            reveal_unlock_passphrase: false,
            reveal_editor_password: false,
            _subscriptions: vec![sub],
        }
    }

    pub fn open_new(&mut self, window: &mut Window, cx: &mut App) {
        self.editor_open = true;
        self.editing_id = None;
        self.editor_auto_submit = true;
        self.editor_error = None;
        self.reveal_editor_password = false;
        self.pass_input
            .update(cx, |s, cx| s.set_masked(true, window, cx));
        self.title_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.user_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.pass_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.note_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let focus = self.title_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    pub fn open_edit(&mut self, entry: &VaultEntry, window: &mut Window, cx: &mut App) {
        self.editor_open = true;
        self.editing_id = Some(entry.id);
        self.editor_auto_submit = entry.auto_submit;
        self.editor_error = None;
        self.reveal_editor_password = false;
        self.pass_input
            .update(cx, |s, cx| s.set_masked(true, window, cx));
        self.title_input
            .update(cx, |s, cx| s.set_value(entry.title.clone(), window, cx));
        self.user_input
            .update(cx, |s, cx| s.set_value(entry.username.clone(), window, cx));
        self.pass_input
            .update(cx, |s, cx| s.set_value(entry.password.clone(), window, cx));
        self.note_input
            .update(cx, |s, cx| s.set_value(entry.note.clone(), window, cx));
        let focus = self.title_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Toggles password masking on the vault unlock passphrase input.
    pub fn toggle_passphrase_mask(&mut self, window: &mut Window, cx: &mut App) {
        self.reveal_unlock_passphrase = !self.reveal_unlock_passphrase;
        let masked = !self.reveal_unlock_passphrase;
        self.passphrase_input
            .update(cx, |s, cx| s.set_masked(masked, window, cx));
    }

    /// Toggles password masking on the credential editor password input.
    pub fn toggle_editor_pass_mask(&mut self, window: &mut Window, cx: &mut App) {
        self.reveal_editor_password = !self.reveal_editor_password;
        let masked = !self.reveal_editor_password;
        self.pass_input
            .update(cx, |s, cx| s.set_masked(masked, window, cx));
    }

    /// Fills the editor password input with a newly generated 16-character strong password
    /// and reveals it so the user can verify before saving.
    pub fn fill_generated_password(&mut self, window: &mut Window, cx: &mut App) {
        let generated = generate_strong_password(16);
        self.reveal_editor_password = true;
        self.pass_input.update(cx, |s, cx| {
            s.set_masked(false, window, cx);
            s.set_value(generated, window, cx);
        });
    }

    pub fn read_draft(&self, cx: &App) -> VaultEntryDraft {
        VaultEntryDraft {
            title: self.title_input.read(cx).value().to_string(),
            username: self.user_input.read(cx).value().to_string(),
            password: self.pass_input.read(cx).value().to_string(),
            note: self.note_input.read(cx).value().to_string(),
            auto_submit: self.editor_auto_submit,
        }
    }
}

/// Callback type aliases for vault UI interactions.
pub type VaultWindowCb = Rc<dyn Fn(&mut Window, &mut App)>;
pub type VaultEntryActionCb = Rc<dyn Fn(VaultEntryId, &mut App)>;
pub type VaultEntryEditCb = Rc<dyn Fn(VaultEntry, &mut Window, &mut App)>;
pub type RowIndexCb = Rc<dyn Fn(usize, &mut App)>;
pub type RowHoverCb = Rc<dyn Fn(usize, bool, &mut App)>;
pub type RowContextMenuCb = Rc<dyn Fn(usize, gpui_kit::Point<gpui_kit::Pixels>, &mut App)>;

/// Callbacks triggered from interactive vault preview, editor, and lock cards.
#[derive(Clone)]
pub struct VaultCallbacks {
    pub on_unlock_passphrase: VaultWindowCb,
    pub on_unlock_trusted: VaultWindowCb,
    pub on_reset_vault: VaultWindowCb,
    pub on_lock: Rc<dyn Fn(&mut App)>,
    pub on_autofill: Rc<dyn Fn(VaultEntry, AutofillAction)>,
    pub on_toggle_reveal: Rc<dyn Fn(&mut App)>,
    pub on_toggle_entry_submit: VaultEntryActionCb,
    pub on_edit_entry: VaultEntryEditCb,
    pub on_delete_entry: VaultEntryActionCb,
    pub on_save_editor: VaultWindowCb,
    pub on_cancel_editor: Rc<dyn Fn(&mut App)>,
    pub on_toggle_editor_submit: Rc<dyn Fn(&mut App)>,
    pub on_toggle_passphrase_mask: VaultWindowCb,
    pub on_toggle_editor_pass_mask: VaultWindowCb,
    pub on_generate_password: VaultWindowCb,
}

/// Interaction callbacks wired to each rendered vault list row.
pub struct VaultRowActions {
    pub on_activate: RowIndexCb,
    pub on_hover: RowHoverCb,
    pub on_context_menu: RowContextMenuCb,
    pub on_quick_delete: VaultEntryActionCb,
}

/// Extracts a 1-to-2 character monogram from a credential title for its identity badge.
pub(crate) fn vault_title_monogram(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return "KY".to_owned();
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let first = trimmed.chars().next().unwrap_or('K');
    if !first.is_ascii() {
        return first.to_string();
    }
    if words.len() >= 2
        && let (Some(a), Some(b)) = (words[0].chars().next(), words[1].chars().next())
        && a.is_ascii_alphanumeric()
        && b.is_ascii_alphanumeric()
    {
        return format!("{}{}", a.to_ascii_uppercase(), b.to_ascii_uppercase());
    }
    let alnum: Vec<char> = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(2)
        .collect();
    if alnum.is_empty() {
        "KY".to_owned()
    } else {
        alnum.into_iter().map(|c| c.to_ascii_uppercase()).collect()
    }
}

/// Picks a deterministic soft badge color pair from the entry title hash.
fn vault_badge_colors(title: &str, palette: &ThemePalette) -> (Hsla, Hsla) {
    let hash = title.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    match hash % 4 {
        0 => (palette.accent_amber_soft, palette.accent_amber),
        1 => (palette.accent_blue_soft, palette.accent_blue),
        2 => (palette.accent_purple_soft, palette.accent_purple),
        _ => (palette.accent_green_soft, palette.accent_green),
    }
}

/// Evaluates password strength into `(segments 0..=4, label)` based on length and character diversity.
pub(crate) fn evaluate_password_strength(pw: &str) -> (u8, &'static str) {
    let len = pw.chars().count();
    if len == 0 {
        return (0, "未输入");
    }
    let has_lower = pw.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = pw.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = pw.chars().any(|c| c.is_ascii_digit());
    let has_symbol = pw.chars().any(|c| !c.is_ascii_alphanumeric());
    let classes =
        u8::from(has_lower) + u8::from(has_upper) + u8::from(has_digit) + u8::from(has_symbol);

    let score = if len >= 14 && classes >= 3 {
        4
    } else if len >= 10 && classes >= 3 {
        3
    } else if len >= 8 && classes >= 2 {
        2
    } else {
        1
    };

    let label = match score {
        4 => "高强度",
        3 => "较强",
        2 => "一般",
        _ => "偏弱",
    };
    (score, label)
}

/// Renders a single vault credential row in the left list.
pub fn render_vault_list_row(
    ix: usize,
    entry: &VaultEntry,
    is_active: bool,
    palette: &ThemePalette,
    actions: VaultRowActions,
) -> AnyElement {
    let row_bg = if is_active {
        palette.bg_hover
    } else {
        gpui_kit::transparent_black()
    };

    let subtitle = if entry.auto_submit {
        format!("{} · 回车", entry.username)
    } else {
        entry.username.clone()
    };

    let monogram = vault_title_monogram(&entry.title);
    let (badge_bg, badge_fg) = vault_badge_colors(&entry.title, palette);

    let on_activate = Rc::clone(&actions.on_activate);
    let on_hover = Rc::clone(&actions.on_hover);
    let on_ctx = Rc::clone(&actions.on_context_menu);
    let _ = &actions.on_quick_delete;

    div()
        .id(SharedString::from(format!("vault-row-{}", entry.id.0)))
        .w_full()
        .h(px(44.0))
        .pb(px(2.0))
        .on_hover(move |hovered, _, cx| {
            if !is_blind_dragging() {
                on_hover(ix, *hovered, cx);
            }
        })
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, _| {
            reset_blind_drag_click_guard();
        })
        .on_mouse_up(gpui_kit::MouseButton::Left, move |_, _, cx| {
            if !is_blind_dragging() && !was_blind_drag_consumed() {
                on_activate(ix, cx);
            }
        })
        .on_mouse_down(gpui_kit::MouseButton::Right, move |ev, _, cx| {
            cx.stop_propagation();
            on_ctx(ix, ev.position, cx);
        })
        .child(
            div()
                .size_full()
                .rounded(px(9.0))
                .bg(row_bg)
                .px(px(9.0))
                .py(px(6.0))
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(9.0))
                .child(
                    div()
                        .size(px(26.0))
                        .rounded(px(7.0))
                        .bg(badge_bg)
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(badge_fg)
                        .child(monogram),
                )
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
                                .text_size(px(12.0))
                                .line_height(px(15.0))
                                .font_weight(if is_active {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(palette.text_primary)
                                .truncate()
                                .child(entry.title.clone()),
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
                .when(is_active, |el| {
                    el.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.accent_blue)
                            .child("连填"),
                    )
                }),
        )
        .into_any_element()
}

/// Bundled state for rendering the right-side vault pane.
pub struct VaultPaneProps<'a> {
    pub vault_unlocked: bool,
    pub vault_initialized: bool,
    pub has_passphrase: bool,
    pub policy: VaultUnlockPolicy,
    pub selected_entry: Option<&'a VaultEntry>,
    pub editor_password_value: &'a str,
}

/// Renders the right-side vault surface (editor, lock prompt, empty state, or credential detail).
pub fn render_vault_right_pane(
    props: VaultPaneProps<'_>,
    state: &VaultUiState,
    palette: &ThemePalette,
    callbacks: &VaultCallbacks,
) -> AnyElement {
    if state.editor_open {
        return render_editor_card(state, props.editor_password_value, palette, callbacks);
    }

    if !props.vault_unlocked {
        return render_locked_card(
            props.vault_initialized,
            props.has_passphrase,
            props.policy,
            state,
            palette,
            callbacks,
        );
    }

    let Some(entry) = props.selected_entry else {
        return div()
            .size_full()
            .rounded(px(12.0))
            .bg(palette.bg_surface)
            .p(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .child(
                div()
                    .size(px(40.0))
                    .rounded(px(11.0))
                    .bg(palette.accent_amber_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(mono_font_family())
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.accent_amber)
                    .child("KEY"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(ui_weight_bold())
                    .text_color(palette.text_primary)
                    .child(t!("vault.empty_title").to_string()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(palette.text_muted)
                    .text_center()
                    .child(t!("vault.empty_hint").to_string()),
            )
            .into_any_element();
    };

    render_entry_detail_card(
        entry,
        props.policy,
        state.reveal_password,
        palette,
        callbacks,
    )
}

/// Renders the vault lock or initial passphrase setup card.
fn render_locked_card(
    vault_initialized: bool,
    has_passphrase: bool,
    policy: VaultUnlockPolicy,
    state: &VaultUiState,
    palette: &ThemePalette,
    callbacks: &VaultCallbacks,
) -> AnyElement {
    let is_init = !vault_initialized || !has_passphrase;
    let title = if is_init {
        t!("vault.init_title").to_string()
    } else {
        "保险箱已锁定".to_owned()
    };
    let hint = if is_init {
        t!("vault.init_hint").to_string()
    } else {
        "输入主密码解锁本地加密库，凭证仅驻留受保护内存".to_owned()
    };
    let btn_label = if is_init {
        t!("vault.create_btn").to_string()
    } else {
        "解锁账号保险箱".to_owned()
    };

    let show_trusted_btn =
        !vault_initialized || !has_passphrase || policy == VaultUnlockPolicy::TrustedDeviceNoPrompt;
    let on_unlock = Rc::clone(&callbacks.on_unlock_passphrase);
    let on_trusted = Rc::clone(&callbacks.on_unlock_trusted);
    let on_reset = Rc::clone(&callbacks.on_reset_vault);
    let on_toggle_mask = Rc::clone(&callbacks.on_toggle_passphrase_mask);

    div()
        .size_full()
        .rounded(px(12.0))
        .bg(palette.bg_surface)
        .px(px(20.0))
        .py(px(16.0))
        .flex()
        .flex_col()
        .justify_between()
        // Top security status strip
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
                                .size(px(6.0))
                                .rounded(px(999.0))
                                .bg(palette.accent_amber),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_secondary)
                                .child("账号保险箱"),
                        ),
                )
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(palette.bg_elevated)
                        .font_family(mono_font_family())
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .text_color(palette.text_muted)
                        .child("Argon2id · AES-256"),
                ),
        )
        // Center unlock form
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .size(px(44.0))
                        .rounded(px(12.0))
                        .bg(palette.accent_amber_soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(mono_font_family())
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.accent_amber)
                        .child("KEY"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_size(px(15.0))
                                .line_height(px(20.0))
                                .font_weight(ui_weight_bold())
                                .text_color(palette.text_primary)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(15.0))
                                .text_color(palette.text_secondary)
                                .text_center()
                                .child(hint),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .max_w(px(314.0))
                        .h(px(36.0))
                        .rounded(px(9.0))
                        .bg(palette.bg_code)
                        .pl(px(12.0))
                        .pr(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div().flex_1().min_w_0().child(input_shield(
                                "vault-passphrase-shield",
                                Input::new(&state.passphrase_input)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .into_any_element(),
                            )),
                        )
                        .child(
                            div()
                                .id("vault-unlock-toggle-mask")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(palette.bg_elevated)
                                .hover(|s| s.bg(palette.bg_hover))
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .text_color(palette.text_muted)
                                .on_click(move |_, win, cx| on_toggle_mask(win, cx))
                                .child(if state.reveal_unlock_passphrase {
                                    "隐藏"
                                } else {
                                    "显示"
                                }),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(palette.bg_surface)
                                .font_family(mono_font_family())
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .text_color(palette.text_muted)
                                .child("Enter"),
                        ),
                )
                .when_some(state.unlock_error.clone(), |el, err| {
                    el.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(palette.accent_red)
                            .text_center()
                            .child(err),
                    )
                })
                .child(
                    div()
                        .id("btn-vault-unlock")
                        .cursor_pointer()
                        .w_full()
                        .max_w(px(314.0))
                        .h(px(34.0))
                        .rounded(px(8.0))
                        .bg(palette.accent_blue)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui_kit::rgb(0xFFFFFF))
                        .on_click(move |_, window, cx| on_unlock(window, cx))
                        .child(btn_label),
                ),
        )
        // Bottom secondary action (trusted-device unlock or reset on mismatch)
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .when(show_trusted_btn && !state.unlock_key_mismatch, |el| {
                    el.child(
                        div()
                            .id("btn-vault-trusted-init")
                            .cursor_pointer()
                            .px(px(12.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(palette.bg_elevated)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(palette.text_secondary)
                            .on_click(move |_, window, cx| on_trusted(window, cx))
                            .child("使用本机密钥环免密解锁"),
                    )
                })
                .when(state.unlock_key_mismatch, |el| {
                    el.child(
                        div()
                            .id("btn-vault-reset")
                            .cursor_pointer()
                            .px(px(12.0))
                            .py(px(5.0))
                            .rounded(px(7.0))
                            .bg(palette.bg_elevated)
                            .hover(|s| s.bg(palette.bg_hover))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.accent_red)
                            .on_click(move |_, window, cx| on_reset(window, cx))
                            .child(t!("vault.reset_vault_btn").to_string()),
                    )
                }),
        )
        .into_any_element()
}

/// Renders the unlocked credential detail card with paired fields and autofill sequence preview.
fn render_entry_detail_card(
    entry: &VaultEntry,
    policy: VaultUnlockPolicy,
    reveal_password: bool,
    palette: &ThemePalette,
    callbacks: &VaultCallbacks,
) -> AnyElement {
    let status_pill_text = if policy == VaultUnlockPolicy::TrustedDeviceNoPrompt {
        "信任本机"
    } else {
        "已解锁"
    };

    let mask_str = if reveal_password {
        entry.password.clone()
    } else {
        "•".repeat(entry.password.chars().count().clamp(8, 14))
    };

    let monogram = vault_title_monogram(&entry.title);
    let (badge_bg, badge_fg) = vault_badge_colors(&entry.title, palette);

    let on_autofill_full = Rc::clone(&callbacks.on_autofill);
    let on_autofill_user = Rc::clone(&callbacks.on_autofill);
    let on_autofill_pass = Rc::clone(&callbacks.on_autofill);
    let entry_full = entry.clone();
    let entry_user = entry.clone();
    let entry_pass = entry.clone();

    let on_edit = Rc::clone(&callbacks.on_edit_entry);
    let entry_edit = entry.clone();
    let on_delete = Rc::clone(&callbacks.on_delete_entry);
    let entry_id = entry.id;
    let on_lock = Rc::clone(&callbacks.on_lock);
    let on_reveal = Rc::clone(&callbacks.on_toggle_reveal);
    let on_toggle_submit = Rc::clone(&callbacks.on_toggle_entry_submit);

    div()
        .size_full()
        .rounded(px(12.0))
        .bg(palette.bg_surface)
        .overflow_hidden()
        .pt(px(15.0))
        .px(px(16.0))
        .pb(px(12.0))
        .flex()
        .flex_col()
        .gap(px(11.0))
        // Identity header
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .size(px(34.0))
                                .rounded(px(9.0))
                                .bg(badge_bg)
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .font_family(mono_font_family())
                                .text_size(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(badge_fg)
                                .child(monogram),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(14.0))
                                        .line_height(px(18.0))
                                        .font_weight(ui_weight_bold())
                                        .text_color(palette.text_primary)
                                        .truncate()
                                        .child(entry.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .line_height(px(13.0))
                                        .text_color(palette.text_muted)
                                        .child("SQLCipher 信封加密 · 本地离线"),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(999.0))
                        .bg(palette.accent_green_soft)
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(
                            div()
                                .size(px(6.0))
                                .rounded(px(999.0))
                                .bg(palette.accent_green),
                        )
                        .child(
                            div()
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.accent_green)
                                .child(status_pill_text),
                        ),
                ),
        )
        // Paired username & password well
        .child(
            div()
                .w_full()
                .rounded(px(10.0))
                .bg(palette.bg_code)
                .p(px(6.0))
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(
                    div()
                        .w_full()
                        .h(px(34.0))
                        .rounded(px(7.0))
                        .bg(palette.bg_elevated)
                        .pl(px(10.0))
                        .pr(px(7.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .w(px(30.0))
                                .flex_shrink_0()
                                .text_size(px(10.0))
                                .line_height(px(14.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_muted)
                                .child("账号"),
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
                                .child(entry.username.clone()),
                        )
                        .child(small_pill(
                            "vault-copy-user",
                            "填账号".to_owned(),
                            palette.bg_surface,
                            palette.bg_hover,
                            palette.text_secondary,
                            move |_, _| {
                                on_autofill_user(entry_user.clone(), AutofillAction::UsernameOnly);
                            },
                        )),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(34.0))
                        .rounded(px(7.0))
                        .bg(palette.bg_elevated)
                        .pl(px(10.0))
                        .pr(px(7.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(30.0))
                                .flex_shrink_0()
                                .text_size(px(10.0))
                                .line_height(px(14.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.text_muted)
                                .child("密码"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(mono_font_family())
                                .text_size(px(12.0))
                                .line_height(px(15.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(palette.text_primary)
                                .truncate()
                                .child(mask_str),
                        )
                        .child(small_pill(
                            "vault-reveal-pw",
                            if reveal_password {
                                "隐藏".to_owned()
                            } else {
                                "显示".to_owned()
                            },
                            palette.bg_surface,
                            palette.bg_hover,
                            palette.text_muted,
                            move |_, cx| on_reveal(cx),
                        ))
                        .child(small_pill(
                            "vault-copy-pass",
                            "填密码".to_owned(),
                            palette.bg_surface,
                            palette.bg_hover,
                            palette.text_secondary,
                            move |_, _| {
                                on_autofill_pass(entry_pass.clone(), AutofillAction::PasswordOnly);
                            },
                        )),
                ),
        )
        // Visual autofill sequence preview card with auto-submit toggle
        .child(
            div()
                .id("vault-auto-submit-row")
                .cursor_pointer()
                .w_full()
                .rounded(px(9.0))
                .bg(palette.bg_code)
                .px(px(11.0))
                .py(px(9.0))
                .on_click(move |_, _, cx| on_toggle_submit(entry_id, cx))
                .flex()
                .flex_col()
                .gap(px(7.0))
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
                                .child("连填序列预览 · 完成后自动还原剪贴板"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .line_height(px(13.0))
                                        .text_color(palette.text_muted)
                                        .child("自动回车"),
                                )
                                .child(mini_toggle(entry.auto_submit, palette)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(step_chip(
                            "账号",
                            false,
                            palette.bg_elevated,
                            palette.text_primary,
                        ))
                        .child(step_arrow(palette))
                        .child(step_chip(
                            "Tab",
                            true,
                            palette.bg_elevated,
                            palette.text_secondary,
                        ))
                        .child(step_arrow(palette))
                        .child(step_chip(
                            "密码",
                            false,
                            palette.bg_elevated,
                            palette.text_primary,
                        ))
                        .child(step_arrow(palette))
                        .child(step_chip(
                            "Enter",
                            true,
                            if entry.auto_submit {
                                palette.accent_blue_soft
                            } else {
                                palette.bg_elevated
                            },
                            if entry.auto_submit {
                                palette.accent_blue
                            } else {
                                palette.text_muted
                            },
                        )),
                ),
        )
        // Optional note line
        .when(!entry.note.trim().is_empty(), |el| {
            el.child(
                div()
                    .px(px(2.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .line_height(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.text_muted)
                            .child("备注"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.0))
                            .line_height(px(14.0))
                            .text_color(palette.text_secondary)
                            .truncate()
                            .child(entry.note.trim().to_owned()),
                    ),
            )
        })
        .child(div().flex_1())
        // Bottom action bar
        .child(
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
                        .gap(px(2.0))
                        .child(
                            div()
                                .id("vault-edit-btn")
                                .cursor_pointer()
                                .px(px(7.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .hover(|s| {
                                    s.bg(palette.bg_elevated).text_color(palette.text_primary)
                                })
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .text_color(palette.text_muted)
                                .on_click(move |_, window, cx| {
                                    on_edit(entry_edit.clone(), window, cx)
                                })
                                .child(t!("preview.edit").to_string()),
                        )
                        .child(
                            div()
                                .id("vault-lock-btn")
                                .cursor_pointer()
                                .px(px(7.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .hover(|s| {
                                    s.bg(palette.bg_elevated).text_color(palette.text_primary)
                                })
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .text_color(palette.text_muted)
                                .on_click(move |_, _, cx| on_lock(cx))
                                .child(t!("vault.lock_btn").to_string()),
                        )
                        .child(
                            div()
                                .id("vault-del-btn")
                                .cursor_pointer()
                                .px(px(7.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .hover(|s| s.bg(palette.bg_elevated).text_color(palette.accent_red))
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .text_color(palette.text_muted)
                                .on_click(move |_, _, cx| on_delete(entry_id, cx))
                                .child(t!("preview.delete").to_string()),
                        ),
                )
                .child(
                    div()
                        .id("vault-autofill-cta")
                        .cursor_pointer()
                        .flex_shrink_0()
                        .px(px(13.0))
                        .py(px(5.0))
                        .rounded(px(7.0))
                        .bg(palette.accent_blue)
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .on_click(move |_, _, _| {
                            if !was_blind_drag_consumed() {
                                on_autofill_full(entry_full.clone(), AutofillAction::FullSequence);
                            }
                        })
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(gpui_kit::rgb(0xFFFFFF))
                                .child("一键连填"),
                        )
                        .child(
                            div()
                                .px(px(5.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(gpui_kit::hsla(0.0, 0.0, 1.0, 0.2))
                                .font_family(mono_font_family())
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .text_color(gpui_kit::rgb(0xFFFFFF))
                                .child("Enter"),
                        ),
                ),
        )
        .into_any_element()
}

/// Renders the inline credential creation/editing form with live strength meter and generator.
fn render_editor_card(
    state: &VaultUiState,
    password_value: &str,
    palette: &ThemePalette,
    callbacks: &VaultCallbacks,
) -> AnyElement {
    let heading = if state.editing_id.is_some() {
        "编辑账号凭证"
    } else {
        "新建账号凭证"
    };

    let (strength_segs, strength_label) = evaluate_password_strength(password_value);
    let pw_len = password_value.chars().count();
    let strength_color = match strength_segs {
        4 => palette.accent_green,
        3 => palette.accent_blue,
        2 => palette.accent_amber,
        1 => palette.accent_red,
        _ => palette.text_muted,
    };
    let strength_text = if pw_len > 0 {
        format!("{strength_label} · {pw_len} 位")
    } else {
        "支持一键生成 16 位强密码".to_owned()
    };

    let on_save = Rc::clone(&callbacks.on_save_editor);
    let on_cancel = Rc::clone(&callbacks.on_cancel_editor);
    let on_cancel_top = Rc::clone(&callbacks.on_cancel_editor);
    let on_toggle = Rc::clone(&callbacks.on_toggle_editor_submit);
    let on_gen = Rc::clone(&callbacks.on_generate_password);
    let on_mask = Rc::clone(&callbacks.on_toggle_editor_pass_mask);

    div()
        .size_full()
        .rounded(px(12.0))
        .bg(palette.bg_surface)
        .pt(px(14.0))
        .px(px(16.0))
        .pb(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        // Header
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
                                .bg(palette.accent_amber),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .line_height(px(17.0))
                                .font_weight(ui_weight_bold())
                                .text_color(palette.text_primary)
                                .child(heading),
                        ),
                )
                .child(if let Some(err) = &state.editor_error {
                    div()
                        .text_size(px(10.0))
                        .text_color(palette.accent_red)
                        .child(err.clone())
                        .into_any_element()
                } else {
                    div()
                        .id("vault-editor-esc-top")
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .line_height(px(13.0))
                        .text_color(palette.text_muted)
                        .hover(|s| s.text_color(palette.text_primary))
                        .on_click(move |_, _, cx| on_cancel_top(cx))
                        .child("Esc 取消")
                        .into_any_element()
                }),
        )
        // Form fields
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(editor_field_row("标题", &state.title_input, None, palette))
                .child(editor_field_row("账号", &state.user_input, None, palette))
                .child(editor_field_row(
                    "密码",
                    &state.pass_input,
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .flex_shrink_0()
                            .child(
                                div()
                                    .id("vault-gen-strong-pw")
                                    .cursor_pointer()
                                    .px(px(7.0))
                                    .py(px(2.0))
                                    .rounded(px(5.0))
                                    .bg(palette.accent_blue_soft)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(9.0))
                                    .line_height(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.accent_blue)
                                    .on_click(move |_, win, cx| on_gen(win, cx))
                                    .child("生成强密码"),
                            )
                            .child(
                                div()
                                    .id("vault-editor-toggle-mask")
                                    .cursor_pointer()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded(px(5.0))
                                    .bg(palette.bg_elevated)
                                    .hover(|s| s.bg(palette.bg_hover))
                                    .text_size(px(9.0))
                                    .line_height(px(12.0))
                                    .text_color(palette.text_muted)
                                    .on_click(move |_, win, cx| on_mask(win, cx))
                                    .child(if state.reveal_editor_password {
                                        "隐藏"
                                    } else {
                                        "显示"
                                    }),
                            )
                            .into_any_element(),
                    ),
                    palette,
                ))
                // 4-segment password strength indicator
                .child(
                    div()
                        .w_full()
                        .px(px(2.0))
                        .pb(px(2.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(div().flex_1().flex().items_center().gap(px(4.0)).children(
                            (1..=4u8).map(|idx| {
                                div().flex_1().h(px(4.0)).rounded(px(999.0)).bg(
                                    if idx <= strength_segs {
                                        strength_color
                                    } else {
                                        palette.bg_elevated
                                    },
                                )
                            }),
                        ))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(9.0))
                                .line_height(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(strength_color)
                                .child(strength_text),
                        ),
                )
                .child(editor_field_row("备注", &state.note_input, None, palette)),
        )
        .child(div().flex_1())
        // Footer with auto-submit toggle and Save/Cancel actions
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id("vault-editor-auto-submit")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .on_click(move |_, _, cx| on_toggle(cx))
                        .child(mini_toggle(state.editor_auto_submit, palette))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(14.0))
                                .text_color(palette.text_secondary)
                                .child("连填后自动回车"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id("vault-editor-cancel")
                                .cursor_pointer()
                                .px(px(11.0))
                                .py(px(5.0))
                                .rounded(px(7.0))
                                .bg(palette.bg_elevated)
                                .hover(|s| s.bg(palette.bg_hover))
                                .text_size(px(11.0))
                                .line_height(px(15.0))
                                .text_color(palette.text_secondary)
                                .on_click(move |_, _, cx| on_cancel(cx))
                                .child(t!("vault.cancel_btn").to_string()),
                        )
                        .child(
                            div()
                                .id("vault-editor-save")
                                .cursor_pointer()
                                .px(px(13.0))
                                .py(px(5.0))
                                .rounded(px(7.0))
                                .bg(palette.accent_blue)
                                .text_size(px(11.0))
                                .line_height(px(15.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(gpui_kit::rgb(0xFFFFFF))
                                .on_click(move |_, window, cx| on_save(window, cx))
                                .child("保存至加密库"),
                        ),
                ),
        )
        .into_any_element()
}

fn editor_field_row(
    label: &str,
    input: &Entity<InputState>,
    right_slot: Option<AnyElement>,
    palette: &ThemePalette,
) -> AnyElement {
    div()
        .w_full()
        .h(px(34.0))
        .rounded(px(8.0))
        .bg(palette.bg_code)
        .pl(px(10.0))
        .pr(px(8.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .w(px(36.0))
                .flex_shrink_0()
                .text_size(px(10.0))
                .line_height(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.text_muted)
                .child(label.to_owned()),
        )
        .child(
            div().flex_1().min_w_0().child(input_shield(
                SharedString::from(format!("vault-field-{label}")),
                Input::new(input)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
                    .into_any_element(),
            )),
        )
        .when_some(right_slot, |el, slot| el.child(slot))
        .into_any_element()
}

fn mini_toggle(on: bool, palette: &ThemePalette) -> AnyElement {
    div()
        .w(px(28.0))
        .h(px(16.0))
        .rounded(px(999.0))
        .bg(if on {
            palette.accent_blue
        } else {
            palette.bg_elevated
        })
        .p(px(2.0))
        .flex()
        .items_center()
        .when(on, |el| el.justify_end())
        .when(!on, |el| el.justify_start())
        .child(
            div()
                .size(px(12.0))
                .rounded(px(999.0))
                .bg(gpui_kit::rgb(0xFFFFFF)),
        )
        .into_any_element()
}

fn step_chip(label: &'static str, mono: bool, bg: Hsla, fg: Hsla) -> AnyElement {
    div()
        .px(px(7.0))
        .py(px(2.5))
        .rounded(px(5.0))
        .bg(bg)
        .when(mono, |el| el.font_family(mono_font_family()))
        .text_size(px(if mono { 9.0 } else { 10.0 }))
        .line_height(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .child(label)
        .into_any_element()
}

fn step_arrow(palette: &ThemePalette) -> AnyElement {
    div()
        .font_family(mono_font_family())
        .text_size(px(10.0))
        .text_color(palette.text_muted)
        .child("→")
        .into_any_element()
}

fn small_pill(
    id: &'static str,
    label: String,
    bg_idle: Hsla,
    bg_hover: Hsla,
    fg: Hsla,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex_shrink_0()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .bg(bg_idle)
        .hover(|s| s.bg(bg_hover))
        .text_size(px(10.0))
        .line_height(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .on_click(move |_, window, cx| {
            if !was_blind_drag_consumed() {
                on_click(window, cx);
            }
        })
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monogram_handles_latin_multiword_and_cjk() {
        assert_eq!(vault_title_monogram("GitHub 个人开发账号"), "GI");
        assert_eq!(vault_title_monogram("Cloud Flare"), "CF");
        assert_eq!(vault_title_monogram("生产数据库"), "生");
        assert_eq!(vault_title_monogram("   "), "KY");
    }

    #[test]
    fn password_strength_ranks_weak_to_strong() {
        assert_eq!(evaluate_password_strength("").0, 0);
        assert_eq!(evaluate_password_strength("12345").0, 1);
        assert_eq!(evaluate_password_strength("Abcd1234").0, 2);
        assert_eq!(evaluate_password_strength("Abcd1234!x").0, 3);
        assert_eq!(evaluate_password_strength("Kp9#vL2m!xR8qW4z").0, 4);
    }
}
