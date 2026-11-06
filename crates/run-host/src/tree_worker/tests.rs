use std::sync::mpsc::channel;
use std::sync::Arc;

use nexus_run_contracts::{RunId, StoreError, Timestamp};
use nexus_run_store_sqlite::Lease;

use super::{
    locked, InFlight, LeaseView, TreeBinding, TreeCx, TreeOp, TreeResult, TreeRunner, TreeWorker,
    WorkTree,
};
use crate::ports::SystemClock;
use crate::tree_ledger::TreeLedger;
use crate::tree_worker::TreeStop;

struct Never;

impl TreeRunner for Never {
    fn run(&mut self, _op: &TreeOp, _cx: &mut TreeCx<'_>) -> TreeResult {
        TreeStop::SpawnFailed.into_result()
    }
}

struct Held;

impl LeaseView for Held {
    fn lease(&self, _run: &RunId) -> Result<Option<Lease>, StoreError> {
        Ok(Some(Lease {
            owner_id: "me".to_owned(),
            epoch: 1,
            expires_at: Timestamp::from_unix_millis(i64::MAX),
        }))
    }
}

/// A send only fails on a worker thread that is gone. The operation never started, so the tree is not busy with it.
#[test]
fn a_failed_send_leaves_the_tree_free() {
    let temp = tempfile::tempdir().unwrap();
    let worker = TreeWorker::new(Never, TreeLedger::new(temp.path(), "ws"));
    worker.bind(TreeBinding {
        owner_id: "me".to_owned(),
        leases: Arc::new(Held),
        clock: Arc::new(SystemClock),
    });
    let (send, receive) = channel();
    drop(receive);
    *locked(&worker.jobs) = Some(send);
    let op = TreeOp::Park {
        run: RunId::new(ulid::Ulid::new()),
        dir: temp.path().join("v2"),
        occasion: nexus_run_contracts::Outcome::Withdrawn,
    };
    assert_eq!(worker.begin(op), Err(InFlight));
    assert!(!worker.busy(), "the tree stayed busy with nothing running");
}
