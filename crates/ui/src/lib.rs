//! Shared visual language for Rust Harness.
//!
//! Keep this crate small: semantic tokens and primitives that more than one
//! view needs. Component-specific styling belongs in `harness-views`.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, Theme as KitTheme,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::{
    App, ClickEvent, ElementId, Hsla, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};

/// Semantic colors for the active color scheme.
///
/// The tokens are derived from the gpui-component theme rather than hardcoded,
/// so switching between light and dark keeps our surfaces and the widget
/// library in step.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

impl Theme {
    /// Tokens for the currently active theme.
    pub fn current(cx: &App) -> Self {
        Self::from_kit(cx.theme())
    }

    fn from_kit(kit: &KitTheme) -> Self {
        Self {
            bg: kit.background,
            surface: kit.sidebar,
            border: kit.border,
            text: kit.foreground,
            muted: kit.muted_foreground,
            accent: kit.primary,
            success: kit.success,
            warning: kit.warning,
            danger: kit.danger,
        }
    }
}

/// A small filled circle used for engine and session status.
pub fn status_dot(color: Hsla) -> impl IntoElement {
    div().w(px(9.)).h(px(9.)).rounded_full().bg(color)
}

/// A removable composer attachment: the label plus a small `×` button.
pub fn attachment_chip(
    theme: &Theme,
    label: impl Into<gpui_kit::SharedString>,
    remove_id: impl Into<ElementId>,
    on_remove: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into().to_string();
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .child(div().text_xs().child(label))
        .child(
            Button::new(remove_id)
                .small()
                .ghost()
                .label("×")
                .on_click(on_remove),
        )
}
