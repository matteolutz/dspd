use gpui_kit::{
    assets::IconName,
    base::StyledExt,
    component::{
        button::{Button, ButtonVariants},
        ActiveTheme, Colorize, Sizable, Size,
    },
    div,
    prelude::FluentBuilder,
    AnyView, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription,
};

use crate::{
    components::{ChannelStripColor, GenericStageEditor},
    context::GpuiContextExtension,
    state::{Stage, StripSelectionEvent, UiState},
    tabs::utils::{self, select_stage},
};

mod actions {
    use gpui_kit::{App, KeyBinding};

    #[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
    #[action(namespace = rack, no_json)]
    pub struct ChangeStripBy {
        pub by: i8,
    }

    #[derive(Debug, Clone, PartialEq, gpui_kit::Action)]
    #[action(namespace = rack, no_json)]
    pub struct ChangeStageBy {
        pub by: i8,
    }

    pub const KEY_CONTEXT: &str = "rack";

    pub fn init(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("left", ChangeStripBy { by: -1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("right", ChangeStripBy { by: 1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("up", ChangeStageBy { by: -1 }, Some(KEY_CONTEXT)),
            KeyBinding::new("down", ChangeStageBy { by: 1 }, Some(KEY_CONTEXT)),
        ]);
    }
}

pub(super) fn init(cx: &mut App) {
    actions::init(cx);
}

const MAX_PLUGINS: usize = 8;

struct RackStage {
    stage: Entity<Stage>,
    editor: AnyView,
}

pub struct RackTab {
    stages: [Option<RackStage>; MAX_PLUGINS],

    _subs: Vec<Subscription>,
    focus_handle: FocusHandle,
}

impl RackTab {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let _subs = vec![
            cx.observe_and_notify(&UiState::strip_selection(cx)),
            cx.subscribe(
                &UiState::strip_selection(cx),
                |this, _, evt, cx| match evt {
                    StripSelectionEvent::StripChanged(_) => {
                        this.stages = Self::make_stages(cx);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];

        let stages = Self::make_stages(cx);

        Self {
            stages,
            _subs,
            focus_handle: cx.focus_handle(),
        }
    }

    fn make_stages(cx: &mut Context<Self>) -> [Option<RackStage>; MAX_PLUGINS] {
        let Some(strip) = UiState::strip_selection(cx)
            .read(cx)
            .selected_strip_entity(cx)
        else {
            return std::array::from_fn(|_| None);
        };

        let stages: [Option<Entity<Stage>>; MAX_PLUGINS] = strip
            .read(cx)
            .stages_without_io_meters(cx)
            .cloned()
            .map(Some)
            .chain(std::iter::repeat(None)) // make sure, we get exactly MAX_PLUGINS items
            .take(MAX_PLUGINS)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Failed to collect MAX_PLUGINS");

        stages.map(|s| {
            s.map(|s| {
                let editor: AnyView = cx.new(|cx| GenericStageEditor::new(&s, cx)).into();
                RackStage { editor, stage: s }
            })
        })
    }
}

impl Render for RackTab {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let strip_selection = *UiState::strip_selection(cx).read(cx);

        let Some(strip) = strip_selection
            .selected_strip()
            .and_then(|s| UiState::strip_at(s, cx))
        else {
            return div()
                .size_full()
                .flex()
                .justify_center()
                .items_center()
                .text_color(cx.theme().muted_foreground)
                .child("Please select a channel strip to view its rack")
                .into_any_element();
        };

        let selected_stage_idx = strip_selection.selected_stage();
        let selected_stage = selected_stage_idx.and_then(|idx| self.stages[idx].as_ref());

        div()
            .id("rack")
            .track_focus(&self.focus_handle)
            .key_context(actions::KEY_CONTEXT)
            .on_action(|action: &actions::ChangeStripBy, _, cx| {
                utils::change_strip_by(action.by, cx)
            })
            .on_action(|action: &actions::ChangeStageBy, _, cx| {
                utils::change_stage_by(action.by, cx)
            })
            .w_full()
            .flex_1()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div() // title bar
                    .w_full()
                    .h_12()
                    .flex_shrink_0()
                    .bg(cx.theme().title_bar)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .gap_2()
                    .px_4()
                    .child(
                        div() // selected channel
                            .h_full()
                            .items_center()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new("prev-channel")
                                    .on_click(|_, _, cx| utils::change_strip_by(-1, cx))
                                    .secondary()
                                    .icon(IconName::ChevronLeft)
                                    .with_size(Size::Large)
                                    .size_8()
                                    .text_lg()
                                    .mr_4(),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .rounded_full()
                                            .size_2()
                                            .bg(ChannelStripColor::default().into_hsla()),
                                    )
                                    .child(strip.read(cx).name.clone())
                                    .font_bold()
                                    .text_lg()
                                    .w_32()
                                    .truncate(),
                            )
                            .child(
                                Button::new("next-channel")
                                    .on_click(|_, _, cx| utils::change_strip_by(1, cx))
                                    .secondary()
                                    .icon(IconName::ChevronRight)
                                    .with_size(Size::Large)
                                    .size_8()
                                    .text_lg()
                                    .ml_4(),
                            ),
                    ),
            )
            .child(
                div() // main area
                    .flex_1()
                    .overflow_hidden()
                    .w_full()
                    .flex()
                    .child(
                        div() // stages list
                            .h_full()
                            .w_56()
                            .flex()
                            .flex_col()
                            .p_3()
                            .gap_2()
                            .children(self.stages.iter().enumerate().map(|(idx, stage)| {
                                match stage {
                                    Some(RackStage { stage, .. }) => div()
                                        .id(("stage", idx))
                                        .on_click(move |_, _, cx| select_stage(idx, cx))
                                        .w_full()
                                        .flex_1()
                                        .bg(cx.theme().secondary)
                                        .when(selected_stage_idx == Some(idx), |this| {
                                            this.border_1().border_color(cx.theme().primary)
                                        })
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .w_full()
                                                .h_8()
                                                .bg(cx.theme().secondary.lighten(0.5))
                                                .flex()
                                                .items_center()
                                                .px_2()
                                                .font_semibold()
                                                .child(stage.read(cx).name_or_id(cx).to_string()),
                                        ),
                                    None => div()
                                        .id(("stage", idx))
                                        .hover(|this| this.bg(cx.theme().secondary))
                                        .w_full()
                                        .flex_1()
                                        .border_1()
                                        .border_color(cx.theme().secondary)
                                        .bg(cx.theme().border)
                                        .flex()
                                        .justify_center()
                                        .items_center()
                                        .child("ADD PLUGIN"),
                                }
                            })),
                    )
                    .child(
                        div()
                            .h_full()
                            .flex_1()
                            .bg(cx.theme().border)
                            .p_24()
                            .flex()
                            .justify_center()
                            .items_center()
                            .overflow_hidden()
                            .when_some(selected_stage, |this, RackStage { editor, .. }| {
                                this.child(editor.clone())
                            }),
                    )
                    .child(div().h_full().w_56()),
            )
            .into_any_element()
    }
}
