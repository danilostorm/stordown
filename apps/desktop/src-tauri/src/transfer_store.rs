use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct TransferStore {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRecord {
    pub id: String,
    pub direction: String,
    pub name: String,
    pub source: String,
    pub destination: String,
    pub provider: String,
    pub status: String,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub connections: usize,
    pub bind_ips: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewTransferRecord {
    pub id: String,
    pub direction: String,
    pub name: String,
    pub source: String,
    pub destination: String,
    pub provider: String,
    pub connections: usize,
    pub bind_ips: Vec<String>,
}

impl TransferStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Falha ao criar pasta do banco StorDown: {error}"))?;
        }

        let connection = Connection::open(path)
            .map_err(|error| format!("Falha ao abrir banco StorDown: {error}"))?;

        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| format!("Falha ao configurar SQLite WAL: {error}"))?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|error| format!("Falha ao configurar SQLite: {error}"))?;

        connection
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS transfers (
                    id TEXT PRIMARY KEY,
                    direction TEXT NOT NULL,
                    name TEXT NOT NULL,
                    source TEXT NOT NULL,
                    destination TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    status TEXT NOT NULL,
                    bytes_transferred INTEGER NOT NULL DEFAULT 0,
                    total_bytes INTEGER,
                    connections INTEGER NOT NULL DEFAULT 1,
                    bind_ips_json TEXT NOT NULL DEFAULT '[]',
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    error TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_transfers_status_updated
                    ON transfers(status, updated_at DESC);

                CREATE INDEX IF NOT EXISTS idx_transfers_updated
                    ON transfers(updated_at DESC);
                "#,
            )
            .map_err(|error| format!("Falha ao preparar banco StorDown: {error}"))?;

        connection
            .execute(
                "UPDATE transfers
                 SET status = 'interrupted', updated_at = ?1
                 WHERE status IN ('running', 'paused')",
                params![now_epoch()],
            )
            .map_err(|error| format!("Falha ao recuperar histórico StorDown: {error}"))?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self, String> {
        let connection =
            Connection::open_in_memory().map_err(|error| format!("SQLite memory error: {error}"))?;

        connection
            .execute_batch(
                r#"
                CREATE TABLE transfers (
                    id TEXT PRIMARY KEY,
                    direction TEXT NOT NULL,
                    name TEXT NOT NULL,
                    source TEXT NOT NULL,
                    destination TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    status TEXT NOT NULL,
                    bytes_transferred INTEGER NOT NULL DEFAULT 0,
                    total_bytes INTEGER,
                    connections INTEGER NOT NULL DEFAULT 1,
                    bind_ips_json TEXT NOT NULL DEFAULT '[]',
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    error TEXT
                );
                "#,
            )
            .map_err(|error| error.to_string())?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub async fn insert(&self, record: NewTransferRecord) -> Result<TransferRecord, String> {
        let now = now_epoch();
        let bind_ips_json =
            serde_json::to_string(&record.bind_ips).map_err(|error| error.to_string())?;
        let connection = self.connection.lock().await;

        connection
            .execute(
                "INSERT INTO transfers (
                    id, direction, name, source, destination, provider, status,
                    bytes_transferred, total_bytes, connections, bind_ips_json,
                    created_at, updated_at, error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 0, NULL, ?7, ?8, ?9, ?10, NULL)",
                params![
                    record.id,
                    record.direction,
                    record.name,
                    record.source,
                    record.destination,
                    record.provider,
                    record.connections as i64,
                    bind_ips_json,
                    now,
                    now,
                ],
            )
            .map_err(|error| format!("Falha ao adicionar transferência à fila: {error}"))?;

        drop(connection);
        self.get(&record.id)
            .await?
            .ok_or_else(|| "Transferência recém-criada não encontrada".to_string())
    }

    pub async fn get(&self, id: &str) -> Result<Option<TransferRecord>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection
            .prepare(
                "SELECT id, direction, name, source, destination, provider, status,
                        bytes_transferred, total_bytes, connections, bind_ips_json,
                        created_at, updated_at, error
                 FROM transfers
                 WHERE id = ?1",
            )
            .map_err(|error| error.to_string())?;

        statement
            .query_row(params![id], row_to_record)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub async fn list(&self, limit: usize) -> Result<Vec<TransferRecord>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection
            .prepare(
                "SELECT id, direction, name, source, destination, provider, status,
                        bytes_transferred, total_bytes, connections, bind_ips_json,
                        created_at, updated_at, error
                 FROM transfers
                 ORDER BY
                    CASE status
                      WHEN 'running' THEN 0
                      WHEN 'paused' THEN 1
                      WHEN 'queued' THEN 2
                      WHEN 'interrupted' THEN 3
                      ELSE 4
                    END,
                    updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map(params![limit as i64], row_to_record)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub async fn update_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "UPDATE transfers
                 SET status = ?2, error = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![id, status, error, now_epoch()],
            )
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    pub async fn update_progress(
        &self,
        id: &str,
        bytes_transferred: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "UPDATE transfers
                 SET bytes_transferred = MAX(bytes_transferred, ?2),
                     total_bytes = COALESCE(?3, total_bytes),
                     updated_at = ?4
                 WHERE id = ?1",
                params![
                    id,
                    to_sql_i64(bytes_transferred),
                    total_bytes.map(to_sql_i64),
                    now_epoch()
                ],
            )
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    pub async fn complete(
        &self,
        id: &str,
        bytes_transferred: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "UPDATE transfers
                 SET status = 'completed',
                     bytes_transferred = MAX(bytes_transferred, ?2),
                     total_bytes = COALESCE(?3, total_bytes, ?2),
                     error = NULL,
                     updated_at = ?4
                 WHERE id = ?1",
                params![
                    id,
                    to_sql_i64(bytes_transferred),
                    total_bytes.map(to_sql_i64),
                    now_epoch()
                ],
            )
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute("DELETE FROM transfers WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub async fn clear_finished(&self) -> Result<usize, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "DELETE FROM transfers
                 WHERE status IN ('completed', 'failed', 'cancelled')",
                [],
            )
            .map_err(|error| error.to_string())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRecord> {
    let bind_ips_json: String = row.get(10)?;
    let bind_ips = serde_json::from_str::<Vec<String>>(&bind_ips_json).unwrap_or_default();
    let bytes: i64 = row.get(7)?;
    let total: Option<i64> = row.get(8)?;
    let connections: i64 = row.get(9)?;

    Ok(TransferRecord {
        id: row.get(0)?,
        direction: row.get(1)?,
        name: row.get(2)?,
        source: row.get(3)?,
        destination: row.get(4)?,
        provider: row.get(5)?,
        status: row.get(6)?,
        bytes_transferred: bytes.max(0) as u64,
        total_bytes: total.map(|value| value.max(0) as u64),
        connections: connections.max(1) as usize,
        bind_ips,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        error: row.get(13)?,
    })
}

fn to_sql_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::{NewTransferRecord, TransferStore};

    #[tokio::test]
    async fn stores_and_updates_transfer_history() {
        let store = TransferStore::memory().unwrap();

        store
            .insert(NewTransferRecord {
                id: "abc".to_string(),
                direction: "download".to_string(),
                name: "file.iso".to_string(),
                source: "https://example.test/file.iso".to_string(),
                destination: "C:\\Downloads\\file.iso".to_string(),
                provider: "http".to_string(),
                connections: 8,
                bind_ips: vec!["192.168.1.10".to_string(), "192.168.1.11".to_string()],
            })
            .await
            .unwrap();

        store.update_status("abc", "running", None).await.unwrap();
        store
            .update_progress("abc", 512, Some(1024))
            .await
            .unwrap();
        store.complete("abc", 1024, Some(1024)).await.unwrap();

        let record = store.get("abc").await.unwrap().unwrap();
        assert_eq!(record.status, "completed");
        assert_eq!(record.bytes_transferred, 1024);
        assert_eq!(record.total_bytes, Some(1024));
        assert_eq!(record.bind_ips.len(), 2);
    }
}
