use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup},
};

use crate::{
    BotState,
    db::{self, VoteSubject},
    handlers::banned_words::update_cache,
};

/// Helper to create the vote button layout
pub fn vote_keyboard(current_votes: i64, target_votes: i64) -> InlineKeyboardMarkup {
    let vote_label = format!("👍 Agree ({}/{})", current_votes, target_votes);

    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(vote_label, "vote_agree"),
        InlineKeyboardButton::callback("👑 Admin Approve", "vote_admin_approve"),
    ]])
}

/// Handles button presses for active proposals
pub async fn handle_vote_callback(
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

    let Some(target) = db::get_required_votes(&state.db, msg_id, chat_id).await else {
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

        match db::finalize_vote(&state.db, msg_id, chat_id).await {
            Ok(Some(subject)) => {
                let text = on_vote_passed(&state, chat_id, &subject).await;
                bot.answer_callback_query(q.id)
                    .text("✅ Admin approved!")
                    .await?;
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!("Proposal Approved by Admin! ✧٩(ˊᗜˋ*)و✧\n{text}"),
                    )
                    .await;
            }
            Ok(None) => {
                bot.answer_callback_query(q.id)
                    .text("⚠️ This vote is no longer active")
                    .await?;
            }
            Err(e) => {
                log::error!("finalize_vote failed: {e}");
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
                    match db::finalize_vote(&state.db, msg_id, chat_id).await {
                        Ok(Some(subject)) => {
                            let text = on_vote_passed(&state, chat_id, &subject).await;
                            bot.answer_callback_query(q.id)
                                .text("👍 Vote counted!")
                                .await?;
                            let _ = bot
                                .edit_message_text(
                                    msg.chat().id,
                                    msg.id(),
                                    format!(
                                        "🎉 Proposal Passed! ({}/{})\n{}",
                                        current_votes, target, text
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
                            log::error!("finalize_vote failed: {e}");
                            bot.answer_callback_query(q.id)
                                .text("⚠️ Something went wrong, try again")
                                .await?;
                        }
                    }
                } else {
                    bot.answer_callback_query(q.id)
                        .text("👍 Vote counted!")
                        .await?;
                    let keyboard = vote_keyboard(current_votes, target);
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

/// Runs the in-memory side effects of a vote that just passed (the database
/// part already happened in finalize_vote) and returns the text to show in chat.
async fn on_vote_passed(state: &BotState, chat_id: i64, subject: &VoteSubject) -> String {
    match subject {
        VoteSubject::Word { word, action } => {
            update_cache(state, chat_id, word, *action).await;
            format!("\"{}\" has been {}ned.", word, action.as_str())
        }
        VoteSubject::Quote { .. } => "🗑️ The quote has been removed.".to_string(),
    }
}

/// Opens a vote about `subject`. The calling feature does its own validation first.
/// `title` is the proposal text shown in chat, e.g. `Ban word "foo"`.
pub async fn open_vote(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    subject: VoteSubject,
    title: String,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let Some(proposer) = msg.from.as_ref() else {
        return Ok(());
    };
    let proposer_id = proposer.id.0 as i64;

    match db::has_open_vote(&state.db, chat_id, &subject).await {
        Ok(true) => {
            bot.send_message(
                msg.chat.id,
                "⚠️ There's already an open proposal for this (・_・ )",
            )
            .await?;
            return Ok(());
        }
        Ok(false) => {}
        Err(e) => {
            log::error!("has_open_vote failed: {e}");
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
    let sent = bot
        .send_message(msg.chat.id, format!("(人´꒳`*) Proposal: {title}"))
        .reply_markup(vote_keyboard(1, target))
        .await?;

    if let Err(e) = db::create_vote(
        &state.db,
        sent.id.0 as i64,
        chat_id,
        proposer_id,
        target,
        &subject,
    )
    .await
    {
        log::error!("create_vote failed: {e}");
        let _ = bot
            .edit_message_text(
                msg.chat.id,
                sent.id,
                "⚠️ Could not create the proposal, try again",
            )
            .await;
        return Ok(());
    }

    // The proposer's vote alone may be enough
    if target <= 1
        && let Ok(Some(passed)) = db::finalize_vote(&state.db, sent.id.0 as i64, chat_id).await
    {
        let text = on_vote_passed(state, chat_id, &passed).await;
        let _ = bot
            .edit_message_text(
                msg.chat.id,
                sent.id,
                format!("Proposal Passed! (1/1) °˖✧٩(´｡• ᵕ •｡`)۶✧˖°\n{text}"),
            )
            .await;
    }

    Ok(())
}
