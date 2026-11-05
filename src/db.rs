use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use teloxide::{RequestError, types::ChatId};

#[derive(Debug)]
pub struct SqliteRequestError(pub sqlx::Error);
impl From<SqliteRequestError> for RequestError {
    fn from(error: SqliteRequestError) -> Self {
        RequestError::Io(Arc::new(std::io::Error::other(error.0.to_string())))
    }
}

#[derive(sqlx::FromRow, Clone)]
pub struct Quote {
    pub id: i64,
    pub chat_id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub message_text: String,
    pub message_date: chrono::DateTime<Utc>,
}

pub async fn create_tables(db: &SqlitePool) -> Result<(), sqlx::Error> {
    // Authorized chats
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS authorized_chats (
            chat_id INTEGER PRIMARY KEY,
            required_votes INTEGER DEFAULT NULL
        )
        "#,
    )
    .execute(db)
    .await?;

    // Migration to add required_votes column
    let _ =
        sqlx::query("ALTER TABLE authorized_chats ADD COLUMN required_votes INTEGER DEFAULT NULL")
            .execute(db)
            .await;

    // Quotes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS quotes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            username TEXT,
            message_text TEXT NOT NULL,
            message_date DATETIME DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(db)
    .await?;

    // Banned words
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS banned_words (
            chat_id INTEGER NOT NULL,
            word TEXT NOT NULL,
            PRIMARY KEY (chat_id, word)
        )
        "#,
    )
    .execute(db)
    .await?;

    // Active vote proposals
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS word_votes (
            msg_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            word TEXT NOT NULL,
            action TEXT NOT NULL,
            required INTEGER NOT NULL,
            PRIMARY KEY (msg_id, chat_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_word_votes_open ON word_votes (chat_id, word)",
    )
    .execute(db)
    .await?;

    // User votes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS word_vote_signatures (
            msg_id INTEGER,
            chat_id INTEGER,
            user_id INTEGER,
            PRIMARY KEY (msg_id, chat_id, user_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn is_chat_authorized(db: &SqlitePool, chat_id: ChatId) -> bool {
    sqlx::query("SELECT 1 FROM authorized_chats WHERE chat_id = ?")
        .bind(chat_id.0)
        .fetch_optional(db)
        .await
        .map(|r| r.is_some())
        .unwrap_or(false)
}

/// Fetch all stored banned words grouped by chat_id
pub async fn load_banned_words(
    db: &SqlitePool,
) -> Result<HashMap<i64, HashSet<String>>, sqlx::Error> {
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT chat_id, word FROM banned_words")
        .fetch_all(db)
        .await?;

    let mut map: HashMap<i64, HashSet<String>> = HashMap::new();
    for (chat_id, word) in rows {
        map.entry(chat_id).or_default().insert(word);
    }

    Ok(map)
}

/// True if there is already an open proposal for this word in this chat.
pub async fn has_open_proposal(
    db: &SqlitePool,
    chat_id: i64,
    word: &str,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM word_votes WHERE chat_id = ? AND word = ?")
            .bind(chat_id)
            .bind(word)
            .fetch_optional(db)
            .await?;
    Ok(row.is_some())
}

/// Get the vote threshold for a chat. Returns None if not set (default threshold applies)
pub async fn get_vote_threshold_setting(db: &SqlitePool, chat_id: i64) -> Option<i64> {
    sqlx::query_as::<_, (Option<i64>,)>(
        "SELECT required_votes FROM authorized_chats WHERE chat_id = ?",
    )
    .bind(chat_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .and_then(|r| r.0)
}

/// Set the vote threshold for a chat. If `votes` is None, default threshold will be used
pub async fn set_vote_threshold(
    db: &SqlitePool,
    chat_id: i64,
    votes: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("UPDATE authorized_chats SET required_votes = ? WHERE chat_id = ?")
        .bind(votes)
        .bind(chat_id)
        .execute(db)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Creates a vote proposal and records the proposer's initial vote
pub async fn create_vote_session(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
    word: &str,
    action: &str,
    proposer_id: i64,
    required: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    sqlx::query(
        "INSERT INTO word_votes (msg_id, chat_id, word, action, required) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(msg_id)
    .bind(chat_id)
    .bind(word)
    .bind(action)
    .bind(required)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO word_vote_signatures (msg_id, chat_id, user_id) VALUES (?, ?, ?)")
        .bind(msg_id)
        .bind(chat_id)
        .bind(proposer_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Registers a user vote. Returns Ok(Some(current_total)) if new, or Ok(None) if user already voted.
pub async fn register_user_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
    user_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO word_vote_signatures (msg_id, chat_id, user_id) VALUES (?, ?, ?)",
    )
    .bind(msg_id)
    .bind(chat_id)
    .bind(user_id)
    .execute(db)
    .await?;

    if res.rows_affected() == 0 {
        return Ok(None); // Duplicate vote attempt
    }

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM word_vote_signatures WHERE msg_id = ? AND chat_id = ?",
    )
    .bind(msg_id)
    .bind(chat_id)
    .fetch_one(db)
    .await?;

    Ok(Some(count.0))
}

/// Closes a proposal and applies its effect to the DB in one transaction.
/// Returns Some((word, action)) only for the caller that actually closed it.
pub async fn finalize_proposal(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let mut tx = db.begin().await?;

    let row: Option<(String, String)> = sqlx::query_as(
        "DELETE FROM word_votes WHERE msg_id = ? AND chat_id = ? RETURNING word, action",
    )
    .bind(msg_id)
    .bind(chat_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some((word, action)) = &row {
        let sql = if action == "ban" {
            "INSERT OR IGNORE INTO banned_words (chat_id, word) VALUES (?, ?)"
        } else {
            "DELETE FROM banned_words WHERE chat_id = ? AND word = ?"
        };
        sqlx::query(sql)
            .bind(chat_id)
            .bind(word)
            .execute(&mut *tx)
            .await?;

        // Also clean up the votes for this proposal
        sqlx::query("DELETE FROM word_vote_signatures WHERE msg_id = ? AND chat_id = ?")
            .bind(msg_id)
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(row)
}
