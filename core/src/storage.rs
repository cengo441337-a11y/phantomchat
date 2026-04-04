use rusqlite::{params, Connection, Result};
use std::path::PathBuf;
use std::sync::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);
}

pub fn init_db(path: PathBuf, password: &str) -> Result<()> {
    let conn = Connection::open(path)?;
    
    // Encrypt using SQLCipher PRAGMA
    conn.execute(&format!("PRAGMA key = '{}'", password), [])?;

    // Messages Table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            peer_id TEXT,
            content TEXT,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    // Peers Table (v5: avatar_cid)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS peers (
            peer_id TEXT PRIMARY KEY,
            phantom_id TEXT NOT NULL,
            avatar_cid TEXT,
            last_seen DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    // Groups & Group Messages (v5 - S3)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS groups (
            group_id TEXT PRIMARY KEY,
            group_name TEXT,
            group_key BLOB
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS group_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id TEXT,
            sender_id TEXT,
            content TEXT,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    let mut db = DB_CONN.lock().unwrap();
    *db = Some(conn);
    Ok(())
}

pub fn save_message(peer_id: &str, content: &str) -> Result<()> {
    let db = DB_CONN.lock().unwrap();
    if let Some(conn) = db.as_ref() {
        conn.execute(
            "INSERT INTO messages (peer_id, content) VALUES (?1, ?2)",
            params![peer_id, content],
        )?;
    }
    Ok(())
}

pub fn save_group_message(group_id: &str, sender_id: &str, content: &str) -> Result<()> {
    let db = DB_CONN.lock().unwrap();
    if let Some(conn) = db.as_ref() {
        conn.execute(
            "INSERT INTO group_messages (group_id, sender_id, content) VALUES (?1, ?2, ?3)",
            params![group_id, sender_id, content],
        )?;
    }
    Ok(())
}

pub fn panic_wipe(path: PathBuf) {
    {
        let mut db = DB_CONN.lock().unwrap();
        *db = None;
    }
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}
