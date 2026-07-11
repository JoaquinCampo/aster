use aster::{
    verification::{CheckEvidence, VerificationStatus, digest},
    workflow::*,
};
use uuid::Uuid;

fn check(status: VerificationStatus) -> CheckEvidence {
    CheckEvidence {
        check: "test".into(),
        status,
        exit_code: Some(if status == VerificationStatus::Passed {
            0
        } else {
            1
        }),
        stdout: b"out".to_vec(),
        stderr: vec![],
        stdout_digest: digest(b"out"),
        stderr_digest: digest(b""),
        artifacts: vec![],
        detail: None,
    }
}

#[test]
fn proportional_templates_have_independent_parallel_checkers_and_bounded_fixers() {
    let dag = MakerCheckerFixerDag::template(VerificationPolicy::proportional(Risk::High)).unwrap();
    assert_eq!(
        dag.nodes
            .iter()
            .filter(|n| n.role == DagRole::IndependentChecker)
            .count(),
        2
    );
    assert_eq!(
        dag.nodes
            .iter()
            .filter(|n| n.role == DagRole::Fixer)
            .count(),
        0
    );
    let failed: Vec<_> = dag
        .nodes
        .iter()
        .filter(|n| n.role == DagRole::IndependentChecker)
        .map(|n| n.id)
        .collect();
    let mut conditional = dag.clone();
    conditional.append_fixer_round(&failed).unwrap();
    assert_eq!(
        conditional
            .nodes
            .iter()
            .filter(|n| n.role == DagRole::Fixer)
            .count(),
        1
    );
    assert!(conditional.append_fixer_round(&[]).is_err());
    assert!(
        dag.nodes
            .iter()
            .filter(|n| matches!(
                n.role,
                DagRole::DeterministicChecker | DagRole::IndependentChecker
            ))
            .all(|n| n.dependencies == vec![dag.maker])
    );
    let actors = [Uuid::new_v4(), Uuid::new_v4()];
    dag.validate_checker_attempts(&actors).unwrap();
    assert!(
        dag.validate_checker_attempts(&[actors[0], actors[0]])
            .is_err()
    );
    assert!(
        dag.validate_checker_attempts(&[dag.maker, actors[0]])
            .is_err()
    );
}

#[test]
fn policy_rejects_under_verification_and_unbounded_loops() {
    assert!(
        VerificationPolicy {
            risk: Risk::High,
            deterministic_checks: 1,
            independent_checkers: 2,
            max_fixer_rounds: 3
        }
        .validate()
        .is_err()
    );
    assert!(
        VerificationPolicy {
            risk: Risk::Low,
            deterministic_checks: 1,
            independent_checkers: 0,
            max_fixer_rounds: 11
        }
        .validate()
        .is_err()
    );
}

#[test]
fn final_evidence_preserves_non_pass_outcomes_and_bounds() {
    let policy = VerificationPolicy::proportional(Risk::Medium);
    let review = ReviewEvidence {
        checker_id: Uuid::new_v4(),
        attempt: 1,
        status: VerificationStatus::Passed,
        rationale: "ok".into(),
    };
    let final_result = assemble_final(
        &policy,
        vec![check(VerificationStatus::TimedOut)],
        vec![review.clone()],
        1,
    )
    .unwrap();
    assert_eq!(final_result.status, VerificationStatus::TimedOut);
    assert!(
        assemble_final(
            &policy,
            vec![check(VerificationStatus::Passed)],
            vec![review],
            3
        )
        .is_err()
    );
}

#[test]
fn high_risk_gate_stalls_and_handoffs_are_explicit() {
    let policy = VerificationPolicy::proportional(Risk::High);
    let reviews = (0..2)
        .map(|_| ReviewEvidence {
            checker_id: Uuid::new_v4(),
            attempt: 1,
            status: VerificationStatus::Passed,
            rationale: "independent pass".into(),
        })
        .collect::<Vec<_>>();
    assert!(
        policy
            .gate(
                &[
                    check(VerificationStatus::Passed),
                    check(VerificationStatus::Failed)
                ],
                &reviews
            )
            .is_err()
    );
    let handoff = Handoff {
        objective: "repair".into(),
        summary: "same failed check".into(),
        constraints: vec!["do not weaken gate".into()],
        decisions: vec![],
        open_issues: vec!["failure".into()],
        artifacts: vec![],
    };
    let mut detector = ProgressDetector::new(3).unwrap();
    let task = Uuid::new_v4();
    assert!(
        detector
            .observe(task, "same", handoff.clone())
            .unwrap()
            .is_none()
    );
    assert!(
        detector
            .observe(task, "same", handoff.clone())
            .unwrap()
            .is_none()
    );
    let evidence = detector.observe(task, "same", handoff).unwrap().unwrap();
    assert_eq!(evidence.observations, 3);
    assert!(evidence.reason.contains("stalled"));
}

#[test]
fn all_terminal_check_states_remain_distinct() {
    let values = [
        VerificationStatus::Passed,
        VerificationStatus::Failed,
        VerificationStatus::Inconclusive,
        VerificationStatus::Cancelled,
        VerificationStatus::TimedOut,
    ];
    let json = serde_json::to_string(&values).unwrap();
    let roundtrip: Vec<VerificationStatus> = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtrip, values);
}
