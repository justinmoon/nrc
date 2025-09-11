use std::time::Duration;

use nostr_sdk::prelude::*;
use nrc::ops::{spawn_orchestrator, CreateDmStep, OperationKind, OpsCommand, OpsStore};
use nrc::AppEvent;
use tempfile::TempDir;

#[tokio::test]
async fn orchestrator_reemits_storage_request_on_resume() {
    let tmp = TempDir::new().unwrap();
    let store = OpsStore::new(tmp.path()).unwrap();

    // Build a dummy key package event (content doesn't matter for this test)
    let keys = Keys::generate();
    let other_keys = Keys::generate();
    let key_package_event = EventBuilder::text_note("dummy kp")
        .sign(&other_keys)
        .await
        .unwrap();

    let other = other_keys.public_key();
    // Persist an op already at RequestCreateGroup step
    let op_id = store
        .enqueue(OperationKind::CreateDm {
            other_pubkey: other,
            step: CreateDmStep::RequestCreateGroup {
                key_package: key_package_event.clone(),
            },
        })
        .unwrap();

    // Prepare channels and a client; client won't be used on this step
    let client = Client::builder().signer(keys.clone()).build();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let (ops_cmd_tx, ops_cmd_rx) = tokio::sync::mpsc::unbounded_channel();

    spawn_orchestrator(
        store.clone(),
        client,
        keys.clone(),
        event_tx,
        ops_cmd_rx,
        tmp.path().to_path_buf(),
    );

    // Wake the orchestrator and expect an OpNeedsStorageCreateGroup event
    let _ = ops_cmd_tx.send(OpsCommand::Wake);

    let got = tokio::time::timeout(Duration::from_secs(2), async move {
        while let Some(ev) = event_rx.recv().await {
            if let AppEvent::OpNeedsStorageCreateGroup {
                op_id: ev_id,
                other_pubkey,
                key_package,
                ..
            } = ev
            {
                assert_eq!(ev_id, op_id);
                assert_eq!(other_pubkey, other);
                assert_eq!(key_package.id, key_package_event.id);
                break;
            }
        }
    })
    .await;

    assert!(
        got.is_ok(),
        "expected OpNeedsStorageCreateGroup within timeout"
    );
}
