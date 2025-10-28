use teloxide::{
    prelude::{Requester, ResponseResult},
    types::Message,
};

use crate::{BotState, db::SqliteRequestError, db::is_chat_authorized, utils::check_admin};

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
