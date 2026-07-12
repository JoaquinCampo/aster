use anyhow::Result;
use aster::{effects::*, store::Store};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

struct Spy(Arc<AtomicUsize>);
#[async_trait]
impl EffectAdapter for Spy {
    async fn read_file(&self, _: &Path) -> Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn write_file(&self, _: &Path, _: &[u8]) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn exec(
        &self,
        _: &Path,
        _: &[String],
        _: &BTreeMap<String, String>,
        _: &Path,
    ) -> Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn network(&self, _: &str, _: &[u8]) -> Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn secret(&self, _: &str) -> Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    async fn external(&self, _: &str, _: &str, _: &[u8]) -> Result<Vec<u8>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
}
fn grant(root: PathBuf) -> ScopedGrant {
    ScopedGrant {
        id: Uuid::new_v4(),
        task_id: Uuid::new_v4(),
        capabilities: [
            Capability::FileRead,
            Capability::FileWrite,
            Capability::ProcessExec,
            Capability::Network,
            Capability::SecretRead,
            Capability::External,
        ]
        .into_iter()
        .collect(),
        workspace: root,
        worktrees: vec![],
        executable_allowlist: [PathBuf::from("/bin/echo")].into_iter().collect(),
        network_allowlist: ["api.example.com".into()].into_iter().collect(),
        external_allowlist: ["github".into()].into_iter().collect(),
        secret_destinations: BTreeMap::from([(
            "TOKEN".into(),
            BTreeSet::from(["api.example.com".into()]),
        )]),
        isolation: IsolationProfile {
            filesystem: FilesystemIsolation::WorkspaceReadWrite,
            process: ProcessIsolation::ScrubbedEnvironment,
            network: NetworkIsolation::Allowlisted,
            secrets: SecretIsolation::DestinationScoped,
        },
        expires_at: None,
    }
}

#[test]
fn mcp_destination_is_mediated_by_runtime_effect_broker_policy() {
    use aster::mcp::{EffectBrokerMediator, NetworkDisclosure, NetworkMediator};
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(Arc::new(AtomicUsize::new(0))),
    };
    let grant = grant(d.path().to_owned());
    let mediator = EffectBrokerMediator {
        broker: &broker,
        grant: &grant,
    };
    assert!(
        mediator
            .authorize(&NetworkDisclosure {
                destination: "api.example.com".into(),
                context_classes: vec!["tool arguments".into()],
                operation: "mcp.streamable-http".into()
            })
            .is_ok()
    );
    assert!(
        mediator
            .authorize(&NetworkDisclosure {
                destination: "evil.example".into(),
                context_classes: vec![],
                operation: "mcp.streamable-http".into()
            })
            .is_err()
    );
}

#[tokio::test]
async fn allowed_effect_is_authorized_before_adapter() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x");
    std::fs::write(&f, "x").unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    let op = Uuid::new_v4();
    broker
        .execute(
            op,
            &grant(d.path().to_owned()),
            None,
            EffectRequest::ReadFile { path: f },
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.effect_authorizations(op).unwrap().len(), 1)
}

#[tokio::test]
async fn all_denials_never_reach_adapter() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    let g = grant(d.path().to_owned());
    let requests = vec![
        EffectRequest::ReadFile {
            path: PathBuf::from("/etc/passwd"),
        },
        EffectRequest::WriteFile {
            path: d.path().join("../escape"),
            data: vec![],
        },
        EffectRequest::Exec {
            program: PathBuf::from("/bin/rm"),
            args: vec![],
            env: BTreeMap::new(),
            cwd: d.path().to_owned(),
        },
        EffectRequest::Network {
            destination: "evil.example".into(),
            payload: vec![],
        },
        EffectRequest::Secret {
            name: "TOKEN".into(),
            destination: "evil.example".into(),
        },
        EffectRequest::External {
            service: "slack".into(),
            action: "post".into(),
            payload: vec![],
        },
    ];
    for r in requests {
        assert!(broker.execute(Uuid::new_v4(), &g, None, r).await.is_err())
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0)
}

#[tokio::test]
async fn approval_is_bound_to_exact_task_grant_and_request() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x");
    std::fs::write(&f, "x").unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    let g = grant(d.path().to_owned());
    let request = EffectRequest::ReadFile { path: f };
    let approval = Approval {
        id: Uuid::new_v4(),
        task_id: g.task_id,
        grant_id: g.id,
        request_hash: request_hash(&EffectRequest::Network {
            destination: "api.example.com".into(),
            payload: vec![],
        })
        .unwrap(),
        expires_at: Utc::now() + Duration::minutes(1),
    };
    assert!(
        broker
            .execute(Uuid::new_v4(), &g, Some(&approval), request)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0)
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_is_denied() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    symlink("/etc/passwd", d.path().join("link")).unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    assert!(
        broker
            .execute(
                Uuid::new_v4(),
                &grant(d.path().to_owned()),
                None,
                EffectRequest::ReadFile {
                    path: d.path().join("link")
                }
            )
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0)
}

#[tokio::test]
async fn sensitive_effects_cannot_bypass_explicit_approval() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    let request = EffectRequest::Network {
        destination: "api.example.com".into(),
        payload: b"unapproved".to_vec(),
    };
    assert!(
        broker
            .execute(Uuid::new_v4(), &grant(d.path().to_owned()), None, request)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_digest_and_secret_destination_are_exact() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = EffectBroker {
        store: &store,
        adapter: Spy(calls.clone()),
    };
    let g = grant(d.path().to_owned());
    let original = EffectRequest::Network {
        destination: "api.example.com".into(),
        payload: b"one".to_vec(),
    };
    let approval = Approval::for_request(
        g.task_id,
        g.id,
        &original,
        Utc::now() + Duration::minutes(1),
    )
    .unwrap();
    let mutated = EffectRequest::Network {
        destination: "api.example.com".into(),
        payload: b"two".to_vec(),
    };
    assert!(
        broker
            .execute(Uuid::new_v4(), &g, Some(&approval), mutated)
            .await
            .is_err()
    );
    let secret = EffectRequest::Secret {
        name: "TOKEN".into(),
        destination: "api.example.com".into(),
    };
    let approval =
        Approval::for_request(g.task_id, g.id, &secret, Utc::now() + Duration::minutes(1)).unwrap();
    let redirected = EffectRequest::Secret {
        name: "TOKEN".into(),
        destination: "evil.example".into(),
    };
    assert!(
        broker
            .execute(Uuid::new_v4(), &g, Some(&approval), redirected)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg(unix)]
#[test]
fn denied_mcp_stdio_launch_creates_no_process() {
    use aster::mcp::StdioTransport;
    let d = tempfile::tempdir().unwrap();
    let marker = d.path().join("spawned");
    let store = Store::open(d.path().join("db")).unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let mut g = grant(d.path().to_owned());
    g.capabilities.remove(&Capability::ProcessExec);
    g.executable_allowlist.insert(PathBuf::from("/bin/sh"));
    let args = vec!["-c".into(), format!("touch {}", marker.display())];
    let env = BTreeMap::new();
    let request = EffectRequest::Exec {
        program: PathBuf::from("/bin/sh"),
        args: args.clone(),
        env: env.clone(),
        cwd: d.path().to_owned(),
    };
    let approval =
        Approval::for_request(g.task_id, g.id, &request, Utc::now() + Duration::minutes(1))
            .unwrap();
    assert!(
        StdioTransport::spawn_authorized(
            &broker,
            &g,
            &approval,
            Path::new("/bin/sh"),
            &args,
            &env,
            d.path()
        )
        .is_err()
    );
    assert!(!marker.exists());
    let operations = store.operations_for(g.task_id).unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].state, aster::domain::OperationState::Failed);
    assert!(
        store
            .effect_authorizations(operations[0].id)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn mcp_stdio_approval_is_invalidated_by_argument_or_environment_mutation() {
    use aster::mcp::StdioTransport;
    let d = tempfile::tempdir().unwrap();
    let marker = d.path().join("spawned");
    let store = Store::open(d.path().join("db")).unwrap();
    let broker = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    };
    let mut g = grant(d.path().to_owned());
    g.executable_allowlist.insert(PathBuf::from("/bin/sh"));
    let approved_args = vec!["-c".into(), "exit 0".into()];
    let approved_env = BTreeMap::from([("MODE".into(), "safe".into())]);
    let approved = EffectRequest::Exec {
        program: PathBuf::from("/bin/sh"),
        args: approved_args.clone(),
        env: approved_env.clone(),
        cwd: d.path().to_owned(),
    };
    let approval = Approval::for_request(
        g.task_id,
        g.id,
        &approved,
        Utc::now() + Duration::minutes(1),
    )
    .unwrap();
    let mutated_args = vec!["-c".into(), format!("touch {}", marker.display())];
    assert!(
        StdioTransport::spawn_authorized(
            &broker,
            &g,
            &approval,
            Path::new("/bin/sh"),
            &mutated_args,
            &approved_env,
            d.path()
        )
        .is_err()
    );
    let mutated_env = BTreeMap::from([("MODE".into(), "unsafe".into())]);
    assert!(
        StdioTransport::spawn_authorized(
            &broker,
            &g,
            &approval,
            Path::new("/bin/sh"),
            &approved_args,
            &mutated_env,
            d.path()
        )
        .is_err()
    );
    assert!(!marker.exists());
    assert_eq!(store.operations_for(g.task_id).unwrap().len(), 2);
}

#[tokio::test]
async fn durable_pending_allow_resumes_exact_effect_after_restart() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db");
    let g = grant(d.path().to_owned());
    let request = EffectRequest::WriteFile {
        path: d.path().join("allowed"),
        data: b"exact".to_vec(),
    };
    let store = Store::open(&db).unwrap();
    let pending = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    }
    .request_approval(&g, request, Utc::now() + Duration::minutes(1))
    .unwrap();
    assert!(!d.path().join("allowed").exists());
    drop(store);
    let store = Store::open(&db).unwrap();
    assert_eq!(store.pending_approvals().unwrap(), vec![pending.clone()]);
    EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    }
    .decide_pending(pending.id, true)
    .await
    .unwrap();
    assert_eq!(std::fs::read(d.path().join("allowed")).unwrap(), b"exact");
    assert!(matches!(
        store
            .pending_approval(pending.id)
            .unwrap()
            .unwrap()
            .decision,
        Some(ApprovalDecision::Allowed(_))
    ));
    assert_eq!(
        store
            .operation(pending.operation_id)
            .unwrap()
            .unwrap()
            .state,
        aster::domain::OperationState::Succeeded
    );
}

#[tokio::test]
async fn durable_pending_deny_fails_without_dispatch() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("db")).unwrap();
    let g = grant(d.path().to_owned());
    let target = d.path().join("denied");
    let pending = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    }
    .request_approval(
        &g,
        EffectRequest::WriteFile {
            path: target.clone(),
            data: b"no".to_vec(),
        },
        Utc::now() + Duration::minutes(1),
    )
    .unwrap();
    assert!(
        EffectBroker {
            store: &store,
            adapter: SystemAdapter
        }
        .decide_pending(pending.id, false)
        .await
        .is_err()
    );
    assert!(!target.exists());
    assert!(matches!(
        store
            .pending_approval(pending.id)
            .unwrap()
            .unwrap()
            .decision,
        Some(ApprovalDecision::Denied { .. })
    ));
    assert_eq!(
        store
            .operation(pending.operation_id)
            .unwrap()
            .unwrap()
            .state,
        aster::domain::OperationState::Failed
    );
}

#[tokio::test]
async fn durable_pending_expiry_and_persisted_mutation_fail_closed() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db");
    let store = Store::open(&db).unwrap();
    let g = grant(d.path().to_owned());
    let pending = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    }
    .request_approval(
        &g,
        EffectRequest::WriteFile {
            path: d.path().join("expired"),
            data: vec![1],
        },
        Utc::now() + Duration::milliseconds(10),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(
        EffectBroker {
            store: &store,
            adapter: SystemAdapter
        }
        .decide_pending(pending.id, true)
        .await
        .unwrap_err()
        .to_string()
        .contains("expired")
    );
    assert!(!d.path().join("expired").exists());

    let pending = EffectBroker {
        store: &store,
        adapter: SystemAdapter,
    }
    .request_approval(
        &g,
        EffectRequest::WriteFile {
            path: d.path().join("original"),
            data: vec![1],
        },
        Utc::now() + Duration::minutes(1),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    let mut mutated = pending.clone();
    mutated.request = EffectRequest::WriteFile {
        path: d.path().join("mutated"),
        data: vec![2],
    };
    conn.execute(
        "UPDATE approval_requests SET body=?2 WHERE id=?1",
        rusqlite::params![
            pending.id.to_string(),
            serde_json::to_string(&mutated).unwrap()
        ],
    )
    .unwrap();
    assert!(
        EffectBroker {
            store: &store,
            adapter: SystemAdapter
        }
        .decide_pending(pending.id, true)
        .await
        .unwrap_err()
        .to_string()
        .contains("mutated")
    );
    assert!(!d.path().join("original").exists());
    assert!(!d.path().join("mutated").exists());
}
