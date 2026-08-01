use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

pub type DbPool = Pool<SqliteConnectionManager>;

/// 建立连接池并自动初始化所有表结构。
pub fn init_pool(db_path: &str) -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|c| {
        c.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    });
    let pool = Pool::builder().max_size(8).build(manager)?;
    {
        let conn = pool.get()?;
        init_schema(&conn)?;
    }
    Ok(pool)
}

/// 程序启动时自动建表，无需手动执行 SQL 文件。
pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS batches (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            batch_no     TEXT NOT NULL UNIQUE,
            project_name TEXT NOT NULL,
            owner        TEXT NOT NULL,
            note         TEXT,
            created_at   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS locations (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            area    TEXT NOT NULL,
            freezer TEXT NOT NULL,
            shelf   TEXT NOT NULL,
            slot    TEXT NOT NULL,
            UNIQUE(area, freezer, shelf, slot)
        );

        CREATE TABLE IF NOT EXISTS samples (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_no         TEXT NOT NULL UNIQUE,
            batch_id          INTEGER NOT NULL,
            sample_type       TEXT NOT NULL,
            status            TEXT NOT NULL,
            location_id       INTEGER,
            last_processed_at TEXT NOT NULL,
            FOREIGN KEY(batch_id) REFERENCES batches(id),
            FOREIGN KEY(location_id) REFERENCES locations(id)
        );

        CREATE TABLE IF NOT EXISTS operation_logs (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_id  INTEGER NOT NULL,
            operator   TEXT NOT NULL,
            action     TEXT NOT NULL,
            note       TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY(sample_id) REFERENCES samples(id)
        );

        CREATE TABLE IF NOT EXISTS exceptions (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_id      INTEGER NOT NULL,
            exception_type TEXT NOT NULL,
            description    TEXT,
            status         TEXT NOT NULL DEFAULT 'open',
            reported_by    TEXT NOT NULL,
            created_at     TEXT NOT NULL,
            resolved_by    TEXT,
            resolved_at    TEXT,
            FOREIGN KEY(sample_id) REFERENCES samples(id)
        );

        CREATE TABLE IF NOT EXISTS location_movements (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            sample_id   INTEGER NOT NULL,
            location_id INTEGER NOT NULL,
            direction   TEXT NOT NULL,
            operator    TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            FOREIGN KEY(sample_id) REFERENCES samples(id),
            FOREIGN KEY(location_id) REFERENCES locations(id)
        );
        "#,
    )
}
