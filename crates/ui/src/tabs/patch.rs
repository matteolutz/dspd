use gpui_kit::{
    base::{Tab, Tabs},
    component::{scroll::ScrollableElement, ActiveTheme},
    div,
    prelude::FluentBuilder,
    Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, PromptButton,
    PromptLevel, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
};
use itertools::Itertools;

use crate::{
    context::GpuiContextExtension,
    state::{PatchDirection, UiState},
    window::WindowExt,
};

trait PatchDirectionExt {
    fn name(&self) -> &'static str;
}
impl PatchDirectionExt for PatchDirection {
    fn name(&self) -> &'static str {
        match self {
            Self::Capture => "Capture",
            Self::Playback => "Playback",
        }
    }
}

pub struct PatchTab {
    direction: PatchDirection,
    selected_device_id: Option<SharedString>,

    _subs: Vec<Subscription>,
}

impl PatchTab {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _subs = vec![cx.observe_and_notify(&UiState::patch(cx))];

        let direction = PatchDirection::Capture;
        let selected_device_id = direction.devices(cx).first().map(|d| d.id.clone());

        Self {
            direction: PatchDirection::Capture,
            selected_device_id,
            _subs,
        }
    }

    fn set_direction(&mut self, direction: PatchDirection, cx: &mut Context<Self>) {
        self.direction = direction;
        self.selected_device_id = direction.devices(cx).first().map(|d| d.id.clone());
        cx.notify();
    }

    fn do_patch(
        &mut self,
        channel_idx: usize,
        device_channel_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(device_id) = self.selected_device_id.as_ref() else {
            return;
        };

        let device_id = device_id.to_string();
        let direction = self.direction;

        let current_patch_entry = self.direction.get_patch_for(channel_idx, cx);
        if current_patch_entry.is_some_and(|e| e.device_id != device_id) {
            // this channel is already patched to a different device
            window.spawn_detached_prompt(
                PromptLevel::Warning,
                "Channel is already patched to another device",
                Some(format!("Channel {} is already patched to a different device. Do you want to overwrite the existing patch?", channel_idx + 1).as_str()),
                &[PromptButton::ok("Overwrite"), PromptButton::cancel("Cancel")],
                cx,
                move |answer, _, cx| {
                    if answer == 1 {
                        // cancel
                        return;
                    }

                    cx.spawn(async move |cx| {
                        _ = UiState::client(cx).patch_device(direction, &device_id, device_channel_idx, channel_idx, cx).await;
                    }).detach();
                },
            );

            return;
        }

        cx.spawn(async move |_, cx| {
            _ = UiState::client(cx)
                .patch_device(direction, &device_id, device_channel_idx, channel_idx, cx)
                .await;
        })
        .detach();
    }
}

impl PatchTab {
    fn render_direction_selector(
        &self,
        _window: &mut Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        Tabs::new("direction")
            .rounded(cx.theme().radius)
            .max_w_112()
            .h_12()
            .flex()
            .bg(cx.theme().secondary)
            .overflow_hidden()
            .children(
                [PatchDirection::Capture, PatchDirection::Playback]
                    .into_iter()
                    .with_position()
                    .map(|(direction_pos, direction)| {
                        Tab::new(direction.name())
                            .flex_1()
                            .px_4()
                            .selected(self.direction == direction)
                            .h_full()
                            .flex()
                            .items_center()
                            .when_else(
                                self.direction == direction,
                                |this| {
                                    this.font_weight(gpui_kit::FontWeight::BOLD)
                                        .bg(cx.theme().primary)
                                },
                                |this| this.hover(|this| this.bg(cx.theme().secondary_hover)),
                            )
                            .when(direction_pos.is_first(), |this| {
                                this.rounded_l(cx.theme().radius)
                            })
                            .when(direction_pos.is_last(), |this| {
                                this.rounded_r(cx.theme().radius)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_direction(direction, cx);
                            }))
                            .child(direction.name())
                    }),
            )
    }

    fn render_device_selector(&self, _window: &mut Window, cx: &Context<Self>) -> impl IntoElement {
        let devices = UiState::patch(cx).read(cx).devices_for(self.direction);

        Tabs::new("selected-device")
            .rounded(cx.theme().radius)
            .max_w_112()
            .h_12()
            .flex()
            .bg(cx.theme().secondary)
            .overflow_hidden()
            .children(
                devices
                    .into_iter()
                    .with_position()
                    .map(|(device_pos, dev)| {
                        let is_selected = self
                            .selected_device_id
                            .as_ref()
                            .is_some_and(|sel| sel == &dev.id);

                        Tab::new(dev.id.clone())
                            .flex_1()
                            .px_4()
                            .selected(is_selected)
                            .h_full()
                            .flex()
                            .items_center()
                            .when_else(
                                is_selected,
                                |this| {
                                    this.font_weight(gpui_kit::FontWeight::BOLD)
                                        .bg(cx.theme().primary)
                                },
                                |this| this.hover(|this| this.bg(cx.theme().secondary_hover)),
                            )
                            .when(device_pos.is_first(), |this| {
                                this.rounded_l(cx.theme().radius)
                            })
                            .when(device_pos.is_last(), |this| {
                                this.rounded_r(cx.theme().radius)
                            })
                            .on_click({
                                let dev_id = dev.id.clone();
                                cx.listener(move |this, _, _, cx| {
                                    this.selected_device_id = Some(dev_id.clone());
                                    cx.notify();
                                })
                            })
                            .child(dev.name.clone())
                    }),
            )
    }
}

impl Render for PatchTab {
    fn render(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::prelude::Context<Self>,
    ) -> impl gpui_kit::prelude::IntoElement {
        let direction_channels = self.direction.get_num_channels(cx);
        let selected_device = self
            .selected_device_id
            .as_ref()
            .and_then(|id| self.direction.device_by_id(id, cx));

        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_2()
            .py_2()
            .child(self.render_direction_selector(window, cx))
            .child(self.render_device_selector(window, cx))
            .child(div().w_full().h_0p5().bg(cx.theme().secondary))
            .when_some(selected_device, |this, selected_device| {
                this.child(
                    div()
                        .w_full()
                        .h_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .grid()
                                .grid_rows(direction_channels as u16 + 1)
                                .grid_cols(selected_device.num_channels as u16 + 1)
                                .children(
                                    (0..((direction_channels + 1)
                                        * (selected_device.num_channels + 1)))
                                        .map(|idx| {
                                            let channel_idx =
                                                idx / (selected_device.num_channels + 1);
                                            let device_channel_idx =
                                                idx % (selected_device.num_channels + 1);

                                            // top left corner
                                            if channel_idx == 0 && device_channel_idx == 0 {
                                                return div()
                                                    .row_start(1)
                                                    .col_start(1)
                                                    .row_span(1)
                                                    .col_span(1)
                                                    .size_7()
                                                    .into_any_element();
                                            }

                                            // top row (device channel labels)
                                            if channel_idx == 0 {
                                                return div()
                                                    .row_start(1)
                                                    .col_start(device_channel_idx as i16 + 1)
                                                    .row_span(1)
                                                    .col_span(1)
                                                    .size_7()
                                                    .border_1()
                                                    .border_t_0()
                                                    .border_color(cx.theme().secondary)
                                                    .flex()
                                                    .justify_center()
                                                    .items_center()
                                                    .bg(cx.theme().background)
                                                    .text_xs()
                                                    .child(format!("{}", device_channel_idx))
                                                    .into_any_element();
                                            }

                                            // left column (direction channel labels)
                                            if device_channel_idx == 0 {
                                                return div()
                                                    .row_start(channel_idx as i16 + 1)
                                                    .col_start(1)
                                                    .row_span(1)
                                                    .col_span(1)
                                                    .size_7()
                                                    .border_1()
                                                    .border_l_0()
                                                    .border_color(cx.theme().secondary)
                                                    .flex()
                                                    .justify_center()
                                                    .items_center()
                                                    .bg(cx.theme().background)
                                                    .text_xs()
                                                    .child(format!("{}", channel_idx))
                                                    .into_any_element();
                                            }

                                            let channel_patch =
                                                self.direction.get_patch_for(channel_idx - 1, cx);
                                            let is_patched = channel_patch.is_some_and(|entry| {
                                                entry.channel_idx == device_channel_idx - 1
                                            });

                                            let is_patched_to_selected_device = self
                                                .selected_device_id
                                                .as_ref()
                                                .zip(channel_patch)
                                                .is_some_and(|(selected, d)| {
                                                    selected == &d.device_id
                                                });

                                            div()
                                                .id(SharedString::from(format!(
                                                    "patch-{}-{}",
                                                    channel_idx, device_channel_idx
                                                )))
                                                .row_start(channel_idx as i16 + 1)
                                                .col_start(device_channel_idx as i16 + 1)
                                                .row_span(1)
                                                .col_span(1)
                                                .size_7()
                                                .border_1()
                                                .border_color(cx.theme().secondary)
                                                .bg(
                                                    if is_patched && is_patched_to_selected_device {
                                                        cx.theme().primary
                                                    } else if is_patched {
                                                        cx.theme().secondary
                                                    } else {
                                                        cx.theme().background
                                                    },
                                                )
                                                .cursor(CursorStyle::Crosshair)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.do_patch(
                                                            channel_idx - 1,
                                                            device_channel_idx - 1,
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                ))
                                                .into_any_element()
                                        }),
                                ),
                        ),
                )
            })
    }
}
