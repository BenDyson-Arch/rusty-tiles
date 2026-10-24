//! Conservative tile-frame box of admitted source bounds.
#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}
impl Bounds {
    pub fn box_values(self) -> [f64; 12] {
        let center = std::array::from_fn::<_, 3, _>(|i| (self.min[i] + self.max[i]) / 2.0);
        let half = std::array::from_fn::<_, 3, _>(|i| {
            if self.min[i] == self.max[i] {
                return 0.0;
            }
            // Round outward: midpoint/subtraction rounding must not put a
            // stored vertex just outside its emitted box at mixed magnitudes.
            (center[i] - self.min[i])
                .max(self.max[i] - center[i])
                .next_up()
        });
        [
            center[0], center[1], center[2], half[0], 0.0, 0.0, 0.0, half[1], 0.0, 0.0, 0.0,
            half[2],
        ]
    }
}
