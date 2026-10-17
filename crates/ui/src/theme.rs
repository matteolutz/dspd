use std::path::PathBuf;

use gpui_kit::{
    component::{Theme, ThemeRegistry},
    App, SharedString,
};

pub(super) fn set_theme(cx: &mut App) {
    let themes_dir = PathBuf::from("./themes");
    println!("Looking for themes in {}", themes_dir.display());

    let theme_name = SharedString::from("macOS Classic Dark");
    // Load and watch themes from ./themes directory
    if let Err(err) = ThemeRegistry::watch_dir(themes_dir, cx, move |cx| {
        if let Some(theme) = ThemeRegistry::global(cx).themes().get(&theme_name).cloned() {
            Theme::update(cx, |current| current.apply_config(&theme));
            println!("theme was set!");
        }
    }) {
        println!("Failed to watch themes directory: {}", err);
    }
}
