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
    pub scheduled_at: Option<i64>,
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
    pub scheduled_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRule {
    pub id: i64,
    pub name: String,
    pub extensions: Vec<String>,
    pub destination: String,
    pub enabled: bool,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewDownloadRule {
    pub name: String,
    pub extensions: Vec<String>,
    pub destination: String,
    pub enabled: bool,
    pub priority: i64,
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
                    scheduled_at INTEGER,
                    error TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_transfers_status_updated
                    ON transfers(status, updated_at DESC);

                CREATE INDEX IF NOT EXISTS idx_transfers_updated
                    ON transfers(updated_at DESC);

                CREATE TABLE IF NOT EXISTS download_rules (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    extensions_json TEXT NOT NULL DEFAULT '[]',
                    destination TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    priority INTEGER NOT NULL DEFAULT 100,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_download_rules_enabled_priority
                    ON download_rules(enabled, priority ASC, id ASC);
                "#,
            )
            .map_err(|error| format!("Falha ao preparar banco StorDown: {error}"))?;

        ensure_column(&connection, "transfers", "scheduled_at", "INTEGER")?;

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
                    scheduled_at INTEGER,
                    error TEXT
                );

                CREATE TABLE download_rules (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    extensions_json TEXT NOT NULL DEFAULT '[]',
                    destination TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    priority INTEGER NOT NULL DEFAULT 100,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
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

        let scheduled_at = record.scheduled_at.filter(|value| *value > now);
        let initial_status = if scheduled_at.is_some() { "scheduled" } else { "queued" };

        connection
            .execute(
                "INSERT INTO transfers (
                    id, direction, name, source, destination, provider, status,
                    bytes_transferred, total_bytes, connections, bind_ips_json,
                    created_at, updated_at, scheduled_at, error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL, ?8, ?9, ?10, ?11, ?12, NULL)",
                params![
                    record.id,
                    record.direction,
                    record.name,
                    record.source,
                    record.destination,
                    record.provider,
                    initial_status,
                    record.connections as i64,
                    bind_ips_json,
                    now,
                    now,
                    scheduled_at,
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
                        created_at, updated_at, scheduled_at, error
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
                        created_at, updated_at, scheduled_at, error
                 FROM transfers
                 ORDER BY
                    CASE status
                      WHEN 'running' THEN 0
                      WHEN 'paused' THEN 1
                      WHEN 'queued' THEN 2
                      WHEN 'scheduled' THEN 3
                      WHEN 'interrupted' THEN 4
                      ELSE 5
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

    pub async fn list_scheduled(&self) -> Result<Vec<TransferRecord>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection
            .prepare(
                "SELECT id, direction, name, source, destination, provider, status,
                        bytes_transferred, total_bytes, connections, bind_ips_json,
                        created_at, updated_at, scheduled_at, error
                 FROM transfers
                 WHERE status = 'scheduled' AND scheduled_at IS NOT NULL
                 ORDER BY scheduled_at ASC",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], row_to_record)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub async fn list_download_rules(&self) -> Result<Vec<DownloadRule>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection
            .prepare(
                "SELECT id, name, extensions_json, destination, enabled, priority, created_at, updated_at
                 FROM download_rules
                 ORDER BY priority ASC, id ASC",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], row_to_download_rule)
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub async fn upsert_download_rule(
        &self,
        id: Option<i64>,
        rule: NewDownloadRule,
    ) -> Result<DownloadRule, String> {
        let now = now_epoch();
        let extensions = normalize_extensions(rule.extensions);
        let extensions_json =
            serde_json::to_string(&extensions).map_err(|error| error.to_string())?;
        let connection = self.connection.lock().await;

        let rule_id = if let Some(id) = id {
            connection
                .execute(
                    "UPDATE download_rules
                     SET name = ?2, extensions_json = ?3, destination = ?4,
                         enabled = ?5, priority = ?6, updated_at = ?7
                     WHERE id = ?1",
                    params![
                        id,
                        rule.name.trim(),
                        extensions_json,
                        rule.destination.trim(),
                        i64::from(rule.enabled),
                        rule.priority,
                        now,
                    ],
                )
                .map_err(|error| error.to_string())?;
            id
        } else {
            connection
                .execute(
                    "INSERT INTO download_rules (
                        name, extensions_json, destination, enabled, priority, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        rule.name.trim(),
                        extensions_json,
                        rule.destination.trim(),
                        i64::from(rule.enabled),
                        rule.priority,
                        now,
                        now,
                    ],
                )
                .map_err(|error| error.to_string())?;
            connection.last_insert_rowid()
        };

        drop(connection);
        self.get_download_rule(rule_id)
            .await?
            .ok_or_else(|| "Regra recém-salva não encontrada".to_string())
    }

    pub async fn delete_download_rule(&self, id: i64) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute("DELETE FROM download_rules WHERE id = ?1", params![id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub async fn get_download_rule(&self, id: i64) -> Result<Option<DownloadRule>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection
            .prepare(
                "SELECT id, name, extensions_json, destination, enabled, priority, created_at, updated_at
                 FROM download_rules
                 WHERE id = ?1",
            )
            .map_err(|error| error.to_string())?;

        statement
            .query_row(params![id], row_to_download_rule)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub async fn match_download_rule(
        &self,
        file_name: &str,
    ) -> Result<Option<DownloadRule>, String> {
        let extension = Path::new(file_name)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());

        let Some(extension) = extension else {
            return Ok(None);
        };

        let rules = self.list_download_rules().await?;
        Ok(rules.into_iter().find(|rule| {
            rule.enabled
                && rule
                    .extensions
                    .iter()
                    .any(|candidate| candidate == "*" || candidate == &extension)
        }))
    }
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| error.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    if !columns.iter().any(|name| name == column) {
        connection
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(|error| format!("Falha ao migrar banco StorDown: {error}"))?;
    }

    Ok(())
}

fn row_to_download_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRule> {
    let extensions_json: String = row.get(2)?;
    let extensions =
        serde_json::from_str::<Vec<String>>(&extensions_json).unwrap_or_default();
    let enabled: i64 = row.get(4)?;

    Ok(DownloadRule {
        id: row.get(0)?,
        name: row.get(1)?,
        extensions,
        destination: row.get(3)?,
        enabled: enabled != 0,
        priority: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn normalize_extensions(values: Vec<String>) -> Vec<String> {
    let mut normalized = values
        .into_iter()
        .flat_map(|value| {
            value
                .split(|ch| ch == ',' || ch == ';' || ch == ' ')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    normalized.sort();
    normalized.dedup();
    normalized
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransferRecord> {
    let bind_ips_json: String = row.get(10)?;
    let bind_ips = serde_json::from_str::<Vec<String>>(&bind_ips_json).unwrap_or_default();
    let bytes: i64 = row.get(7)?;
    let total: Option<i64> = row.get(8)?;
    let connections: i64 = row.get(9)?;
    let scheduled_at: Option<i64> = row.get(13)?;

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
        scheduled_at,
        error: row.get(14)?,
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
    async fn stores_download_rules_and_matches_extension() {
        let store = TransferStore::memory().unwrap();

        let rule = store
            .upsert_download_rule(
                None,
                super::NewDownloadRule {
                    name: "Vídeos".to_string(),
                    extensions: vec!["mkv, mp4".to_string()],
                    destination: "C:\\Downloads\\Vídeos".to_string(),
                    enabled: true,
                    priority: 10,
                },
            )
            .await
            .unwrap();

        assert_eq!(rule.extensions, vec!["mkv", "mp4"]);

        let matched = store
            .match_download_rule("filme.MKV")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(matched.name, "Vídeos");
    }

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
                scheduled_at: None,
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
