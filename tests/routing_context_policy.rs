use aster::{context::*, routing_policy::*, store::Store};
use std::{collections::BTreeMap, path::PathBuf};

fn item(category: &str, content: &str, tokens: u32) -> ContextItem {
    ContextItem {
        category: category.into(),
        content: content.into(),
        estimated_tokens: tokens,
        provenance: Provenance {
            path: PathBuf::from(content),
            ecosystem: "test".into(),
            trust: Trust::TrustedInstruction,
        },
    }
}

#[test]
fn versioned_policy_loads_and_learning_requires_reviewed_revision() {
    let policy = RoutingPolicy::load("config/routing-policy-v1.toml").unwrap();
    let outcomes = vec![OutcomeAggregate {
        policy_revision: 1,
        role: aster::domain::Role::Implementer,
        model: "fake-terra".into(),
        attempts: 4,
        verified_successes: 3,
        failures: 1,
        total_cost_micros: 20,
        total_latency_ms: 40,
    }];
    let mut rec = recommend(&policy, &outcomes).remove(0);
    assert!(!rec.reviewed);
    assert!(
        apply_reviewed_revision(
            &policy,
            RoutingPolicy {
                revision: 2,
                ..policy.clone()
            },
            &rec
        )
        .is_err()
    );
    rec.reviewed = true;
    assert!(
        apply_reviewed_revision_with_history(
            &policy,
            RoutingPolicy {
                revision: 2,
                ..policy.clone()
            },
            &PolicyRecommendation {
                evidence_attempts: 3,
                ..rec.clone()
            },
            &outcomes,
        )
        .is_err()
    );
    assert_eq!(
        apply_reviewed_revision_with_history(
            &policy,
            RoutingPolicy {
                revision: 2,
                ..policy.clone()
            },
            &rec,
            &outcomes,
        )
        .unwrap()
        .revision,
        2
    );
    assert_eq!(
        apply_reviewed_revision(
            &policy,
            RoutingPolicy {
                revision: 2,
                ..policy.clone()
            },
            &rec
        )
        .unwrap()
        .revision,
        2
    );
}

#[test]
fn outcomes_and_advisory_recommendations_persist_without_policy_mutation() {
    let store = Store::open(":memory:").unwrap();
    let outcome = OutcomeAggregate {
        policy_revision: 1,
        role: aster::domain::Role::Verifier,
        model: "fixed-strong".into(),
        attempts: 3,
        verified_successes: 3,
        failures: 0,
        total_cost_micros: 9,
        total_latency_ms: 12,
    };
    store.save_routing_outcome(&outcome).unwrap();
    let policy = RoutingPolicy::load("config/routing-policy-v1.toml").unwrap();
    let rec = recommend(&policy, &store.routing_outcomes(1).unwrap()).remove(0);
    store.save_routing_recommendation(&rec).unwrap();
    assert_eq!(
        RoutingPolicy::load("config/routing-policy-v1.toml")
            .unwrap()
            .revision,
        1
    );
    assert!(!store.routing_recommendations().unwrap()[0].reviewed);
}

#[test]
fn relevance_freshness_category_budgets_dedup_and_critical_protection() {
    let mut budgets = BTreeMap::new();
    budgets.insert("rules".into(), 6);
    let candidates = vec![
        RetrievalCandidate {
            item: item("rules", "critical", 4),
            relevance: 1.0,
            content_version: "1".into(),
            fresh: true,
            critical: true,
        },
        RetrievalCandidate {
            item: item("rules", "critical", 4),
            relevance: 0.9,
            content_version: "1".into(),
            fresh: true,
            critical: false,
        },
        RetrievalCandidate {
            item: item("rules", "stale", 2),
            relevance: 0.8,
            content_version: "old".into(),
            fresh: false,
            critical: false,
        },
        RetrievalCandidate {
            item: item("code", "relevant", 3),
            relevance: 0.7,
            content_version: "1".into(),
            fresh: true,
            critical: false,
        },
    ];
    let result = retrieve_relevant(8, &budgets, candidates).unwrap();
    assert_eq!(result.manifest.items.len(), 2);
    assert_eq!(result.metrics.duplicate_tokens_avoided, 4);
    assert_eq!(result.metrics.stale_items_invalidated, 1);
    let impossible = vec![RetrievalCandidate {
        item: item("rules", "must", 9),
        relevance: 1.0,
        content_version: "1".into(),
        fresh: true,
        critical: true,
    }];
    assert!(
        retrieve_relevant(8, &budgets, impossible)
            .unwrap_err()
            .to_string()
            .contains("critical constraint")
    );
}

#[test]
fn all_nine_dimensions_are_selected_and_serialized_for_audit() {
    let decision = aster::routing::Router::default()
        .decide(aster::routing::RoutingRequest {
            prompt: "implement feature".into(),
            required_quality: 70,
            estimated_tokens: 1000,
            overrides: Default::default(),
        })
        .unwrap();
    let value = serde_json::to_value(&decision.route).unwrap();
    let dimensions = value.get("dimensions").unwrap();
    for key in [
        "effort",
        "context_tokens",
        "output_tokens",
        "max_latency_ms",
        "capabilities",
        "tools",
        "isolation",
        "lifecycle",
        "verification",
    ] {
        assert!(dimensions.get(key).is_some(), "missing {key}");
    }
}
