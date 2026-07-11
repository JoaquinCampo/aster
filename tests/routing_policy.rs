use aster::domain::{Effort, Role};
use aster::routing::{Router, RoutingRequest, UserOverrides, built_in_roles};

fn request(prompt: &str, quality: u8, tokens: u32) -> RoutingRequest {
    RoutingRequest {
        prompt: prompt.into(),
        required_quality: quality,
        estimated_tokens: tokens,
        overrides: UserOverrides::default(),
    }
}

#[test]
fn cheapest_reliable_selection_is_deterministic_and_auditable() {
    let router = Router::default();
    let decision = router.decide(request("implement typed routing", 80, 10_000));
    assert_eq!(decision.route.model, "fake-terra");
    assert_eq!(decision.route.role, "implementer");
    assert_eq!(decision.evidence.len(), 3);
    assert!(!decision.evidence[0].reliable);
    assert_eq!(
        decision.route.decision_id,
        "v1:implementer:fake-terra:80:10000"
    );
}

#[test]
fn dimensions_and_overrides_are_independent_and_budgeted() {
    let router = Router::default();
    let mut r = request("small task", 50, 2_000);
    r.overrides = UserOverrides {
        role: Some(Role::Reviewer),
        model: Some("fixed-strong".into()),
        effort: Some(Effort::High),
        context_tokens: Some(6_000),
        max_cost_micros: Some(0),
        max_latency_ms: Some(100),
    };
    let d = router.decide(r);
    assert_eq!(d.route.role, "reviewer");
    assert_eq!(d.route.model, "fixed-strong");
    assert_eq!(d.route.dimensions.effort, Effort::High);
    assert_eq!(d.route.dimensions.context_tokens, 6_000);
    assert!(
        d.evidence
            .iter()
            .any(|e| e.model == "fixed-strong" && !e.reliable)
    );
}

#[test]
fn escalation_and_deescalation_require_evidence() {
    assert_eq!(Router::adapt_quality(70, &[true, false]), 80);
    assert_eq!(Router::adapt_quality(70, &[true, true, true]), 60);
    assert_eq!(Router::adapt_quality(70, &[true, true]), 70);
}

#[test]
fn roles_and_delegation_benefit_are_explicit() {
    assert_eq!(built_in_roles().len(), 5);
    let router = Router::default();
    let benefit = router.delegation_benefit(&request(&"x".repeat(121), 70, 4_000));
    assert!(benefit.delegated);
    assert!(benefit.expected_quality_gain > 0);
    assert!(benefit.context_savings_tokens > 0);
}

#[test]
fn deterministic_benchmark_against_fixed_strong_baseline() {
    let router = Router::default();
    let scenarios = [
        ("answer briefly", 50, 1_000),
        ("implement a repository refactor", 80, 20_000),
        ("review critical security design", 95, 60_000),
    ];
    let selected: Vec<_> = scenarios
        .into_iter()
        .map(|(p, q, t)| router.decide(request(p, q, t)).route.model)
        .collect();
    assert_eq!(selected, ["fake-luna", "fake-terra", "fixed-strong"]);
    // The fixed baseline is quality-dominant; routing preserves required quality while
    // reducing deterministic profile cost/latency/context consumption where reliable.
    assert_eq!(
        router
            .models
            .iter()
            .find(|m| m.id == "fixed-strong")
            .unwrap()
            .quality,
        95
    );
}
