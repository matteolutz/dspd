use std::collections::HashMap;

use gpui_kit::{
    AnyWindowHandle, App, AppContext, Entity, Global, ReadGlobal, UpdateGlobal, Window,
};

use crate::components::ListPickerContext;

pub fn init(cx: &mut App) {
    cx.set_global(WindowMap::default());
}

#[derive(Default)]
struct WindowMap {
    windows: HashMap<AnyWindowHandle, Entity<WindowContext>>,
}
impl Global for WindowMap {}

pub struct WindowContext {
    list_picker: Entity<ListPickerContext>,
}

impl WindowContext {
    pub fn list_picker(&self) -> Entity<ListPickerContext> {
        self.list_picker.clone()
    }
}

impl WindowContext {
    fn new(cx: &mut App) -> Self {
        let list_picker = cx.new(|cx| ListPickerContext::new(cx));
        Self { list_picker }
    }

    pub fn init(window: &mut Window, cx: &mut App) {
        let window_handle = window.window_handle();
        WindowMap::update_global(cx, |map, cx| {
            map.windows
                .insert(window_handle, cx.new(|cx| WindowContext::new(cx)))
        });
    }
}

pub trait WindowExt {
    fn window_context_entity(&self, cx: &App) -> Entity<WindowContext>;

    fn window_context<'a>(&'a self, cx: &'a mut App) -> &'a WindowContext;
    fn update_window_context(
        &mut self,
        cx: &mut App,
        f: impl FnOnce(&mut WindowContext, &mut Window, &mut App),
    );
}

impl WindowExt for Window {
    fn window_context_entity(&self, cx: &App) -> Entity<WindowContext> {
        let window_handle = self.window_handle();
        WindowMap::global(cx).windows.get(&window_handle)
            .expect("WindowContext not found. Please call WindowContext::init(window, cx) before using this method.")
            .clone()
    }

    fn window_context<'a>(&'a self, cx: &'a mut App) -> &'a WindowContext {
        self.window_context_entity(cx).read(cx)
    }

    fn update_window_context(
        &mut self,
        cx: &mut App,
        f: impl FnOnce(&mut WindowContext, &mut Window, &mut App),
    ) {
        let e = self.window_context_entity(cx);
        e.update(cx, |ctx, cx| f(ctx, self, cx));
    }
}
