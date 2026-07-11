use aster::{
    orchestration::DelegationPolicy,
    provider::FakePiAdapter,
    runtime::Runtime,
    store::Store,
    verification::{DurableEvidence, VerificationOwnerRole, VerificationStatus},
    workflow::{Risk, VerificationPolicy},
};

#[tokio::test]
async fn normalized_evidence_survives_restart_and_payload_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let runtime = Runtime::new(Store::open(&path).unwrap(), FakePiAdapter);
    let run = runtime
        .run_maker_checker_fixer(
            "durable evidence",
            VerificationPolicy::proportional(Risk::Medium),
            DelegationPolicy::default(),
        )
        .await
        .unwrap();
    drop(runtime);

    let reopened = Store::open(&path).unwrap();
    let mut total = 0;
    let mut checker = false;
    for task_id in &run.task_ids {
        for record in reopened.verification_runs_for(*task_id).unwrap() {
            assert_eq!(record.task_id, *task_id);
            assert!(!record.policy.is_empty());
            assert!(!record.command_identity.is_empty());
            assert!(record.completed_at.is_some());
            assert_eq!(record.outcome, VerificationStatus::Passed);
            checker |= matches!(
                record.owner_role,
                VerificationOwnerRole::DeterministicChecker
                    | VerificationOwnerRole::IndependentChecker
            );
            let evidence = reopened.verification_evidence_for(record.id).unwrap();
            assert!(
                evidence
                    .iter()
                    .all(|item| item.digest.starts_with("sha256:"))
            );
            total += 1;
        }
    }
    assert_eq!(total, run.task_ids.len());
    assert!(checker);

    let victim = run.task_ids[0];
    reopened.delete_task_payloads(victim).unwrap();
    let after = Store::open(&path).unwrap();
    for record in after.verification_runs_for(victim).unwrap() {
        assert!(
            after
                .verification_evidence_for(record.id)
                .unwrap()
                .iter()
                .all(|item| item.payload_ref.is_none())
        );
    }
}

#[test]
fn migration_is_idempotent_and_rejects_orphan_or_malformed_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migration.db");
    Store::open(&path).unwrap();
    let store = Store::open(&path).unwrap();
    let orphan = DurableEvidence {
        id: uuid::Uuid::new_v4(),
        run_id: uuid::Uuid::new_v4(),
        kind: "stdout".into(),
        payload_ref: None,
        digest: "sha256:not-a-digest".into(),
        media_type: "text/plain".into(),
        size: 0,
        created_at: chrono::Utc::now(),
    };
    assert!(store.save_verification_evidence(&orphan).is_err());
}
