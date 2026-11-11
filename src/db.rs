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

    // Migration: old vote tables are replaced by votes, vote_signatures and
    // word_vote_details. Safe to remove once every deployment has run it
    sqlx::query("DROP TABLE IF EXISTS word_vote_signatures")
        .execute(db)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS word_votes")
        .execute(db)
        .await?;

    // Generic open votes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS votes (
            msg_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            required INTEGER NOT NULL,
            PRIMARY KEY (msg_id, chat_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS vote_signatures (
            msg_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            agree INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (msg_id, chat_id, user_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    // Migration: add agree column to vote_signatures, defaulting to 1 for existing rows
    let _ = sqlx::query("ALTER TABLE vote_signatures ADD COLUMN agree INTEGER NOT NULL DEFAULT 1")
        .execute(db)
        .await;

    // Details for votes about banned words
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS word_vote_details (
            msg_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            word TEXT NOT NULL,
            action TEXT NOT NULL,
            PRIMARY KEY (msg_id, chat_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    // At most one open vote per word per chat
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_word_vote_details_open
         ON word_vote_details (chat_id, word)",
    )
    .execute(db)
    .await?;

    // Details for votes about removing quotes
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS quote_vote_details (
            msg_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            quote_text TEXT NOT NULL,
            user_id INTEGER NOT NULL,
            author TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (msg_id, chat_id)
        )
        "#,
    )
    .execute(db)
    .await?;

    // Migration: add the author to databases created before it existed
    let _ =
        sqlx::query("ALTER TABLE quote_vote_details ADD COLUMN author TEXT NOT NULL DEFAULT ''")
            .execute(db)
            .await;

    // At most one open vote per quote per chat
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_quote_vote_details_open
         ON quote_vote_details (chat_id, quote_text, user_id)",
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

/// All chats the bot is authorized in
pub async fn list_authorized_chats(db: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT chat_id FROM authorized_chats")
        .fetch_all(db)
        .await
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordAction {
    Ban,
    Unban,
}

impl WordAction {
    pub fn as_str(self) -> &'static str {
        match self {
            WordAction::Ban => "ban",
            WordAction::Unban => "unban",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "ban" => Some(WordAction::Ban),
            "unban" => Some(WordAction::Unban),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteSubject {
    Word {
        word: String,
        action: WordAction,
    },
    Quote {
        text: String,
        user_id: i64,
        author: String,
    },
}

impl VoteSubject {
    fn kind(&self) -> &'static str {
        match self {
            VoteSubject::Word { .. } => "word",
            VoteSubject::Quote { .. } => "quote",
        }
    }
}

/// True if there is already an open vote about this subject in this chat
pub async fn has_open_vote(
    db: &SqlitePool,
    chat_id: i64,
    subject: &VoteSubject,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = match subject {
        VoteSubject::Word { word, .. } => {
            sqlx::query_as("SELECT 1 FROM word_vote_details WHERE chat_id = ? AND word = ?")
                .bind(chat_id)
                .bind(word)
                .fetch_optional(db)
                .await?
        }
        VoteSubject::Quote {
            text,
            user_id,
            author: _,
        } => {
            sqlx::query_as(
                "SELECT 1 FROM quote_vote_details
                 WHERE chat_id = ? AND quote_text = ? AND user_id = ?",
            )
            .bind(chat_id)
            .bind(text)
            .bind(user_id)
            .fetch_optional(db)
            .await?
        }
    };
    Ok(row.is_some())
}

/// Opens a vote and records the proposer's vote, in one transaction.
pub async fn create_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
    proposer_id: i64,
    required: i64,
    subject: &VoteSubject,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    sqlx::query("INSERT INTO votes (msg_id, chat_id, kind, required) VALUES (?, ?, ?, ?)")
        .bind(msg_id)
        .bind(chat_id)
        .bind(subject.kind())
        .bind(required)
        .execute(&mut *tx)
        .await?;

    match subject {
        VoteSubject::Word { word, action } => {
            sqlx::query(
                "INSERT INTO word_vote_details (msg_id, chat_id, word, action) VALUES (?, ?, ?, ?)",
            )
            .bind(msg_id)
            .bind(chat_id)
            .bind(word)
            .bind(action.as_str())
            .execute(&mut *tx)
            .await?;
        }
        VoteSubject::Quote {
            text,
            user_id,
            author,
        } => {
            sqlx::query(
                "INSERT INTO quote_vote_details (msg_id, chat_id, quote_text, user_id, author)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(msg_id)
            .bind(chat_id)
            .bind(text)
            .bind(user_id)
            .bind(author)
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query("INSERT INTO vote_signatures (msg_id, chat_id, user_id) VALUES (?, ?, ?)")
        .bind(msg_id)
        .bind(chat_id)
        .bind(proposer_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Votes still needed to pass, or None if the vote isn't open
pub async fn get_required_votes(db: &SqlitePool, msg_id: i64, chat_id: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT required FROM votes WHERE msg_id = ? AND chat_id = ?")
        .bind(msg_id)
        .bind(chat_id)
        .fetch_optional(db)
        .await
        .unwrap_or_else(|e| {
            log::error!("vote lookup failed: {e}");
            None
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteTally {
    pub agree: i64,
    pub against: i64,
}

/// Current agree / against counts for an open vote.
pub async fn vote_tally(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
) -> Result<VoteTally, sqlx::Error> {
    let (agree, against): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(agree), 0), COALESCE(SUM(1 - agree), 0)
         FROM vote_signatures WHERE msg_id = ? AND chat_id = ?",
    )
    .bind(msg_id)
    .bind(chat_id)
    .fetch_one(db)
    .await?;
    Ok(VoteTally { agree, against })
}

/// Records a vote, or switches the user's earlier vote to the other side.
/// Returns the new tally, or None if the user already voted that way.
pub async fn cast_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
    user_id: i64,
    agree: bool,
) -> Result<Option<VoteTally>, sqlx::Error> {
    // The WHERE makes the update a no-op when the direction is unchanged,
    // so rows_affected is 0 exactly when nothing changed
    let res = sqlx::query(
        "INSERT INTO vote_signatures (msg_id, chat_id, user_id, agree) VALUES (?, ?, ?, ?)
         ON CONFLICT (msg_id, chat_id, user_id)
         DO UPDATE SET agree = excluded.agree
         WHERE vote_signatures.agree != excluded.agree",
    )
    .bind(msg_id)
    .bind(chat_id)
    .bind(user_id)
    .bind(agree)
    .execute(db)
    .await?;

    if res.rows_affected() == 0 {
        return Ok(None);
    }
    Ok(Some(vote_tally(db, msg_id, chat_id).await?))
}

impl VoteSubject {
    /// Deletes detail records for a vote and optionally applies its side effects.
    async fn delete_and_apply(
        kind: &str,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        msg_id: i64,
        chat_id: i64,
        apply_effect: bool,
    ) -> Result<Self, sqlx::Error> {
        match kind {
            "word" => {
                let (word, action): (String, String) = sqlx::query_as(
                    "DELETE FROM word_vote_details WHERE msg_id = ? AND chat_id = ?
                     RETURNING word, action",
                )
                .bind(msg_id)
                .bind(chat_id)
                .fetch_one(&mut **tx)
                .await?;

                let action = WordAction::parse(&action).ok_or_else(|| {
                    sqlx::Error::Decode(format!("bad word action: {action}").into())
                })?;

                if apply_effect {
                    let sql = match action {
                        WordAction::Ban => {
                            "INSERT OR IGNORE INTO banned_words (chat_id, word) VALUES (?, ?)"
                        }
                        WordAction::Unban => {
                            "DELETE FROM banned_words WHERE chat_id = ? AND word = ?"
                        }
                    };
                    sqlx::query(sql)
                        .bind(chat_id)
                        .bind(&word)
                        .execute(&mut **tx)
                        .await?;
                }

                Ok(VoteSubject::Word { word, action })
            }
            "quote" => {
                let (text, user_id, author): (String, i64, String) = sqlx::query_as(
                    "DELETE FROM quote_vote_details WHERE msg_id = ? AND chat_id = ?
                     RETURNING quote_text, user_id, author",
                )
                .bind(msg_id)
                .bind(chat_id)
                .fetch_one(&mut **tx)
                .await?;

                if apply_effect {
                    sqlx::query(
                        "DELETE FROM quotes WHERE chat_id = ? AND message_text = ? AND user_id = ?",
                    )
                    .bind(chat_id)
                    .bind(&text)
                    .bind(user_id)
                    .execute(&mut **tx)
                    .await?;
                }

                Ok(VoteSubject::Quote {
                    text,
                    user_id,
                    author,
                })
            }
            other => Err(sqlx::Error::Decode(
                format!("unknown vote kind: {other}").into(),
            )),
        }
    }
}

async fn close_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
    apply_effect: bool,
) -> Result<Option<VoteSubject>, sqlx::Error> {
    let mut tx = db.begin().await?;

    // Deleting the core row ensures only one caller wins the race condition
    let kind: Option<String> =
        sqlx::query_scalar("DELETE FROM votes WHERE msg_id = ? AND chat_id = ? RETURNING kind")
            .bind(msg_id)
            .bind(chat_id)
            .fetch_optional(&mut *tx)
            .await?;

    let Some(kind) = kind else {
        return Ok(None);
    };

    // Delegate detail cleanup and effect application to VoteSubject
    let subject =
        VoteSubject::delete_and_apply(&kind, &mut tx, msg_id, chat_id, apply_effect).await?;

    sqlx::query("DELETE FROM vote_signatures WHERE msg_id = ? AND chat_id = ?")
        .bind(msg_id)
        .bind(chat_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(Some(subject))
}

/// TEMPORARY: used once at startup to redraw the keyboards of proposals that
/// were open before for/against voting
pub async fn list_open_votes(
    db: &SqlitePool,
) -> Result<Vec<(i64, i64, i64, i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT v.msg_id, v.chat_id, v.required,
                COALESCE(SUM(s.agree), 0), COALESCE(SUM(1 - s.agree), 0)
         FROM votes v
         LEFT JOIN vote_signatures s ON s.msg_id = v.msg_id AND s.chat_id = v.chat_id
         GROUP BY v.msg_id, v.chat_id, v.required",
    )
    .fetch_all(db)
    .await
}

/// Closes a vote and applies its effect in one transaction
/// Returns the subject only to the caller that actually closed it
pub async fn pass_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
) -> Result<Option<VoteSubject>, sqlx::Error> {
    close_vote(db, msg_id, chat_id, true).await
}

/// Closes a vote without applying anything
/// Returns the subject only to the caller that actually closed it
pub async fn reject_vote(
    db: &SqlitePool,
    msg_id: i64,
    chat_id: i64,
) -> Result<Option<VoteSubject>, sqlx::Error> {
    close_vote(db, msg_id, chat_id, false).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_db() -> SqlitePool {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        create_tables(&db).await.unwrap();
        db
    }

    fn word(w: &str, action: WordAction) -> VoteSubject {
        VoteSubject::Word {
            word: w.into(),
            action,
        }
    }

    #[tokio::test]
    async fn passed_ban_vote_bans_the_word_once() {
        let db = test_db().await;
        let subject = word("he", WordAction::Ban);
        create_vote(&db, 10, 1, 100, 2, &subject).await.unwrap();

        assert!(has_open_vote(&db, 1, &subject).await.unwrap());
        assert_eq!(pass_vote(&db, 10, 1).await.unwrap(), Some(subject.clone()));
        // A second closer gets nothing
        assert_eq!(pass_vote(&db, 10, 1).await.unwrap(), None);
        assert!(!has_open_vote(&db, 1, &subject).await.unwrap());

        let banned = load_banned_words(&db).await.unwrap();
        assert!(banned[&1].contains("he"));
    }

    #[tokio::test]
    async fn passed_unban_vote_removes_the_word() {
        let db = test_db().await;
        sqlx::query("INSERT INTO banned_words (chat_id, word) VALUES (1, 'he')")
            .execute(&db)
            .await
            .unwrap();

        create_vote(&db, 10, 1, 100, 2, &word("he", WordAction::Unban))
            .await
            .unwrap();
        pass_vote(&db, 10, 1).await.unwrap();

        let banned = load_banned_words(&db).await.unwrap();
        assert!(banned.get(&1).is_none_or(|s| !s.contains("he")));
    }

    #[tokio::test]
    async fn second_open_vote_for_same_word_is_rejected() {
        let db = test_db().await;
        let subject = word("he", WordAction::Ban);
        create_vote(&db, 10, 1, 100, 2, &subject).await.unwrap();
        assert!(create_vote(&db, 11, 1, 101, 2, &subject).await.is_err());
        // the failed attempt left nothing behind
        assert_eq!(get_required_votes(&db, 11, 1).await, None);
    }

    fn quote_subject(text: &str, user_id: i64) -> VoteSubject {
        VoteSubject::Quote {
            text: text.into(),
            user_id,
            author: "@x".into(),
        }
    }

    async fn add_quote(db: &SqlitePool, chat_id: i64, user_id: i64, text: &str) {
        sqlx::query(
            "INSERT INTO quotes (chat_id, user_id, username, message_text) VALUES (?, ?, 'x', ?)",
        )
        .bind(chat_id)
        .bind(user_id)
        .bind(text)
        .execute(db)
        .await
        .unwrap();
    }

    async fn quote_count(db: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM quotes")
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn passed_quote_vote_removes_only_matching_quotes() {
        let db = test_db().await;
        add_quote(&db, 1, 7, "lol").await; // target
        add_quote(&db, 1, 7, "lol").await; // duplicate by same author: also removed
        add_quote(&db, 1, 8, "lol").await; // same text, other author: kept
        add_quote(&db, 2, 7, "lol").await; // other chat: kept
        add_quote(&db, 1, 7, "other").await; // other text: kept

        let subject = quote_subject("lol", 7);
        create_vote(&db, 10, 1, 100, 2, &subject).await.unwrap();
        assert_eq!(quote_count(&db).await, 5); // nothing deleted before it passes

        assert_eq!(pass_vote(&db, 10, 1).await.unwrap(), Some(subject));
        assert_eq!(quote_count(&db).await, 3);
        assert_eq!(pass_vote(&db, 10, 1).await.unwrap(), None);
    }

    #[tokio::test]
    async fn second_open_vote_for_same_quote_is_rejected() {
        let db = test_db().await;
        let subject = quote_subject("lol", 7);
        create_vote(&db, 10, 1, 100, 2, &subject).await.unwrap();

        assert!(has_open_vote(&db, 1, &subject).await.unwrap());
        // same text with a different author is a different quote
        assert!(
            !has_open_vote(&db, 1, &quote_subject("lol", 8))
                .await
                .unwrap()
        );

        assert!(create_vote(&db, 11, 1, 101, 2, &subject).await.is_err());
        // rejected attempt leaves nothing behind
        assert_eq!(get_required_votes(&db, 11, 1).await, None);
    }

    #[tokio::test]
    async fn votes_can_be_cast_and_switched() {
        let db = test_db().await;
        // the proposer (user 100) counts as agree
        create_vote(&db, 10, 1, 100, 3, &word("he", WordAction::Ban))
            .await
            .unwrap();
        assert_eq!(
            vote_tally(&db, 10, 1).await.unwrap(),
            VoteTally {
                agree: 1,
                against: 0
            }
        );

        assert_eq!(get_required_votes(&db, 10, 1).await, Some(3));
        assert_eq!(get_required_votes(&db, 99, 1).await, None);

        // a new vote against
        assert_eq!(
            cast_vote(&db, 10, 1, 200, false).await.unwrap(),
            Some(VoteTally {
                agree: 1,
                against: 1
            })
        );
        // the same direction again changes nothing
        assert_eq!(cast_vote(&db, 10, 1, 200, false).await.unwrap(), None);
        // switching sides moves the vote
        assert_eq!(
            cast_vote(&db, 10, 1, 200, true).await.unwrap(),
            Some(VoteTally {
                agree: 2,
                against: 0
            })
        );
        // even the proposer can switch
        assert_eq!(
            cast_vote(&db, 10, 1, 100, false).await.unwrap(),
            Some(VoteTally {
                agree: 1,
                against: 1
            })
        );
    }

    #[tokio::test]
    async fn rejected_vote_applies_nothing_and_can_be_proposed_again() {
        let db = test_db().await;
        let subject = word("he", WordAction::Ban);
        create_vote(&db, 10, 1, 100, 2, &subject).await.unwrap();

        assert_eq!(
            reject_vote(&db, 10, 1).await.unwrap(),
            Some(subject.clone())
        );
        // a second closer gets nothing
        assert_eq!(reject_vote(&db, 10, 1).await.unwrap(), None);

        assert!(!has_open_vote(&db, 1, &subject).await.unwrap());
        let banned = load_banned_words(&db).await.unwrap();
        assert!(banned.get(&1).is_none_or(|s| !s.contains("he")));

        // the same word can be proposed again
        create_vote(&db, 11, 1, 101, 2, &subject).await.unwrap();
    }
}
