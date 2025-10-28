use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use teloxide::{RequestError, types::ChatId};

#[derive(Debug)]
pub struct SqliteRequestError(pub sqlx::Error);
impl From<SqliteRequestError> for RequestError {
    fn from(error: SqliteRequestError) -> Self {
        RequestError::Io(std::io::Error::other(error.0.to_string()))
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
    // Create quotes table
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

    // Create authorized_chats table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS authorized_chats (
            chat_id INTEGER PRIMARY KEY
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
