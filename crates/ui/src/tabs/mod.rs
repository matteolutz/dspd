use gpui_kit::App;

mod strips;
pub use strips::*;

mod rack;
pub use rack::*;

pub fn init(cx: &mut App) {
    strips::init(cx);
    rack::init(cx);
}
