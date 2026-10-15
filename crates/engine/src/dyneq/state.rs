#[derive(Debug, Clone, Copy)]
pub struct Coeffs {
    // Denominator is normalized so a0 = 1.
    pub(super) b0: f32,
    pub(super) b1: f32,
    pub(super) b2: f32,
    pub(super) a1: f32,
    pub(super) a2: f32,
}

impl Coeffs {
    pub(super) const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    pub(super) x1: f32,
    pub(super) x2: f32,
    pub(super) y1: f32,
    pub(super) y2: f32,
}

impl BiquadState {
    #[inline]
    pub fn tick(&mut self, x: f32, c: Coeffs) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;

        y
    }
}
