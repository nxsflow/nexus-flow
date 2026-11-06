//! An open that meets a writer on a new db file (6j6v.nmn6). A new file starts in the rollback journal,
//! and `WorkspaceExt::open_store` switches it to WAL. While another connection holds the write lock SQLite answers that
//! switch "database is locked" at once, without consulting `busy_timeout`, so the open used to fail:
//! on the file the change watcher re-arms on after a same-path recreate, and on two processes
//! starting together. The rule itself is pinned without any race by the tests of
//! `nxs_foundation::store::enter_wal`; this one pins that THIS product's open goes through it.
//!
//! The writer's short sleep only makes the old failure likely in every round, so a return of it is
//! seen. A correct open has nothing left to lose, however the threads are scheduled.

use nexus_flow_core::store::Store;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use std::time::Duration;
use tempfile::TempDir;

const ROUNDS: usize = 40;

#[test]
fn an_open_that_meets_a_writer_on_a_new_db_still_gets_a_store() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let db = ws.db_path();
    let path = ws.db_path_str().unwrap();
    for round in 0..ROUNDS {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
        }
        let writer_is_ready = Barrier::new(2);
        let opened = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                // The bare store open: it creates the file and leaves it in the rollback journal.
                let writer = Store::open(&path, ws.replica.site_id).unwrap();
                writer_is_ready.wait();
                while !opened.load(Ordering::SeqCst) {
                    if writer.connection().execute_batch("BEGIN IMMEDIATE").is_ok() {
                        std::thread::sleep(Duration::from_millis(2));
                        let _ = writer.connection().execute_batch("ROLLBACK");
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            writer_is_ready.wait();
            let result = ws.open_store().map(drop);
            opened.store(true, Ordering::SeqCst);
            if let Err(e) = result {
                panic!("round {round}: the open failed: {e}");
            }
        });
    }
}
