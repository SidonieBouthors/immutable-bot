use teloxide::{prelude::*, types::UserId};

use crate::BotState;

pub fn format_user_display(user_id: i64, username: Option<&str>) -> String {
    username
        .map(|u| format!("@{}", u))
        .unwrap_or_else(|| format!("User {}", user_id))
}

pub async fn check_admin(bot: &teloxide::Bot, msg: &Message, state: &BotState) -> bool {
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
