use gpui_kit::App;

mod strips;
pub use strips::*;

pub fn init(cx: &mut App) {
    strips::init(cx);
}
