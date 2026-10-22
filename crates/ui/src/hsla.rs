use gpui_kit::Hsla;

pub trait HslaExt {
    fn with_luminance(self, l: f32) -> Hsla;
}

impl HslaExt for Hsla {
    fn with_luminance(mut self, l: f32) -> Hsla {
        self.l = (self.l + l).min(1.0);
        self
    }
}
