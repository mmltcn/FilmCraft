//! Opt-in process-wide CPU stage wall times for headless profiling.
//! Nested/parallel calls overlap: these totals are not additive frame latency.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
static ENABLED: AtomicBool = AtomicBool::new(false);
static NS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
#[derive(Clone, Copy)]
pub enum Stage {
    SourceConversion,
    Motion,
    Blur,
    Lumetri,
    OtherEffect,
}
impl Stage {
    pub const ALL: [Self; 5] = [Self::SourceConversion, Self::Motion, Self::Blur, Self::Lumetri, Self::OtherEffect];
    pub fn name(self) -> &'static str {
        match self {
            Self::SourceConversion => "source_conversion_scale",
            Self::Motion => "motion",
            Self::Blur => "blur",
            Self::Lumetri => "lumetri",
            Self::OtherEffect => "other_effect",
        }
    }
}
pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
pub fn reset() {
    for n in &NS {
        n.store(0, Ordering::Relaxed);
    }
}
pub fn get() -> Vec<(&'static str, Duration)> {
    Stage::ALL.iter().map(|s| (s.name(), Duration::from_nanos(NS.get(*s as usize).map_or(0, |n| n.load(Ordering::Relaxed))))).collect()
}
pub(crate) struct Guard(Option<(Stage, Instant)>);
pub(crate) fn start(stage: Stage) -> Guard {
    Guard(ENABLED.load(Ordering::Relaxed).then(|| (stage, Instant::now())))
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some((stage, t)) = self.0
            && let Some(n) = NS.get(stage as usize)
        {
            n.fetch_add(u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
        }
    }
}
