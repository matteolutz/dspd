use gpui_kit::{
    px, AbsoluteLength, App, Font, Hsla, Pixels, Point, PromptButton, PromptLevel, SharedString,
    TextAlign, TextRun, Window,
};
use itertools::Itertools;

pub trait WindowExt {
    fn spawn_detached_prompt<T>(
        &self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[T],
        cx: &mut App,
        on_answer: impl FnOnce(usize, &mut Window, &mut App) + 'static,
    ) where
        T: Clone + Into<PromptButton>;

    fn paint_simple_text_line(
        &mut self,
        text: impl Into<SharedString>,
        font: Font,
        font_size: impl Into<AbsoluteLength>,
        color: impl Into<Hsla>,
        origin: Point<Pixels>,
        align: TextAlign,
        cx: &mut App,
    );
}

impl WindowExt for Window {
    fn spawn_detached_prompt<T>(
        &self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[T],
        cx: &mut App,
        on_answer: impl FnOnce(usize, &mut Window, &mut App) + 'static,
    ) where
        T: Clone + Into<PromptButton>,
    {
        let message = message.to_string();
        let detail = detail.map(String::from);
        let answers = answers
            .iter()
            .cloned()
            .map_into::<PromptButton>()
            .collect::<Vec<_>>();

        self.spawn(cx, async move |cx| {
            let Ok(receiver) = cx.update(|window, cx| {
                window.prompt(level, &message, detail.as_deref(), &answers, cx)
            }) else {
                return;
            };

            let Ok(answer) = receiver.await else {
                return;
            };

            _ = cx.update(move |window, cx| on_answer(answer, window, cx));
        })
        .detach();
    }

    fn paint_simple_text_line(
        &mut self,
        text: impl Into<SharedString>,
        font: Font,
        font_size: impl Into<AbsoluteLength>,
        color: impl Into<Hsla>,
        origin: Point<Pixels>,
        align: TextAlign,
        cx: &mut App,
    ) {
        let text = text.into();
        let len = text.len();
        let font_size = font_size.into().to_pixels(self.rem_size());
        let color = color.into();

        let line = self.text_system().shape_line(
            text,
            font_size,
            &[TextRun {
                len,
                font,
                color,
                ..Default::default()
            }],
            None,
        );

        _ = line.paint(origin, px(1.0), align, Some(px(0.0)), self, cx);
    }
}
