use sqlx::sqlite::SqlitePool;
use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(Clone)]
struct Bot {
    db: SqlitePool,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
enum Command {
    #[command(description = "Save a quote (reply to a message)")]
    Quote,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting Immutable Bot...");

    // Initialize database
    let db = SqlitePool::connect("sqlite:/data/quotes.db")
        .await
        .expect("Failed to connect to database");
    
    // Create quotes table
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS quotes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            username TEXT,
            message_text TEXT NOT NULL,
            saved_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&db)
    .await
    .expect("Failed to create table");

    let bot = teloxide::Bot::from_env();
    let bot_state = Bot { db };

    log::info!("Bot started successfully!");

    Command::repl_with_listener(
        bot,
        move |bot: teloxide::Bot, msg: Message, cmd: Command| {
            let bot_state = bot_state.clone();
            async move {
                answer(bot, msg, cmd, bot_state).await
            }
        },
        teloxide::update_listeners::polling_default(teloxide::Bot::from_env()).await,
    )
    .await;
}

async fn answer(bot: teloxide::Bot, msg: Message, cmd: Command, state: Bot) -> ResponseResult<()> {
    match cmd {
        Command::Quote => {
            if let Some(replied_msg) = msg.reply_to_message() {
                if let Some(text) = replied_msg.text() {
                    let user_id = replied_msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
                    let username = replied_msg.from().and_then(|u| u.username.clone());
                    
                    let result = sqlx::query(
                        "INSERT INTO quotes (user_id, username, message_text) VALUES (?, ?, ?)"
                    )
                    .bind(user_id)
                    .bind(username.as_deref())
                    .bind(text)
                    .execute(&state.db)
                    .await;

                    match result {
                        Ok(_) => {
                            let user_display = username
                                .map(|u| format!("@{}", u))
                                .unwrap_or_else(|| format!("User {}", user_id));
                            
                            bot.send_message(
                                msg.chat.id,
                                format!("✅ Quote saved from {}!", user_display)
                            )
                            .await?;
                        }
                        Err(e) => {
                            log::error!("Database error: {}", e);
                            bot.send_message(msg.chat.id, "❌ Failed to save quote")
                                .await?;
                        }
                    }
                } else {
                    bot.send_message(msg.chat.id, "⚠️ Can only save text messages")
                        .await?;
                }
            } else {
                bot.send_message(
                    msg.chat.id,
                    "⚠️ Please reply to a message with /quote to save it"
                )
                .await?;
            }
        }
    }

    Ok(())
}