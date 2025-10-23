use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use sqlx::sqlite::SqlitePool;
use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(Clone)]
struct Bot {
    db: SqlitePool,
    admin_id: UserId,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
enum Command {
    #[command(description = "Display help message")]
    Help,
    #[command(description = "Save a quote (reply to a message)")]
    Quote,
    #[command(description = "Create a 'guess who said this' poll")]
    GuessWho,
    #[command(description = "Send a group hug to everyone!")]
    Hug,
    #[command(description = "Admin: Authorize this chat for bot use")]
    Authorize,
    #[command(description = "Admin: Deauthorize this chat")]
    Deauthorize,
}

#[derive(sqlx::FromRow)]
struct Quote {
    id: i64,
    chat_id: i64,
    user_id: i64,
    username: Option<String>,
    message_text: String,
    message_date: DateTime<Utc>,
}

#[derive(Debug)]
struct SqlxRequestError(sqlx::Error);
impl From<SqlxRequestError> for teloxide::RequestError {
    fn from(error: SqlxRequestError) -> Self {
        teloxide::RequestError::Io(std::io::Error::other(error.0.to_string()))
    }
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    pretty_env_logger::init();
    log::info!("Starting Immutable Bot...");

    // Get Admin ID from environment variable
    let admin_id_str = std::env::var("ADMIN_ID")
        .expect("ADMIN_ID env variable not set. Please set the bot owner's Telegram User ID.");
    let admin_id: UserId = UserId(admin_id_str.parse::<u64>().unwrap_or_else(|_| {
        panic!("Failed to parse ADMIN_ID as a positive integer.");
    }));

    // Initialize database
    let db = SqlitePool::connect("sqlite:data/quotes.db?mode=rwc")
        .await
        .expect("Failed to connect to database");

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
    .execute(&db)
    .await
    .expect("Failed to create table");

    // Create authorized_chats table
    sqlx::query(
        r#"
    CREATE TABLE IF NOT EXISTS authorized_chats (
        chat_id INTEGER PRIMARY KEY
    )
    "#,
    )
    .execute(&db)
    .await
    .expect("Failed to create authorized_chats table");

    let bot = teloxide::Bot::from_env();
    let bot_state = Bot { db, admin_id };

    log::info!("Bot started successfully!");

    Command::repl_with_listener(
        bot,
        move |bot: teloxide::Bot, msg: Message, cmd: Command| {
            let bot_state = bot_state.clone();
            async move { answer(bot, msg, cmd, bot_state).await }
        },
        teloxide::update_listeners::polling_default(teloxide::Bot::from_env()).await,
    )
    .await;
}

async fn is_chat_authorized(db: &SqlitePool, chat_id: ChatId) -> bool {
    sqlx::query("SELECT 1 FROM authorized_chats WHERE chat_id = ?")
        .bind(chat_id.0)
        .fetch_optional(db)
        .await
        .map(|r| r.is_some())
        .unwrap_or(false)
}

async fn answer(bot: teloxide::Bot, msg: Message, cmd: Command, state: Bot) -> ResponseResult<()> {
    let requires_auth = !matches!(cmd, Command::Authorize | Command::Deauthorize);

    if requires_auth && !is_chat_authorized(&state.db, msg.chat.id).await {
        bot.send_message(
            msg.chat.id,
            "❌ This chat is not authorized to talk to me (╭ರ_•́)",
        )
        .await?;
        return Ok(());
    }

    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
            Ok(())
        }
        Command::Quote => handle_quote(bot, msg, state).await,
        Command::GuessWho => handle_guesswho(bot, msg, state).await,
        Command::Hug => handle_hug(bot, msg).await,
        Command::Authorize => handle_authorize(bot, msg, state).await,
        Command::Deauthorize => handle_deauthorize(bot, msg, state).await,
    }
}

async fn handle_quote(bot: teloxide::Bot, msg: Message, state: Bot) -> ResponseResult<()> {
    if let Some(replied_msg) = msg.reply_to_message() {
        if let Some(text) = replied_msg.text() {
            let user_id = replied_msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
            let username = replied_msg.from().and_then(|u| u.username.clone());
            let original_date = replied_msg.date;
            let chat_id = msg.chat.id.0;

            let result = sqlx::query(
                "INSERT INTO quotes (chat_id, user_id, username, message_text, message_date) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(chat_id)
            .bind(user_id)
            .bind(username.as_deref())
            .bind(text)
            .bind(original_date)
            .execute(&state.db)
            .await;

            match result {
                Ok(_) => {
                    let user_display = username
                        .map(|u| format!("@{}", u))
                        .unwrap_or_else(|| format!("User {}", user_id));

                    bot.send_message(
                        msg.chat.id,
                        format!("✅ Quote saved from {}!", user_display),
                    )
                    .await?;
                }
                Err(e) => {
                    log::error!("Database error: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to save quote (⊙ _ ⊙ )")
                        .await?;
                }
            }
        } else {
            bot.send_message(msg.chat.id, "⚠️ Can only save text messages (ᵕ—ᴗ—)")
                .await?;
        }
    } else {
        bot.send_message(
            msg.chat.id,
            "⚠️ Please reply to a message with /quote to save it ꉂ(˵˃ ᗜ ˂˵)",
        )
        .await?;
    }

    Ok(())
}

async fn handle_guesswho(bot: teloxide::Bot, msg: Message, state: Bot) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    // Get all unique users who have quotes
    let users: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT DISTINCT user_id, username FROM quotes WHERE chat_id = ?")
            .bind(chat_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    if users.len() < 2 {
        bot.send_message(
            msg.chat.id,
            "⚠️ Need at least 2 people with saved quotes to play! ٩( ᐖ )人( ᐛ )و",
        )
        .await?;
        return Ok(());
    }

    // Get a random quote
    let quote: Option<Quote> = sqlx::query_as(
        "SELECT id, chat_id, user_id, username, message_text, message_date FROM quotes WHERE chat_id = ? ORDER BY RANDOM() LIMIT 1",
    )
    .bind(chat_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let quote = match quote {
        Some(q) => q,
        None => {
            bot.send_message(msg.chat.id, "❌ No quotes found in database")
                .await?;
            return Ok(());
        }
    };

    // Create poll options: correct answer + random other users
    let correct_answer = format_user_display(quote.user_id, quote.username.as_deref());

    let mut options = vec![correct_answer.clone()];
    let mut rng = rand::rngs::StdRng::from_entropy();

    // Add up to 3 other random users as options
    let other_users: Vec<String> = users
        .iter()
        .filter(|(id, _)| *id != quote.user_id)
        .map(|(id, username)| format_user_display(*id, username.as_deref()))
        .collect();

    let num_options = std::cmp::min(3, other_users.len());
    let mut selected_others: Vec<String> = other_users
        .choose_multiple(&mut rng, num_options)
        .cloned()
        .collect();

    options.append(&mut selected_others);

    // Shuffle options so correct answer isn't always first
    options.shuffle(&mut rng);

    // Find the correct answer index
    let correct_option_id = options
        .iter()
        .position(|opt| opt == &correct_answer)
        .unwrap_or(0) as u8;

    let target_tz: Tz = match "Europe/Paris".parse() {
        Ok(tz) => tz,
        Err(_) => {
            log::error!("Failed to parse timezone identifier.");
            Tz::UTC
        }
    };
    let local_datetime = quote.message_date.with_timezone(&target_tz);
    let formatted_date = local_datetime.format("%b %d, %Y at %I:%M %p").to_string();

    // Send the poll
    bot.send_poll(
        msg.chat.id,
        format!("Who said this? (≖_≖)\n\"{}\"", quote.message_text),
        options,
    )
    .is_anonymous(false)
    .type_(teloxide::types::PollType::Quiz)
    .correct_option_id(correct_option_id)
    .explanation(format!("🗓️ Saved on {}", formatted_date))
    .await?;

    Ok(())
}

async fn handle_hug(bot: teloxide::Bot, msg: Message) -> ResponseResult<()> {
    const HUG_MESSAGES: &[&str] = &[
        "( っ˶´ ˘ `)っ",
        "♡⸜(ˆᗜˆ˵ )⸝♡",
        "(っᵔ◡ᵔ)っ",
        "(づ> v <)づ♡",
        "ʕっ•ᴥ•ʔっ ♡",
        "◝(ᵔᗜᵔ)◜",
        "(૭ ｡•̀ ᵕ •́｡ )૭",
        "(⊙ _ ⊙ )",
        "(◍•ᴗ•◍)♡",
        "≽^•⩊•^≼",
        "ᕙ(  •̀ ᗜ •́  )ᕗ",
        "( ⊃ ◕ _ ◕)⊃",
        "༼つ◕_◕༽つ",
        "(ㅅ´ ˘ `)",
        "(˵ •̀ ᴗ - ˵ ) ✧",
        "(❀❛ ֊ ❛„)♡",
    ];

    let mut rng = rand::rngs::StdRng::from_entropy();
    let index = rng.gen_range(0..HUG_MESSAGES.len());
    let text = HUG_MESSAGES[index];

    bot.send_message(msg.chat.id, text).await?;

    Ok(())
}

fn format_user_display(user_id: i64, username: Option<&str>) -> String {
    username
        .map(|u| format!("@{}", u))
        .unwrap_or_else(|| format!("User {}", user_id))
}

async fn check_admin(bot: &teloxide::Bot, msg: &Message, state: &Bot) -> bool {
    let user_id = msg.from().map(|u| u.id).unwrap_or(UserId(0));
    if user_id != state.admin_id {
        bot.send_message(
            msg.chat.id,
            "❌ This command can only be used by the bot admin ᕦ(ò_óˇ)ᕤ",
        )
        .await
        .log_on_error()
        .await;
        return false;
    }
    true
}

async fn handle_authorize(bot: teloxide::Bot, msg: Message, state: Bot) -> ResponseResult<()> {
    if !check_admin(&bot, &msg, &state).await {
        return Ok(());
    }

    let chat_id = msg.chat.id.0;

    let is_authorized = is_chat_authorized(&state.db, msg.chat.id).await;

    if is_authorized {
        bot.send_message(
            msg.chat.id,
            "⚠️ This chat is already authorized (っ º - º ς)",
        )
        .await?;
    } else {
        // Authorize
        sqlx::query("INSERT INTO authorized_chats (chat_id) VALUES (?)")
            .bind(chat_id)
            .execute(&state.db)
            .await
            .map_err(SqlxRequestError)?;

        bot.send_message(
            msg.chat.id,
            "✅ Chat authorized! ImmutableBot is now your buddy ദ്ദി ˉ͈̀꒳ˉ͈́ )✧".to_string(),
        )
        .await?;
    }

    Ok(())
}

async fn handle_deauthorize(bot: teloxide::Bot, msg: Message, state: Bot) -> ResponseResult<()> {
    if !check_admin(&bot, &msg, &state).await {
        return Ok(());
    }

    let chat_id = msg.chat.id.0;

    // Check if authorized
    let is_authorized = is_chat_authorized(&state.db, msg.chat.id).await;

    if !is_authorized {
        bot.send_message(
            msg.chat.id,
            "⚠️ This chat is not currently authorized (  •̀ω  •́  )",
        )
        .await?;
    } else {
        // De-authorize
        sqlx::query("DELETE FROM authorized_chats WHERE chat_id = ?")
            .bind(chat_id)
            .execute(&state.db)
            .await
            .map_err(SqlxRequestError)?;

        bot.send_message(
            msg.chat.id,
            "⛔ Chat de-authorized! ImmutableBot will no longer respond here (っ◞‸◟ c)".to_string(),
        )
        .await?;
    }

    Ok(())
}
