use gpui_kit::{
    base::StyledExt,
    component::{ActiveTheme, Colorize},
    div, Context, Entity, ParentElement, Render, Styled, Subscription,
};

use crate::state::Stage;

mod generic;
pub use generic::*;

pub struct StageListItem {
    stage: Entity<Stage>,

    _subs: Vec<Subscription>,
}

impl StageListItem {
    pub fn new(stage: &Entity<Stage>, cx: &mut Context<Self>) -> Self {
        let _subs = vec![];

        Self {
            stage: stage.clone(),
            _subs,
        }
    }
}

impl Render for StageListItem {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        div().size_full().flex().flex_col().child(
            div()
                .w_full()
                .h_8()
                .bg(cx.theme().secondary.lighten(0.5))
                .flex()
                .items_center()
                .px_2()
                .font_semibold()
                .child(self.stage.read(cx).name_or_id(cx).to_string()),
        )
    }
}
