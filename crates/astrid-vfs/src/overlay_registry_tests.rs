use super::*;
use crate::Vfs;
use std::time::Duration;

fn p(name: &str) -> PrincipalId {
    PrincipalId::new(name).expect("valid principal")
}

fn fixture() -> (tempfile::TempDir, OverlayVfsRegistry) {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let registry = OverlayVfsRegistry::new(workspace.path().to_path_buf(), DirHandle::new());
    (workspace, registry)
}

#[tokio::test]
async fn first_call_builds_second_call_caches() {
    let (_ws, reg) = fixture();
    let alice = p("alice");
    assert_eq!(reg.len(), 0);

    let o1 = reg.resolve(&alice).await.expect("first resolve");
    let o2 = reg.resolve(&alice).await.expect("second resolve");
    assert!(Arc::ptr_eq(&o1, &o2), "cached overlay must be reused");
    assert_eq!(reg.len(), 1);
}

#[tokio::test]
async fn two_principals_isolate_their_writes() {
    let (_ws, reg) = fixture();
    let alice = p("alice");
    let bob = p("bob");

    let alice_vfs = reg.resolve(&alice).await.expect("alice");
    let bob_vfs = reg.resolve(&bob).await.expect("bob");
    let root = reg.root_handle().clone();

    // Each principal writes a file named `foo.txt` with different bytes.
    let alice_file = alice_vfs
        .open(&root, "foo.txt", true, true)
        .await
        .expect("alice open");
    alice_vfs
        .write(&alice_file, b"alice-bytes")
        .await
        .expect("alice write");
    alice_vfs.close(&alice_file).await.expect("alice close");

    let bob_file = bob_vfs
        .open(&root, "foo.txt", true, true)
        .await
        .expect("bob open");
    bob_vfs
        .write(&bob_file, b"bob-bytes")
        .await
        .expect("bob write");
    bob_vfs.close(&bob_file).await.expect("bob close");

    // Re-open and read each: must see only their own bytes.
    let alice_read = alice_vfs
        .open(&root, "foo.txt", false, false)
        .await
        .expect("alice reopen");
    let alice_contents = alice_vfs.read(&alice_read).await.expect("alice read");
    alice_vfs.close(&alice_read).await.ok();

    let bob_read = bob_vfs
        .open(&root, "foo.txt", false, false)
        .await
        .expect("bob reopen");
    let bob_contents = bob_vfs.read(&bob_read).await.expect("bob read");
    bob_vfs.close(&bob_read).await.ok();

    assert_eq!(alice_contents, b"alice-bytes");
    assert_eq!(bob_contents, b"bob-bytes");
}

#[tokio::test]
async fn cap_of_one_evicts_oldest_on_admission() {
    let ws = tempfile::tempdir().expect("ws");
    let reg = OverlayVfsRegistry::with_limits(
        ws.path().to_path_buf(),
        DirHandle::new(),
        1,
        Duration::from_millis(0),
    );
    let alice = p("alice");
    let bob = p("bob");

    reg.resolve(&alice).await.expect("alice");
    assert_eq!(reg.len(), 1);
    reg.resolve(&bob).await.expect("bob");
    // Cap=1 → alice must have been evicted.
    assert_eq!(reg.len(), 1);
    // Resolving alice again rebuilds — new Arc, but still under cap.
    let again = reg.resolve(&alice).await.expect("alice again");
    drop(again);
    assert_eq!(reg.len(), 1);
}

#[tokio::test]
async fn invalidate_drops_entry() {
    let (_ws, reg) = fixture();
    let alice = p("alice");
    reg.resolve(&alice).await.expect("alice");
    assert_eq!(reg.len(), 1);
    reg.invalidate(&alice);
    assert_eq!(reg.len(), 0);
}

#[tokio::test]
async fn concurrent_first_use_does_not_duplicate() {
    // Smoke test: spawning N tasks that all resolve the same principal at
    // the same time must leave exactly one entry behind. We don't assert
    // that they all see the same Arc — the first-writer-wins path is
    // explicitly allowed to build and drop a duplicate overlay — we only
    // require a single entry remains.
    let (_ws, reg) = fixture();
    let reg = Arc::new(reg);
    let principal = p("racer");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let reg = Arc::clone(&reg);
        let p = principal.clone();
        handles.push(tokio::spawn(async move {
            let _ = reg.resolve(&p).await.expect("resolve");
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    assert_eq!(reg.len(), 1, "one entry after concurrent first-use");
}
