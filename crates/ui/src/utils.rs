pub fn map_range(value: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    let normalized = ((value - in_min) / (in_max - in_min)).clamp(0.0, 1.0);
    out_min + normalized * (out_max - out_min)
}
