use aster::provider::{
    CodexBridgeProvider, DeterministicFakeProvider, OpenAiResponsesProvider, Provider,
    ProviderError, ProviderEvent, ProviderRequest, ReasoningEffort, Usage, XaiProvider,
};
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    response::Response,
    routing::post,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;

async fn server(app: Router) -> reqwest::Url {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}/v1/responses").parse().unwrap()
}
fn request(model: &str, effort: ReasoningEffort) -> ProviderRequest {
    ProviderRequest {
        model: model.into(),
        prompt: "hello".into(),
        effort,
    }
}
async fn sse(_: Request) -> Response {
    let body = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"why\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"call_id\":\"c1\",\"name\":\"tool\",\"delta\":\"{}\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"total_tokens\":5}}}\n\n",
        "data: [DONE]\n\n"
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn parses_fragment_safe_response_events_and_usage() {
    let provider = OpenAiResponsesProvider::new(
        server(Router::new().route("/v1/responses", post(sse))).await,
        None,
    );
    let events: Vec<_> = provider
        .stream(request("vendor-model-1", ReasoningEffort::High))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(events[0], ProviderEvent::OutputDelta("hi".into()));
    assert!(
        matches!(&events[2], ProviderEvent::ToolCallDelta { call_id: Some(id), name: Some(name), .. } if id == "c1" && name == "tool")
    );
    assert_eq!(
        events[3],
        ProviderEvent::Completed(Usage {
            input_tokens: Some(2),
            output_tokens: Some(3),
            total_tokens: Some(5)
        })
    );
}

async fn http_error(_: Request) -> Response {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"error":{"code":"rate_limit","message":"slow down"}}).to_string(),
        ))
        .unwrap()
}
#[tokio::test]
async fn preserves_structured_http_errors() {
    let p = OpenAiResponsesProvider::new(
        server(Router::new().route("/v1/responses", post(http_error))).await,
        None,
    );
    let e = match p
        .stream(request("vendor-model-1", ReasoningEffort::Low))
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("expected HTTP error"),
    };
    assert!(
        matches!(e, ProviderError::Http { status: StatusCode::TOO_MANY_REQUESTS, ref code, .. } if code == "rate_limit")
    );
}

async fn failed(_: Request) -> Response {
    Response::builder().header(header::CONTENT_TYPE, "text/event-stream").body(Body::from("data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"bad\",\"message\":\"nope\"}}}\n\n")).unwrap()
}
#[tokio::test]
async fn preserves_streamed_errors() {
    let p = OpenAiResponsesProvider::new(
        server(Router::new().route("/v1/responses", post(failed))).await,
        None,
    );
    let e = p
        .stream(request("vendor-model-1", ReasoningEffort::Low))
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert!(matches!(e, ProviderError::Response { ref code, .. } if code == "bad"));
}

#[tokio::test]
async fn adapters_enforce_canonical_ids_and_fake_is_deterministic() {
    let endpoint = server(Router::new().route("/v1/responses", post(sse))).await;
    let codex = CodexBridgeProvider::at(endpoint.clone());
    assert!(
        codex
            .stream(request("terra", ReasoningEffort::Medium))
            .await
            .is_err()
    );
    let xai = XaiProvider::new(endpoint, "not-a-real-secret".into());
    assert!(
        xai.stream(request("short", ReasoningEffort::Medium))
            .await
            .is_err()
    );
    assert_eq!(ReasoningEffort::normalize("ultra"), ReasoningEffort::XHigh);
    assert_eq!(
        ReasoningEffort::normalize("surprise"),
        ReasoningEffort::Medium
    );
    let fake = DeterministicFakeProvider::new(vec![ProviderEvent::OutputDelta("fixed".into())]);
    let event = fake
        .stream(request("fake-model-1", ReasoningEffort::None))
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event, ProviderEvent::OutputDelta("fixed".into()));
}

#[tokio::test]
async fn request_uses_normalized_effort_and_never_requires_bridge_auth() {
    async fn inspect(request: Request) -> Response {
        assert!(request.headers().get(header::AUTHORIZATION).is_none());
        let body = axum::body::to_bytes(request.into_body(), 100_000)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["model"], "gpt-5.6-sol");
        assert_eq!(value["reasoning"]["effort"], "xhigh");
        sse(Request::new(Body::empty())).await
    }
    let p =
        CodexBridgeProvider::at(server(Router::new().route("/v1/responses", post(inspect))).await);
    let _: Vec<_> = p
        .stream(request("gpt-5.6-sol", ReasoningEffort::XHigh))
        .await
        .unwrap()
        .collect()
        .await;
}
