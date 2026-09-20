use codex_unified_provider_web::browser_rpc::{
    BrowserRpcClient, BrowserSessionState, BrowserTurnIdentity, BrowserTurnRequest,
    BrowserTurnResult, WebMode,
};

#[tokio::main]
async fn main() {
    let origin = std::env::var("BROWSER_RPC_ORIGIN").expect("BROWSER_RPC_ORIGIN");
    let capability = std::env::var("BROWSER_RPC_CAPABILITY").expect("BROWSER_RPC_CAPABILITY");

    let client = BrowserRpcClient::new(&origin, capability).expect("browser RPC client");
    let session = client.session().await.expect("browser RPC session");
    assert_eq!(session.state, BrowserSessionState::Healthy);
    assert_eq!(session.revision, 2);

    let request = BrowserTurnRequest::new(
        "trace-contract",
        BrowserTurnIdentity {
            thread_id: Some("thread-contract".into()),
            turn_id: "turn-contract".into(),
            request_kind: Some("turn".into()),
        },
        WebMode::Pro,
        "contract probe",
        None,
    );

    let result = client
        .execute_turn(&request)
        .await
        .expect("browser RPC turn");
    assert_eq!(
        result,
        BrowserTurnResult::Success {
            response_id: "response-contract".into(),
            text: "BROWSER RPC CONTRACT OK".into(),
        }
    );

    println!("BROWSER_RPC_CONTRACT_OK");
}
