use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, Message, ReactionType},
};

use crate::{BotState, db};

/// Normalizer: collapse whitespace to a single space, trim, lowercase
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// True if `phrase` appears in `text`
fn contains_phrase(text: &str, phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    let need_before = phrase.chars().next().is_some_and(char::is_alphanumeric);
    let need_after = phrase
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);

    text.match_indices(phrase).any(|(i, _)| {
        let before_ok = !need_before
            || text[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = !need_after
            || text[i + phrase.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric());
        before_ok && after_ok
    })
}

/// Filter that checks messages against the chat's banned words list
pub async fn handle_banned_words_filter(bot: teloxide::Bot, msg: Message, state: BotState) {
    let Some(text) = msg.text() else {
        return;
    };

    let chat_id = msg.chat.id.0;

    let text = normalize(text);

    let hit = {
        let banned_map = state.banned_words.read().await;
        let Some(words) = banned_map.get(&chat_id) else {
            return;
        };
        words.iter().any(|w| contains_phrase(&text, w))
    };

    if hit
        && let Err(e) = bot
            .set_message_reaction(msg.chat.id, msg.id)
            .reaction(vec![ReactionType::Emoji {
                emoji: "🤨".to_string(),
            }])
            .await
    {
        log::warn!("set_message_reaction failed: {e}");
    }
}

/// Helper to create the vote button layout
fn create_vote_keyboard(current_votes: i64, target_votes: i64) -> InlineKeyboardMarkup {
    let vote_label = format!("👍 Agree ({}/{})", current_votes, target_votes);

    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(vote_label, "vote_agree"),
        InlineKeyboardButton::callback("👑 Admin Approve", "vote_admin_approve"),
    ]])
}

enum ProposalAction {
    Ban,
    Unban,
}

impl ProposalAction {
    fn as_str(&self) -> &'static str {
        match self {
            ProposalAction::Ban => "ban",
            ProposalAction::Unban => "unban",
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            ProposalAction::Ban => "Ban word",
            ProposalAction::Unban => "Unban word",
        }
    }
}

async fn propose_word_action(
    bot: teloxide::Bot,
    msg: Message,
    state: BotState,
    word_arg: String,
    action: ProposalAction,
) -> ResponseResult<()> {
    let word = normalize(&word_arg);
    if word.is_empty() || word.chars().count() > 50 {
        bot.send_message(
            msg.chat.id,
            format!(
                "⚠️ Specify a word or phrase (max 50 characters) to {} ( ˃ ꒳ ˂ )ﾉ",
                action.as_str()
            ),
        )
        .await?;
        return Ok(());
    }

    let chat_id = msg.chat.id.0;
    let Some(proposer) = msg.from.as_ref() else {
        return Ok(());
    };
    let proposer_id = proposer.id.0 as i64;

    // Check if action is valid
    {
        let map = state.banned_words.read().await;
        let is_banned = map.get(&chat_id).is_some_and(|set| set.contains(&word));

        match action {
            ProposalAction::Ban if is_banned => {
                bot.send_message(
                    msg.chat.id,
                    format!("⚠️ \"{}\" is already banned in this chat!  (￣ヘ￣)", word),
                )
                .await?;
                return Ok(());
            }
            ProposalAction::Unban if !is_banned => {
                bot.send_message(
                    msg.chat.id,
                    format!("⚠️ \"{}\" is not in the banned words list! (•̀⤙•́ )", word),
                )
                .await?;
                return Ok(());
            }
            _ => {}
        }
    }

    // Check if there's already an open proposal for this word
    match db::has_open_proposal(&state.db, chat_id, &word).await {
        Ok(true) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "⚠️ There's already an open proposal for \"{}\" (・_・ )",
                    word
                ),
            )
            .await?;
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => {
            log::error!("has_open_proposal failed: {e}");
            bot.send_message(msg.chat.id, "⚠️ Something went wrong, try again")
                .await?;
            return Ok(());
        }
    }

    // Determine required votes
    let target = match db::get_vote_threshold_setting(&state.db, chat_id).await {
        Some(custom) => custom,
        None => {
            let count = bot.get_chat_member_count(msg.chat.id).await.unwrap_or(4) as i64;
            std::cmp::max(1, count - 1)
        }
    };

    // Send a message with inline buttons for voting
    let keyboard = create_vote_keyboard(1, target);
    let sent = bot
        .send_message(
            msg.chat.id,
            format!("(人´꒳`*) Proposal: {} \"{}\"", action.display_name(), word),
        )
        .reply_markup(keyboard)
        .await?;

    // Create a vote session in the database
    if let Err(e) = db::create_vote_session(
        &state.db,
        sent.id.0 as i64,
        chat_id,
        &word,
        action.as_str(),
        proposer_id,
        target,
    )
    .await
    {
        log::error!("create_vote_session failed: {e}");
        let _ = bot
            .edit_message_text(
                msg.chat.id,
                sent.id,
                "⚠️ Could not create the proposal, try again",
            )
            .await;
        return Ok(());
    }

    // The proposer's vote alone may already be enough
    if target <= 1
        && let Ok(Some((word, action))) =
            db::finalize_proposal(&state.db, sent.id.0 as i64, chat_id).await
    {
        update_cache(&state, chat_id, &word, &action).await;
        let _ = bot
            .edit_message_text(
                msg.chat.id,
                sent.id,
                format!(
                    "🎉 Proposal Passed! (1/1)\n\"{}\" has been {}ned.",
                    word, action
                ),
            )
            .await;
    }

    Ok(())
}

pub async fn handle_propose_ban(
    bot: Bot,
    msg: Message,
    state: BotState,
    word: String,
) -> ResponseResult<()> {
    propose_word_action(bot, msg, state, word, ProposalAction::Ban).await
}

pub async fn handle_propose_unban(
    bot: Bot,
    msg: Message,
    state: BotState,
    word: String,
) -> ResponseResult<()> {
    propose_word_action(bot, msg, state, word, ProposalAction::Unban).await
}

/// Handles button presses for active proposals
pub async fn handle_word_vote_callback(
    bot: teloxide::Bot,
    q: CallbackQuery,
    state: BotState,
) -> ResponseResult<()> {
    let Some(data) = &q.data else {
        return Ok(());
    };
    let Some(msg) = &q.message else {
        return Ok(());
    };

    let chat_id = msg.chat().id.0;
    let msg_id = msg.id().0 as i64;
    let user_id = q.from.id.0 as i64;

    let required: Option<(i64,)> =
        sqlx::query_as("SELECT required FROM word_votes WHERE msg_id = ? AND chat_id = ?")
            .bind(msg_id)
            .bind(chat_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or_else(|e| {
                log::error!("proposal lookup failed: {e}");
                None
            });

    let Some((target,)) = required else {
        bot.answer_callback_query(q.id)
            .text("⚠️ This vote is no longer active ╮(￣▿￣)╭")
            .await?;
        return Ok(());
    };

    // 👑 Admin Approve
    if data == "vote_admin_approve" {
        if q.from.id != state.admin_id {
            bot.answer_callback_query(q.id)
                .text("⚠️ Only the admin can instant-approve proposals! (눈_눈)")
                .show_alert(true)
                .await?;
            return Ok(());
        }

        match db::finalize_proposal(&state.db, msg_id, chat_id).await {
            Ok(Some((word, action))) => {
                update_cache(&state, chat_id, &word, &action).await;
                bot.answer_callback_query(q.id)
                    .text("✅ Admin approved!")
                    .await?;
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "Proposal Approved by Admin! ✧٩(ˊᗜˋ*)و✧\n\"{}\" has been {}ned.",
                            word, action
                        ),
                    )
                    .await;
            }
            Ok(None) => {
                bot.answer_callback_query(q.id)
                    .text("⚠️ This vote is no longer active")
                    .await?;
            }
            Err(e) => {
                log::error!("finalize_proposal failed: {e}");
                bot.answer_callback_query(q.id)
                    .text("⚠️ Something went wrong, try again")
                    .await?;
            }
        }
        return Ok(());
    }

    // 👍 Agree
    if data == "vote_agree" {
        // Try registering user vote
        let vote_res = db::register_user_vote(&state.db, msg_id, chat_id, user_id).await;

        match vote_res {
            Ok(None) => {
                // Duplicate vote attempt blocked!
                bot.answer_callback_query(q.id)
                    .text("⚠️ You have already voted on this proposal!")
                    .show_alert(true)
                    .await?;
            }
            Ok(Some(current_votes)) => {
                if current_votes >= target {
                    // Threshold met -> try to close the proposal. Only one caller wins.
                    match db::finalize_proposal(&state.db, msg_id, chat_id).await {
                        Ok(Some((word, action))) => {
                            update_cache(&state, chat_id, &word, &action).await;
                            bot.answer_callback_query(q.id)
                                .text("👍 Vote counted!")
                                .await?;
                            let _ = bot
                                .edit_message_text(
                                    msg.chat().id,
                                    msg.id(),
                                    format!(
                                        "🎉 Proposal Passed! ({}/{})\n\"{}\" has been {}ned.",
                                        current_votes, target, word, action
                                    ),
                                )
                                .await;
                        }
                        Ok(None) => {
                            // Someone else closed it a moment ago
                            bot.answer_callback_query(q.id)
                                .text("⚠️ This vote is no longer active")
                                .await?;
                        }
                        Err(e) => {
                            log::error!("finalize_proposal failed: {e}");
                            bot.answer_callback_query(q.id)
                                .text("⚠️ Something went wrong, try again")
                                .await?;
                        }
                    }
                } else {
                    bot.answer_callback_query(q.id)
                        .text("👍 Vote counted!")
                        .await?;
                    let keyboard = create_vote_keyboard(current_votes, target);
                    let _ = bot
                        .edit_message_reply_markup(msg.chat().id, msg.id())
                        .reply_markup(keyboard)
                        .await;
                }
            }
            Err(_) => {
                bot.answer_callback_query(q.id)
                    .text("⚠️ Failed to record vote.")
                    .await?;
            }
        }
    }

    Ok(())
}

/// Update the in-memory cache. Call only after finalize_proposal succeeded.
async fn update_cache(state: &BotState, chat_id: i64, word: &str, action: &str) {
    let mut map = state.banned_words.write().await;
    if action == "ban" {
        map.entry(chat_id).or_default().insert(word.to_string());
    } else if let Some(set) = map.get_mut(&chat_id) {
        set.remove(word);
    }
}
