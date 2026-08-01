use crate::error::ApiError;
use rusqlite::Connection;

/// 启动时自动建表，无需手工执行 SQL
pub const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS batches (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_no     TEXT NOT NULL UNIQUE,
    project_name TEXT NOT NULL,
    manager      TEXT NOT NULL,
    remark       TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS locations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    region     TEXT NOT NULL,
    freezer_no TEXT NOT NULL,
    shelf      TEXT NOT NULL,
    slot       TEXT NOT NULL,
    UNIQUE(region, freezer_no, shelf, slot)
);

CREATE TABLE IF NOT EXISTS samples (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    sample_no   TEXT NOT NULL UNIQUE,
    batch_no    TEXT NOT NULL REFERENCES batches(batch_no),
    sample_type TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'collected',
    location_id INTEGER REFERENCES locations(id),
    updated_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS operation_logs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    sample_no  TEXT NOT NULL,
    operator   TEXT NOT NULL,
    action     TEXT NOT NULL,
    note       TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exception_flags (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    sample_no   TEXT NOT NULL,
    flag_type   TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    resolved    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    resolved_at TEXT
);

-- 样本位置迁入/迁出历史（from 为 NULL 表示首次放入；to 为 NULL 表示归档释放）
CREATE TABLE IF NOT EXISTS location_movements (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    sample_no        TEXT NOT NULL,
    from_location_id INTEGER REFERENCES locations(id),
    to_location_id   INTEGER REFERENCES locations(id),
    operator         TEXT NOT NULL,
    created_at       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_samples_batch_no ON samples(batch_no);
CREATE INDEX IF NOT EXISTS idx_samples_status ON samples(status);
CREATE INDEX IF NOT EXISTS idx_samples_location_id ON samples(location_id);
CREATE INDEX IF NOT EXISTS idx_logs_sample_no ON operation_logs(sample_no);
CREATE INDEX IF NOT EXISTS idx_flags_sample_no ON exception_flags(sample_no);
CREATE INDEX IF NOT EXISTS idx_movements_from ON location_movements(from_location_id);
CREATE INDEX IF NOT EXISTS idx_movements_to ON location_movements(to_location_id);
"#;

/// 打开文件数据库并初始化表结构（目录不存在时自动创建）
pub fn init(path: &str) -> Result<Connection, ApiError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ApiError::internal(format!("无法创建数据库目录 {parent:?}: {e}")))?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// 内存数据库（测试用）
#[cfg(test)]
pub fn init_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(SCHEMA).expect("init schema");
    conn
}
