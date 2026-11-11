use teloxide::{
    prelude::*,
    types::{CallbackQueryId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId},
};

use crate::{
    BotState,
    db::{self, VoteSubject},
    handlers::banned_words::update_cache,
};

/// Helper to create the vote button layout
pub fn vote_keyboard(agree: i64, against: i64, target: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(format!("👍 Agree ({agree}/{target})"), "vote_agree"),
            InlineKeyboardButton::callback(
                format!("👎 Against ({against}/{target})"),
                "vote_against",
            ),
        ],
        vec![
            InlineKeyboardButton::callback("👑 Admin Approve", "vote_admin_approve"),
            InlineKeyboardButton::callback("🧨 Admin Reject", "vote_admin_reject"),
        ],
    ])
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

    // 👑 Admin Approve / Reject
    if data == "vote_admin_approve" || data == "vote_admin_reject" {
        let approve = data == "vote_admin_approve";

        if q.from.id != state.admin_id {
            let verb = if approve { "approve" } else { "reject" };
            bot.answer_callback_query(q.id)
                .text(format!(
                    "⚠️ Only the admin can instant-{verb} proposals! (눈_눈)"
                ))
                .show_alert(true)
                .await?;
            return Ok(());
        }

        if approve {
            return close_vote(
                &bot,
                &q,
                &state,
                Outcome::Passed,
                "✅ Admin approved!",
                "Proposal Approved by Admin! ✧٩(ˊᗜˋ*)و✧",
            )
            .await;
        }
        return close_vote(
            &bot,
            &q,
            &state,
            Outcome::Rejected,
            "❌ Admin rejected!",
            "Proposal Rejected by Admin! (╯°□°)╯",
        )
        .await;
    }

    // 👍 Agree
    if data == "vote_agree" {
        match db::cast_vote(&state.db, msg_id, chat_id, user_id, true).await {
            Ok(None) => {
                // Same direction as their existing vote
                bot.answer_callback_query(q.id)
                    .text("⚠️ You have already voted for this proposal!")
                    .show_alert(true)
                    .await?;
            }
            Ok(Some(tally)) if tally.agree >= target => {
                return close_vote(
                    &bot,
                    &q,
                    &state,
                    Outcome::Passed,
                    "👍 Vote counted!",
                    &passed_header(tally.agree, target),
                )
                .await;
            }
            Ok(Some(tally)) => {
                bot.answer_callback_query(q.id)
                    .text("👍 Vote counted!")
                    .await?;
                let _ = bot
                    .edit_message_reply_markup(msg.chat().id, msg.id())
                    .reply_markup(vote_keyboard(tally.agree, tally.against, target))
                    .await;
            }
            Err(e) => {
                log::error!("cast_vote failed: {e}");
                bot.answer_callback_query(q.id)
                    .text("⚠️ Failed to record vote.")
                    .await?;
            }
        }
        return Ok(());
    }

    // 👎 Against
    if data == "vote_against" {
        match db::cast_vote(&state.db, msg_id, chat_id, user_id, false).await {
            Ok(None) => {
                // Same direction as their existing vote
                bot.answer_callback_query(q.id)
                    .text("⚠️ You have already voted against this proposal!")
                    .show_alert(true)
                    .await?;
            }
            Ok(Some(tally)) if tally.against >= target => {
                // Enough votes against -> close without applying anything
                return close_vote(
                    &bot,
                    &q,
                    &state,
                    Outcome::Rejected,
                    "👎 Vote counted!",
                    &rejected_header(tally.against, target),
                )
                .await;
            }
            Ok(Some(tally)) => {
                bot.answer_callback_query(q.id)
                    .text("👎 Vote counted!")
                    .await?;
                let _ = bot
                    .edit_message_reply_markup(msg.chat().id, msg.id())
                    .reply_markup(vote_keyboard(tally.agree, tally.against, target))
                    .await;
            }
            Err(e) => {
                log::error!("cast_vote failed: {e}");
                bot.answer_callback_query(q.id)
                    .text("⚠️ Failed to record vote.")
                    .await?;
            }
        }
    }

    Ok(())
}

/// Runs the in-memory side effects of a vote that just passed (the database
/// part already happened in pass_vote) and returns the text to show in chat.
async fn on_vote_passed(state: &BotState, chat_id: i64, subject: &VoteSubject) -> String {
    match subject {
        VoteSubject::Word { word, action } => {
            update_cache(state, chat_id, word, *action).await;
            format!("\"{}\" has been {}ned.", word, action.as_str())
        }
        VoteSubject::Quote { text, author, .. } => {
            format!("🗑️ Quote removed:\n{}", quote_line(text, author))
        }
    }
}
async fn on_vote_rejected(_state: &BotState, _chat_id: i64, subject: &VoteSubject) -> String {
    match subject {
        VoteSubject::Word { word, action } => {
            format!("\"{}\" will not be {}ed.", word, action.as_str())
        }
        VoteSubject::Quote { text, author, .. } => {
            format!("♻️ Quote kept:\n{}", quote_line(text, author))
        }
    }
}

fn passed_header(agree: i64, target: i64) -> String {
    format!("Proposal Passed! ({agree}/{target}) °˖✧٩(´｡• ᵕ •｡`)۶✧˖°")
}

fn rejected_header(against: i64, target: i64) -> String {
    format!("Proposal Rejected! ({against}/{target}) (╥﹏╥)")
}

fn quote_line(text: &str, author: &str) -> String {
    if author.is_empty() {
        format!("\"{text}\"")
    } else {
        format!("\"{text}\" — {author}")
    }
}

/// How a vote ended
enum Outcome {
    Passed,
    Rejected,
}

/// Closes a vote
async fn close_vote(
    bot: &Bot,
    q: &CallbackQuery,
    state: &BotState,
    outcome: Outcome,
    answer_text: &str,
    header: &str,
) -> ResponseResult<()> {
    let Some(msg) = &q.message else {
        return Ok(());
    };
    let chat = msg.chat().id;
    let message = msg.id();

    // Some(body) only for the caller that actually closed the vote
    let closed: Result<Option<String>, sqlx::Error> = match outcome {
        Outcome::Passed => match db::pass_vote(&state.db, message.0 as i64, chat.0).await {
            Ok(Some(subject)) => Ok(Some(on_vote_passed(state, chat.0, &subject).await)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        },
        Outcome::Rejected => match db::reject_vote(&state.db, message.0 as i64, chat.0).await {
            Ok(Some(subject)) => Ok(Some(on_vote_rejected(state, chat.0, &subject).await)),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        },
    };

    match closed {
        Ok(Some(body)) => {
            bot.answer_callback_query(q.id.clone())
                .text(answer_text)
                .await?;
            let _ = bot
                .edit_message_text(chat, message, format!("{header}\n{body}"))
                .await;
        }
        Ok(None) => {
            // Someone else closed it a moment ago
            bot.answer_callback_query(q.id.clone())
                .text("⚠️ This vote is no longer active")
                .await?;
        }
        Err(e) => {
            log::error!("closing the vote failed: {e}");
            bot.answer_callback_query(q.id.clone())
                .text("⚠️ Something went wrong, try again")
                .await?;
        }
    }
    Ok(())
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
        .reply_markup(vote_keyboard(1, 0, target))
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
        && let Ok(Some(passed)) = db::pass_vote(&state.db, sent.id.0 as i64, chat_id).await
    {
        let text = on_vote_passed(state, chat_id, &passed).await;
        let _ = bot
            .edit_message_text(
                msg.chat.id,
                sent.id,
                format!("{}\n{text}", passed_header(1, 1)),
            )
            .await;
    }

    Ok(())
}
