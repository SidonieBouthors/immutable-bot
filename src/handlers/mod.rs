pub mod admin;
pub mod banned_words;
pub mod hug;
pub mod link_rewrite;
pub mod quotes;
pub mod voting;

use crate::{BotState, db::is_chat_authorized};
use teloxide::{
    payloads::SetMyCommandsSetters,
    prelude::*,
    types::{BotCommandScope, Message, Recipient},
    utils::command::BotCommands,
};

use admin::{handle_authorize, handle_deauthorize, handle_set_threshold};
use banned_words::{handle_propose_ban, handle_propose_unban};
use hug::handle_hug;
use quotes::{handle_guesswho, handle_quote, handle_yeet};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
pub enum Command {
    #[command(description = "Display help message")]
    Help,
    #[command(description = "Save a quote (reply to a message)")]
    Quote,
    #[command(description = "Create a 'guess who said this' poll")]
    GuessWho,
    #[command(description = "Reply to a quote poll to propose removing that quote")]
    Yeet,
    #[command(description = "Send a group hug to everyone!")]
    Hug,
    #[command(description = "Propose banning a word (needs votes)")]
    Ban(String),
    #[command(description = "Propose unbanning a word (needs votes)")]
    Unban(String),
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
pub enum AdminCommand {
    #[command(description = "Admin: Authorize this chat for bot use")]
    Authorize,
    #[command(description = "Admin: Deauthorize this chat")]
    Deauthorize,
    #[command(description = "Admin: Set votes needed to ban / unban a word (number or 'default')")]
    SetVoteCount(String),
}

async fn check_authorized(
    bot: &teloxide::Bot,
    msg: &Message,
    state: &BotState,
) -> ResponseResult<bool> {
    if is_chat_authorized(&state.db, msg.chat.id).await {
        return Ok(true);
    }
    bot.send_message(
        msg.chat.id,
        "❌ This chat is not authorized to talk to me (╭ರ_•́)",
    )
    .await?;
    Ok(false)
}

pub async fn answer(
    bot: teloxide::Bot,
    msg: Message,
    cmd: Command,
    state: BotState,
) -> ResponseResult<()> {
    if !check_authorized(&bot, &msg, &state).await? {
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
        Command::Yeet => handle_yeet(bot, msg, state).await,
        Command::Hug => handle_hug(bot, msg).await,
        Command::Ban(word) => handle_propose_ban(bot, msg, state, word).await,
        Command::Unban(word) => handle_propose_unban(bot, msg, state, word).await,
    }
}

pub async fn answer_admin(
    bot: teloxide::Bot,
    msg: Message,
    cmd: AdminCommand,
    state: BotState,
) -> ResponseResult<()> {
    let requires_auth = !matches!(cmd, AdminCommand::Authorize | AdminCommand::Deauthorize);
    if requires_auth && !check_authorized(&bot, &msg, &state).await? {
        return Ok(());
    }

    match cmd {
        AdminCommand::Authorize => handle_authorize(bot, msg, state).await,
        AdminCommand::Deauthorize => handle_deauthorize(bot, msg, state).await,
        AdminCommand::SetVoteCount(arg) => handle_set_threshold(bot, msg, state, arg).await,
    }
}

/// Shows the admin the public and admin commands in this chat's menu
pub async fn register_admin_commands(bot: &teloxide::Bot, chat_id: ChatId, admin_id: UserId) {
    let mut commands = Command::bot_commands();
    commands.extend(AdminCommand::bot_commands());

    if let Err(e) = bot
        .set_my_commands(commands)
        .scope(BotCommandScope::ChatMember {
            chat_id: Recipient::Id(chat_id),
            user_id: admin_id,
        })
        .await
    {
        log::warn!("set_my_commands for the admin in chat {chat_id} failed: {e}");
    }
}
