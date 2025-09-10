use nostr_sdk::prelude::*;
use nrc::ops::{CreateDmStep, OpStatus, OperationKind, OpsStore};
use tempfile::TempDir;

#[tokio::test]
async fn ops_store_enqueue_and_load() {
    let tmp = TempDir::new().unwrap();
    let store = OpsStore::new(tmp.path()).unwrap();

    // Build a minimal event for SendMessage op payload
    let keys = Keys::generate();
    let event = EventBuilder::text_note("hello world")
        .sign(&keys)
        .await
        .unwrap();

    let id = store
        .enqueue(OperationKind::SendMessage {
            event: event.clone(),
        })
        .unwrap();

    let loaded = store.load(&id).unwrap();
    match loaded.kind {
        OperationKind::SendMessage { event: e } => {
            assert_eq!(e.id, event.id);
        }
        _ => panic!("unexpected kind"),
    }
    assert_eq!(loaded.status, OpStatus::Pending);
}

#[tokio::test]
async fn ops_store_mark_success() {
    let tmp = TempDir::new().unwrap();
    let store = OpsStore::new(tmp.path()).unwrap();

    let other = Keys::generate().public_key();
    let op_id = store
        .enqueue(OperationKind::CreateDm {
            other_pubkey: other,
            step: CreateDmStep::FetchKeyPackage,
        })
        .unwrap();

    // Mark success
    store.mark_success(&op_id).unwrap();
    let loaded = store.load(&op_id).unwrap();
    assert_eq!(loaded.status, OpStatus::Success);
}
