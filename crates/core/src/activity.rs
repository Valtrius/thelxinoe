use std::time::Instant;

/// Cumulative active wall time for playback telemetry, independent of playhead
/// position and speed. Idle, paused and buffering intervals do not contribute.
pub struct ActivityClock {
    observed: Instant,
    active: bool,
    total: f64,
}
impl Default for ActivityClock {
    fn default() -> Self {
        Self {
            observed: Instant::now(),
            active: false,
            total: 0.0,
        }
    }
}
impl ActivityClock {
    fn sample(&mut self) {
        let now = Instant::now();
        if self.active {
            self.total += now.duration_since(self.observed).as_secs_f64().min(30.0);
        }
        self.observed = now;
    }
    pub fn set_active(&mut self, active: bool) -> bool {
        self.sample();
        let changed = self.active != active;
        self.active = active;
        changed
    }
    pub fn seconds(&mut self) -> f64 {
        self.sample();
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn activity_clock_preserves_active_time_but_excludes_pauses_and_suspension() {
        let mut clock = ActivityClock::default();
        clock.observed -= Duration::from_secs(20);
        assert_eq!(clock.seconds(), 0.0);
        clock.set_active(true);
        clock.observed -= Duration::from_secs(2);
        assert!(clock.set_active(false));
        clock.observed -= Duration::from_secs(20);
        assert!((2.0..2.1).contains(&clock.seconds()));
        clock.set_active(true);
        clock.observed -= Duration::from_secs(3600);
        clock.set_active(false);
        assert!((32.0..32.1).contains(&clock.seconds()));
    }
}
