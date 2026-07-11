use aster::domain::{Effort, Role};
use aster::routing::{ConstraintKind, Router, RoutingRequest, UserOverrides, built_in_roles};

fn request(prompt: &str, quality: u8, tokens: u32) -> RoutingRequest {
    RoutingRequest {
        prompt: prompt.into(),
        required_quality: quality,
        estimated_tokens: tokens,
        overrides: UserOverrides::default(),
    }
}

#[test]
fn cheapest_eligible_selection_is_deterministic_and_auditable() {
    let decision = Router::default()
        .decide(request("implement typed routing", 80, 10_000))
        .unwrap();
    assert_eq!(decision.route.model, "fake-terra");
    assert_eq!(decision.route.role, "implementer");
    assert!(
        decision
            .route
            .rationale
            .contains("no persisted outcome history")
    );
    assert_eq!(
        decision.route.decision_id,
        "v1:implementer:fake-terra:80:10000"
    );
    assert!(decision.evidence.iter().all(|e| !e.constraints.is_empty()));
}

#[test]
fn safe_overrides_remain_independent() {
    let mut r = request("small task", 50, 2_000);
    r.overrides = UserOverrides {
        role: Some(Role::Reviewer),
        model: Some("fixed-strong".into()),
        effort: Some(Effort::High),
        context_tokens: Some(6_000),
        max_cost_micros: None,
        max_latency_ms: None,
        ..UserOverrides::default()
    };
    let d = Router::default().decide(r).unwrap();
    assert_eq!(d.route.role, "reviewer");
    assert_eq!(d.route.model, "fixed-strong");
    assert_eq!(d.route.dimensions.effort, Effort::High);
    assert_eq!(d.route.dimensions.context_tokens, 6_000);
}

#[test]
fn unknown_and_constraint_violating_overrides_return_typed_failure() {
    let router = Router::default();
    let mut unknown = request("small", 50, 2_000);
    unknown.overrides.model = Some("invented".into());
    let error = router.decide(unknown).unwrap_err();
    assert!(error.reason.contains("unknown model override"));
    assert_eq!(error.evidence.len(), 3);

    let mut unsafe_override = request("critical", 90, 2_000);
    unsafe_override.overrides.model = Some("fake-luna".into());
    let error = router.decide(unsafe_override).unwrap_err();
    assert!(error.reason.contains("violates hard constraints"));
    let luna = error
        .evidence
        .iter()
        .find(|e| e.model == "fake-luna")
        .unwrap();
    assert!(
        luna.constraints
            .iter()
            .any(|c| c.kind == ConstraintKind::Hard && !c.satisfied)
    );
}

#[test]
fn no_candidate_never_falls_back_by_violating_constraints() {
    let mut r = request("huge", 100, 200_000);
    r.overrides.max_cost_micros = Some(0);
    let error = Router::default().decide(r).unwrap_err();
    assert!(error.reason.contains("all candidates"));
    assert!(error.evidence.iter().all(|e| !e.reliable));
}

#[test]
fn roles_have_all_nine_complete_contracts_and_no_model_or_effort_binding() {
    let roles = built_in_roles();
    assert_eq!(roles.len(), 9);
    for role in roles {
        assert!(!role.purpose.is_empty());
        assert!(!role.boundaries.is_empty());
        assert!(!role.expected_inputs.is_empty());
        assert!(!role.expected_outputs.is_empty());
        assert!(!role.default_context_policy.is_empty());
        assert!(!role.default_capabilities.is_empty());
        assert!(!role.allowed_tools.is_empty());
        assert!(!role.verification.is_empty());
        assert!(!role.fallback.is_empty());
        assert!(!role.isolation.is_empty());
        assert!(!role.completion.is_empty());
        let serialized = serde_json::to_value(role).unwrap();
        assert!(serialized.get("model").is_none());
        assert!(serialized.get("effort").is_none());
    }
}

#[test]
fn escalation_and_deescalation_require_evidence() {
    assert_eq!(Router::adapt_quality(70, &[true, false]), 80);
    assert_eq!(Router::adapt_quality(70, &[true, true, true]), 60);
    assert_eq!(Router::adapt_quality(70, &[true, true]), 70);
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
        .map(|(p, q, t)| router.decide(request(p, q, t)).unwrap().route.model)
        .collect();
    assert_eq!(selected, ["fake-luna", "fake-terra", "fixed-strong"]);
}
