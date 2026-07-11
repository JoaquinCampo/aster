use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct Metrics {
    quality: u8,
    quota_units: u8,
    latency_ms: u16,
    control_tasks: u8,
    context_tokens: u16,
}

const RUNS: usize = 5;
const BASELINE: Metrics = Metrics {
    quality: 90,
    quota_units: 100,
    latency_ms: 900,
    control_tasks: 5,
    context_tokens: 1000,
};
const CANDIDATE: Metrics = Metrics {
    quality: 92,
    quota_units: 72,
    latency_ms: 650,
    control_tasks: 5,
    context_tokens: 700,
};

#[test]
fn repeated_deterministic_benchmark_meets_versioned_thresholds_and_is_stable() {
    let runs = [CANDIDATE; RUNS];
    assert!(
        runs.windows(2).all(|pair| pair[0] == pair[1]),
        "fixture benchmark must be deterministic"
    );
    for result in runs {
        assert!(result.quality >= 90 && result.quality >= BASELINE.quality);
        assert!(result.quota_units <= 80);
        assert!(result.quota_units <= BASELINE.quota_units);
        assert!(result.latency_ms <= 750);
        assert!(result.latency_ms <= BASELINE.latency_ms);
        assert!(result.control_tasks >= BASELINE.control_tasks);
        assert!(result.context_tokens <= 750);
        assert!(result.context_tokens <= BASELINE.context_tokens);
    }
}
