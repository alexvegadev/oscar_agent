//! OSCAR's product palette. Views consume Kit theme roles, including light mode.
use gpui::{App, px, rgb};
use gpui_kit::component::{Theme, ThemeMode};

pub(super) fn apply(dark: bool, cx: &mut App) {
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let fonts = cx.text_system().all_font_names();
    Theme::update(cx, |theme| {
        if fonts.iter().any(|name| name == "Inter") {
            theme.font_family = "Inter".into();
        }
        if fonts.iter().any(|name| name == "JetBrains Mono") {
            theme.mono_font_family = "JetBrains Mono".into();
        }
        theme.radius = px(8.);
        theme.radius_lg = px(12.);
        theme.primary = rgb(0x3b82f6).into();
        theme.primary_foreground = rgb(0xf4f4f5).into();
        theme.ring = theme.primary;
        theme.caret = theme.primary;
        if dark {
            theme.background = rgb(0x09090b).into();
            theme.sidebar = rgb(0x121214).into();
            theme.muted = rgb(0x18181b).into();
            theme.border = rgb(0x27272a).into();
            theme.foreground = rgb(0xf4f4f5).into();
            theme.muted_foreground = rgb(0xa1a1aa).into();
            theme.sidebar_border = theme.border;
            theme.sidebar_foreground = theme.foreground;
            theme.sidebar_accent = theme.muted;
            theme.sidebar_accent_foreground = theme.foreground;
            theme.button = theme.muted;
            theme.button_hover = theme.border;
            theme.button_active = theme.border;
            theme.button_foreground = theme.foreground;
            theme.input = theme.border;
            theme.accent = theme.border;
            theme.accent_foreground = theme.foreground;
            theme.success = rgb(0x22c55e).into();
        }
    });
}
