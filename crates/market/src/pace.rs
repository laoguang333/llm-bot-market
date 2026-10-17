//! A lagging account only loses decisions after sustained expensive bot calls.
//! The eight-call window tolerates startup work and isolated OS scheduling pauses.

#[derive(Default)]
pub(crate) struct DecisionPace {
    samples: u8,
    over_budget: u8,
}

impl DecisionPace {
    pub fn record(&mut self, nanos: u64, budget_us: u64) {
        self.samples = self.samples.saturating_add(1).min(8);
        self.over_budget =
            (self.over_budget << 1) | u8::from(nanos > budget_us.saturating_mul(1000));
    }

    pub fn is_slow(&self) -> bool {
        self.samples == 8 && self.over_budget.count_ones() >= 6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_and_isolated_stalls_get_grace_but_sustained_work_does_not() {
        let mut pace = DecisionPace::default();
        for _ in 0..7 {
            pace.record(10_000_000, 2000);
            assert!(!pace.is_slow(), "observe eight calls before judging a bot");
        }
        pace.record(10_000_000, 2000);
        assert!(pace.is_slow());
        for _ in 0..3 {
            pace.record(100_000, 2000);
        }
        assert!(!pace.is_slow(), "a recovering bot gets decisions back");
        for i in 0..64 {
            pace.record(if i % 8 == 0 { 100_000_000 } else { 100_000 }, 2000);
        }
        assert!(
            !pace.is_slow(),
            "one OS stall per window is not a slow strategy"
        );
    }
}
