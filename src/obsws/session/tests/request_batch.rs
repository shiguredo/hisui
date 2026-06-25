//! RequestBatch 系のテスト (`haltOnFailure` の挙動)。

use crate::obsws::session::{ObswsSession, SessionAction};

use super::common::*;

#[tokio::test]
async fn request_batch_with_halt_on_failure_stops_after_first_failure() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .on_text_message(
            r#"{"op":8,"d":{"requestId":"batch-1","haltOnFailure":true,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
        )
        .await
        .expect("request batch must be parsed");
    let text = unwrap_send_text(action);
    let results = parse_request_batch_results(&text);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "CreateScene");
    assert!(results[0].1);
    assert_eq!(results[1].0, "CreateScene");
    assert!(!results[1].1);
}

#[tokio::test]
async fn request_batch_without_halt_on_failure_continues_after_failure() {
    let mut session = ObswsSession::new(None, default_coordinator_handle());
    let identify_action = session
        .on_text_message(r#"{"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}}"#)
        .await
        .expect("identify must succeed");
    assert!(matches!(identify_action, SessionAction::SendText { .. }));

    let action = session
        .on_text_message(
            r#"{"op":8,"d":{"requestId":"batch-2","haltOnFailure":false,"requests":[{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"CreateScene","requestData":{"sceneName":"Scene B"}},{"requestType":"SetCurrentProgramScene","requestData":{"sceneName":"Scene B"}}]}}"#,
        )
        .await
        .expect("request batch must be parsed");
    let text = unwrap_send_text(action);
    let results = parse_request_batch_results(&text);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0, "CreateScene");
    assert!(results[0].1);
    assert_eq!(results[1].0, "CreateScene");
    assert!(!results[1].1);
    assert_eq!(results[2].0, "SetCurrentProgramScene");
    assert!(results[2].1);
}
