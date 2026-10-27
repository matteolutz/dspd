use gpui_kit::Context;

#[derive(Debug, Copy, Clone, Default)]
pub enum UiTab {
    #[default]
    Strips = 0,
    Rack = 1,
    Patch = 2,
    Setup = 3,
}

impl UiTab {
    pub fn names() -> impl Iterator<Item = (usize, &'static str)> {
        ["Strips", "Rack", "Patch", "Setup"].into_iter().enumerate()
    }
}

impl UiTab {
    pub fn select_tab(&mut self, new_tab: Self, cx: &mut Context<Self>) {
        *self = new_tab;
        cx.notify();
    }

    pub fn select_tab_by_idx(&mut self, tab_idx: usize, cx: &mut Context<Self>) {
        if let Ok(new_tab) = UiTab::try_from(tab_idx) {
            self.select_tab(new_tab, cx);
        }
    }
}

impl Into<usize> for UiTab {
    fn into(self) -> usize {
        self as usize
    }
}

impl TryFrom<usize> for UiTab {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Strips),
            1 => Ok(Self::Rack),
            2 => Ok(Self::Patch),
            3 => Ok(Self::Setup),
            _ => Err(()),
        }
    }
}
