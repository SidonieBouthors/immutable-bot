pub mod admin;
pub mod banned_words;
pub mod hug;
pub mod link_rewrite;
pub mod quotes;

use teloxide::{prelude::*, types::Message, utils::command::BotCommands};

use crate::{BotState, db::is_chat_authorized};

use admin::{handle_authorize, handle_deauthorize, handle_set_threshold};
use banned_words::{handle_propose_ban, handle_propose_unban};
use hug::handle_hug;
use quotes::{handle_guesswho, handle_quote};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
pub enum Command {
    #[command(description = "Display help message")]
    Help,
    #[command(description = "Save a quote (reply to a message)")]
    Quote,
    #[command(description = "Create a 'guess who said this' poll")]
    GuessWho,
    #[command(description = "Send a group hug to everyone!")]
    Hug,
    #[command(description = "Propose banning a word (needs votes)")]
    Ban(String),
    #[command(description = "Propose unbanning a word (needs votes)")]
    Unban(String),
    #[command(description = "Admin: Authorize this chat for bot use")]
    Authorize,
    #[command(description = "Admin: Deauthorize this chat")]
    Deauthorize,
    #[command(description = "Admin: Set votes needed to ban / unban a word (number or 'default')")]
    SetThreshold(String),
}

pub async fn answer(
    bot: teloxide::Bot,
    msg: Message,
    cmd: Command,
    state: BotState,
) -> ResponseResult<()> {
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
        Command::Ban(word) => handle_propose_ban(bot, msg, state, word).await,
        Command::Unban(word) => handle_propose_unban(bot, msg, state, word).await,
        Command::Authorize => handle_authorize(bot, msg, state).await,
        Command::Deauthorize => handle_deauthorize(bot, msg, state).await,
        Command::SetThreshold(arg) => handle_set_threshold(bot, msg, state, arg).await,
    }
}
