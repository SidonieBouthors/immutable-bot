use teloxide::{
    Bot,
    prelude::{Requester, ResponseResult},
    types::Message,
};

use crate::{
    BotState,
    db::{self, SqliteRequestError, is_chat_authorized},
    utils::check_admin,
};

pub async fn handle_authorize(
    bot: teloxide::Bot,
    msg: Message,
    state: BotState,
) -> ResponseResult<()> {
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
            .map_err(SqliteRequestError)?;

        bot.send_message(
            msg.chat.id,
            "✅ Chat authorized! ImmutableBot is now your buddy ദ്ദി ˉ͈̀꒳ˉ͈́ )✧".to_string(),
        )
        .await?;
    }

    Ok(())
}

pub async fn handle_deauthorize(
    bot: teloxide::Bot,
    msg: Message,
    state: BotState,
) -> ResponseResult<()> {
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
            .map_err(SqliteRequestError)?;

        bot.send_message(
            msg.chat.id,
            "⛔ Chat de-authorized! ImmutableBot will no longer respond here (っ◞‸◟ c)".to_string(),
        )
        .await?;
    }

    Ok(())
}

pub async fn handle_set_threshold(
    bot: Bot,
    msg: Message,
    state: BotState,
    arg: String,
) -> ResponseResult<()> {
    if !check_admin(&bot, &msg, &state).await {
        return Ok(());
    }

    let arg = arg.trim();
    let votes = if arg.eq_ignore_ascii_case("default") {
        None
    } else {
        match arg.parse::<i64>() {
            Ok(n) if n >= 1 => Some(n),
            _ => {
                bot.send_message(
                    msg.chat.id,
                    "⚠️ Usage: /setthreshold <number ≥ 1 | default>",
                )
                .await?;
                return Ok(());
            }
        }
    };

    match db::set_vote_threshold(&state.db, msg.chat.id.0, votes).await {
        Ok(true) => {
            bot.send_message(
                msg.chat.id,
                "✅ Vote threshold updated (applies to new proposals)",
            )
            .await?;
        }
        Ok(false) => {
            bot.send_message(msg.chat.id, "⚠️ This chat is not authorized")
                .await?;
        }
        Err(e) => {
            log::error!("set_vote_threshold failed: {e}");
            bot.send_message(msg.chat.id, "⚠️ Something went wrong, try again")
                .await?;
        }
    }
    Ok(())
}
