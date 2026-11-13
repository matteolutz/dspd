use gpui_kit::{App, PromptButton, PromptLevel, Window};
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
}
