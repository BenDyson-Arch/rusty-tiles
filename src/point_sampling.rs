//! Shared budgeted voxel grid for point-cloud representatives and vector aggregates.
pub(crate) struct VoxelGrid {
    lo: [f64; 3],
    extent: [f64; 3],
    side: usize,
}

impl VoxelGrid {
    pub fn new(lo: [f64; 3], extent: [f64; 3], budget: usize) -> Self {
        assert!(budget > 0);
        let mut side = ((budget as f64).cbrt().round() as usize).max(1);
        while side.checked_pow(3).is_none_or(|n| n > budget) {
            side -= 1;
        }
        Self { lo, extent, side }
    }

    pub fn key(&self, point: [f64; 3]) -> usize {
        let cell: [usize; 3] = std::array::from_fn(|i| {
            if self.extent[i] == 0. {
                0
            } else {
                (((point[i] - self.lo[i]) / self.extent[i] * self.side as f64) as usize)
                    .min(self.side - 1)
            }
        });
        (cell[0] * self.side + cell[1]) * self.side + cell[2]
    }

    pub fn error_bound(&self) -> f64 {
        let cell = self.extent.map(|v| v / self.side as f64);
        cell[0].hypot(cell[1]).hypot(cell[2])
    }
}
