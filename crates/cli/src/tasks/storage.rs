//! Task payloads, request receipts, and run journals are encrypted even in the local database.
use crate::app::OutboxItem;
use crate::local_store::LocalStore;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(crate) fn initialize(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_tasks (
            id TEXT PRIMARY KEY, revision INTEGER NOT NULL, ciphertext BLOB NOT NULL);
         CREATE TABLE IF NOT EXISTS task_receipts (
            id TEXT PRIMARY KEY, ciphertext BLOB NOT NULL);
         CREATE TABLE IF NOT EXISTS task_runs (
            id TEXT PRIMARY KEY, status TEXT NOT NULL, ciphertext BLOB NOT NULL);",
    )?;
    Ok(())
}

/// Claims reach the WAL before effects start, including across an OS/power failure.
/// Other chat transactions keep the store's configured synchronization level.
fn durable<T>(
    connection: &mut Connection,
    write: impl FnOnce(&Transaction<'_>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let previous: i64 = connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let result: anyhow::Result<T> = (|| {
        let tx = connection.transaction()?;
        let value = write(&tx)?;
        tx.commit()?;
        Ok(value)
    })();
    let restored = connection.pragma_update(None, "synchronous", previous);
    let value = result?;
    restored?;
    Ok(value)
}

impl LocalStore {
    pub(crate) fn task_rows(&self) -> anyhow::Result<Vec<Vec<u8>>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT ciphertext FROM durable_tasks ORDER BY id")?;
        let rows = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn task_row(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT ciphertext FROM durable_tasks WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub(crate) fn task_receipt(&self, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT ciphertext FROM task_receipts WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// CAS, the receipt, task sync, and dispatch intent commit together.
    pub(crate) fn commit_task(
        &self,
        id: &str,
        expected: Option<u64>,
        revision: u64,
        ciphertext: &[u8],
        receipt: Option<(&str, &[u8])>,
        outbox: &[OutboxItem],
        local_run: Option<(&str, &[u8])>,
    ) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        durable(&mut connection, |tx| {
            let changed = match expected {
            Some(expected) => tx.execute(
                "UPDATE durable_tasks SET revision=?2,ciphertext=?3 WHERE id=?1 AND revision=?4",
                params![id, revision, ciphertext, expected],
            )?,
            None => tx.execute(
                "INSERT OR IGNORE INTO durable_tasks(id,revision,ciphertext) VALUES(?1,?2,?3)",
                params![id, revision, ciphertext],
            )?,
        };
            anyhow::ensure!(
                changed == 1,
                "Task revision conflict; read the task again before editing."
            );
            if let Some((id, ciphertext)) = receipt {
                tx.execute(
                    "INSERT INTO task_receipts(id,ciphertext) VALUES(?1,?2)",
                    params![id, ciphertext],
                )?;
            }
            for item in outbox {
                crate::local_store::queue_outbox_tx(&tx, item)?;
            }
            if let Some((id, ciphertext)) = local_run {
                tx.execute(
                    "INSERT INTO task_runs(id,status,ciphertext) VALUES(?1,'pending',?2)",
                    params![id, ciphertext],
                )?;
            }
            Ok(())
        })
    }

    /// A relay replay never replaces a newer revision. Same-revision differences are handled
    /// by the caller as a conflict, not by last arrival.
    pub(crate) fn sync_task(
        &self,
        id: &str,
        revision: u64,
        ciphertext: &[u8],
    ) -> anyhow::Result<bool> {
        let changed = self.connection.lock().unwrap().execute(
            "INSERT INTO durable_tasks(id,revision,ciphertext) VALUES(?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,ciphertext=excluded.ciphertext WHERE durable_tasks.revision < excluded.revision",
            params![id,revision,ciphertext])?;
        Ok(changed == 1)
    }

    /// The execution claim is persisted before any tool can run. It remains after completion.
    pub(crate) fn claim_task_run(&self, id: &str, ciphertext: &[u8]) -> anyhow::Result<bool> {
        let mut connection = self.connection.lock().unwrap();
        durable(&mut connection, |tx| {
            tx.execute(
                "INSERT OR IGNORE INTO task_runs(id,status,ciphertext) VALUES(?1,'pending',?2)",
                params![id, ciphertext],
            )?;
            let claimed = tx.execute(
                "UPDATE task_runs SET status='claimed' WHERE id=?1 AND status='pending'",
                [id],
            )? == 1;
            Ok(claimed)
        })
    }

    pub(crate) fn task_run_status(
        &self,
        id: &str,
        status: &str,
        ciphertext: &[u8],
    ) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        durable(&mut connection, |tx| {
            tx.execute(
                "UPDATE task_runs SET status=?2,ciphertext=?3 WHERE id=?1",
                params![id, status, ciphertext],
            )?;
            Ok(())
        })
    }

    pub(crate) fn task_runs(&self, status: &str) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT id,ciphertext FROM task_runs WHERE status=?1 ORDER BY id")?;
        let rows = statement
            .query_map([status], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
