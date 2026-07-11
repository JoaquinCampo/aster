use aster::{
    domain::Task,
    provider::{
        FakePiAdapter, OpenAiResponsesProvider, PiAdapter, Provider, ProviderRequest,
        ReasoningEffort,
    },
    routing::Router,
    store::Store,
};
use futures_util::StreamExt;
use reqwest::Url;
use std::{net::TcpListener, time::Duration};

#[tokio::test]
async fn routine_harness_operations_make_no_network_connection() {
    let deny = TcpListener::bind("127.0.0.1:0").unwrap();
    deny.set_nonblocking(true).unwrap();

    let store = Store::open(":memory:").unwrap();
    let task = Task::new(
        "local-only routine operation".into(),
        Router::default().route("local-only routine operation"),
    );
    store.save_task(&task).unwrap();
    let adapter = FakePiAdapter;
    adapter.execute(&task.prompt, &task.route).await.unwrap();
    assert_eq!(store.tasks().unwrap().len(), 1);

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        deny.accept().is_err(),
        "routine local operation opened a socket"
    );
}

#[tokio::test]
async fn configured_provider_connects_only_to_disclosed_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, peer) = listener.accept().unwrap();
        use std::io::{Read, Write};
        let mut request = vec![0; 4096];
        let n = socket.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..n]);
        assert!(request.starts_with("POST /v1/responses HTTP/1.1"));
        socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 14\r\n\r\ndata: [DONE]\n\n").unwrap();
        peer
    });
    let endpoint = Url::parse(&format!("http://{addr}/v1/responses")).unwrap();
    let provider = OpenAiResponsesProvider::new(endpoint, None);
    let disclosure = provider.network_disclosure().unwrap();
    assert_eq!(disclosure.destination, format!("http://{addr}"));
    assert_eq!(
        disclosure.classification,
        "task_communication_not_product_telemetry"
    );
    assert_eq!(
        disclosure.context,
        [
            "model identifier",
            "task prompt/context",
            "reasoning effort"
        ]
    );
    let audit_store = Store::open(":memory:").unwrap();
    let audit_task = Task::new(
        "disclosure owner".into(),
        Router::default().route("disclosure owner"),
    );
    audit_store.save_task(&audit_task).unwrap();
    audit_store
        .append(&disclosure.audit_event(audit_task.id))
        .unwrap();
    let event = audit_store.audit_for(audit_task.id).unwrap().pop().unwrap();
    assert_eq!(event.kind, "network.destination_disclosed");
    assert!(event.detail.contains(&format!("destination=http://{addr}")));
    assert!(
        event
            .detail
            .contains("classification=task_communication_not_product_telemetry")
    );

    let mut stream = provider
        .stream(ProviderRequest {
            model: "fixture-model".into(),
            prompt: "explicit task context".into(),
            effort: ReasoningEffort::Low,
        })
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    let peer = server.join().unwrap();
    assert_eq!(peer.ip(), addr.ip());
}
