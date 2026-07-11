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
