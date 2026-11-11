use crate::handlers::voting;
use crate::{
    BotState,
    db::{VoteSubject, WordAction},
};
use teloxide::{
    prelude::*,
    types::{Message, ReactionType},
};

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

fn display_name(action: WordAction) -> &'static str {
    match action {
        WordAction::Ban => "Ban word",
        WordAction::Unban => "Unban word",
    }
}

async fn propose_word_action(
    bot: teloxide::Bot,
    msg: Message,
    state: BotState,
    word_arg: String,
    action: WordAction,
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

    // Check if action is valid
    {
        let map = state.banned_words.read().await;
        let is_banned = map.get(&chat_id).is_some_and(|set| set.contains(&word));

        match action {
            WordAction::Ban if is_banned => {
                bot.send_message(
                    msg.chat.id,
                    format!("⚠️ \"{}\" is already banned in this chat!  (￣ヘ￣)", word),
                )
                .await?;
                return Ok(());
            }
            WordAction::Unban if !is_banned => {
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

    let title = format!("{} \"{}\"", display_name(action), word);
    voting::open_vote(
        &bot,
        &msg,
        &state,
        VoteSubject::Word { word, action },
        title,
    )
    .await
}

pub async fn handle_propose_ban(
    bot: Bot,
    msg: Message,
    state: BotState,
    word: String,
) -> ResponseResult<()> {
    propose_word_action(bot, msg, state, word, WordAction::Ban).await
}

pub async fn handle_propose_unban(
    bot: Bot,
    msg: Message,
    state: BotState,
    word: String,
) -> ResponseResult<()> {
    propose_word_action(bot, msg, state, word, WordAction::Unban).await
}

/// Update the in-memory cache. Call only after pass_vote succeeded
pub(crate) async fn update_cache(state: &BotState, chat_id: i64, word: &str, action: WordAction) {
    let mut map = state.banned_words.write().await;
    match action {
        WordAction::Ban => {
            map.entry(chat_id).or_default().insert(word.to_string());
        }
        WordAction::Unban => {
            if let Some(set) = map.get_mut(&chat_id) {
                set.remove(word);
            }
        }
    }
}
