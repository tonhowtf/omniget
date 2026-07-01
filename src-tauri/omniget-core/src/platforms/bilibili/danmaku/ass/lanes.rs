#[derive(Debug, Clone, Copy)]
pub struct ScrollLane {
    pub free_at_secs: f64,
}

pub struct ScrollLaneSet {
    pub lanes: Vec<ScrollLane>,
}

impl ScrollLaneSet {
    pub fn new(count: usize) -> Self {
        Self {
            lanes: vec![ScrollLane { free_at_secs: 0.0 }; count],
        }
    }

    pub fn allocate(&mut self, start_secs: f64, exit_secs: f64) -> usize {
        let mut best = 0usize;
        let mut best_free_at = f64::MAX;
        for (i, lane) in self.lanes.iter().enumerate() {
            if lane.free_at_secs <= start_secs {
                self.lanes[i].free_at_secs = exit_secs;
                return i;
            }
            if lane.free_at_secs < best_free_at {
                best_free_at = lane.free_at_secs;
                best = i;
            }
        }
        self.lanes[best].free_at_secs = exit_secs;
        best
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StaticLane {
    pub free_at_secs: f64,
}

pub struct StaticLaneSet {
    pub lanes: Vec<StaticLane>,
}

impl StaticLaneSet {
    pub fn new(count: usize) -> Self {
        Self {
            lanes: vec![StaticLane { free_at_secs: 0.0 }; count],
        }
    }

    pub fn allocate(&mut self, start_secs: f64, end_secs: f64) -> usize {
        let mut best = 0usize;
        let mut best_free_at = f64::MAX;
        for (i, lane) in self.lanes.iter().enumerate() {
            if lane.free_at_secs <= start_secs {
                self.lanes[i].free_at_secs = end_secs;
                return i;
            }
            if lane.free_at_secs < best_free_at {
                best_free_at = lane.free_at_secs;
                best = i;
            }
        }
        self.lanes[best].free_at_secs = end_secs;
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_lane_reuses_freed_slot() {
        let mut lanes = ScrollLaneSet::new(3);
        let a = lanes.allocate(0.0, 5.0);
        let b = lanes.allocate(0.5, 5.5);
        let c = lanes.allocate(1.0, 6.0);
        let d = lanes.allocate(6.0, 11.0);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c, 2);
        assert_eq!(d, 0);
    }

    #[test]
    fn static_lane_picks_earliest_free() {
        let mut lanes = StaticLaneSet::new(2);
        let a = lanes.allocate(0.0, 4.0);
        let b = lanes.allocate(2.0, 6.0);
        let c = lanes.allocate(4.5, 8.0);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c, 0);
    }
}
