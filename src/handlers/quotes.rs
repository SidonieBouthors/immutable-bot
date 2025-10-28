use chrono_tz::Tz;
use rand::{SeedableRng, seq::SliceRandom};
use teloxide::{
    payloads::SendPollSetters,
    prelude::{Requester, ResponseResult},
    types::{ForwardedFrom, Message},
};

use crate::{BotState, db::Quote, utils::format_user_display};

pub async fn handle_quote(bot: teloxide::Bot, msg: Message, state: BotState) -> ResponseResult<()> {
    let Some(replied_msg) = msg.reply_to_message() else {
        bot.send_message(
            msg.chat.id,
            "⚠️ Please reply to a message with /quote to save it ꉂ(˵˃ ᗜ ˂˵)",
        )
        .await?;
        return Ok(());
    };

    let Some(text) = replied_msg.text() else {
        bot.send_message(msg.chat.id, "⚠️ Can only save text messages (ᵕ—ᴗ—)")
            .await?;
        return Ok(());
    };

    let (user_id, username);
    let original_date = replied_msg.date;
    let chat_id = msg.chat.id.0;

    match msg.forward_from() {
        Some(ForwardedFrom::User(u)) => {
            user_id = u.id.0 as i64;
            username = u.username.clone();
        }
        _ => {
            user_id = replied_msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
            username = replied_msg.from().and_then(|u| u.username.clone());
        }
    };

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

    Ok(())
}

pub async fn handle_guesswho(
    bot: teloxide::Bot,
    msg: Message,
    state: BotState,
) -> ResponseResult<()> {
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
    .explanation(format!("🗓️ Quote from {}", formatted_date))
    .await?;

    Ok(())
}
