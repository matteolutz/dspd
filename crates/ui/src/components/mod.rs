pub mod meters;
pub mod mixing;

mod strip;
use gpui_kit::App;
pub use strip::*;

mod stage;
pub use stage::*;

mod list_picker;
pub use list_picker::*;

mod window;
pub use window::*;

mod utils;

pub fn init(cx: &mut App) {
    window::init(cx);
}
