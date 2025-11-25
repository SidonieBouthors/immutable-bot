use log::warn;
use teloxide::{
    prelude::{Requester, ResponseResult},
    sugar::request::{RequestLinkPreviewExt, RequestReplyExt},
    types::{Message, MessageEntityKind},
};
use url::{Url, form_urlencoded};

pub async fn handle_link_rewrite(bot: teloxide::Bot, msg: Message) -> ResponseResult<()> {
    // Ignore non-text messages
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()),
    };

    if let Some(entities) = msg.entities() {
        for entity in entities {
            let original_url = match &entity.kind {
                // Plain URL
                MessageEntityKind::Url => {
                    let utf16 = text
                        .encode_utf16()
                        .skip(entity.offset)
                        .take(entity.length)
                        .collect::<Vec<_>>();

                    match String::from_utf16(&utf16) {
                        Ok(url) => url,
                        Err(e) => {
                            warn!(r#"Unable to convert link from "{text}" to UTF-16 string: {e}"#);
                            continue;
                        }
                    }
                }

                // Hyperlink
                MessageEntityKind::TextLink { url } => url.to_string(),
                _ => continue,
            };

            if let Some(sanitized_link) = sanitize_link(&original_url)
                && sanitized_link != original_url
            {
                bot.send_message(
                    msg.chat.id,
                    format!("🔗 Better Link:\n{}", sanitized_link.trim()),
                )
                .reply_to(msg.id)
                .disable_link_preview(false)
                .await?;
            }
        }
    }

    Ok(())
}

const ALLOWED_PARAMS: &[&str] = &["v"];

fn sanitize_link(original_link: &str) -> Option<String> {
    let mut url = Url::parse(original_link).ok()?;

    // Remove tracking parameters
    let mut cleaned_query: Vec<(String, String)> = Vec::new();
    let query_pairs = url.query_pairs().into_owned();
    for (key, value) in query_pairs {
        if ALLOWED_PARAMS.contains(&key.as_str()) {
            cleaned_query.push((key, value));
        }
    }

    // Rebuild the query string
    if cleaned_query.is_empty() {
        url.set_query(None);
    } else {
        let new_query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(cleaned_query)
            .finish();
        url.set_query(Some(&new_query));
    }

    // Rewrite domains
    let host = url.host_str()?.to_lowercase();
    let new_host = match host.as_str() {
        // X/Twitter
        "x.com" | "twitter.com" => Some("fxtwitter.com"),
        // Instagram
        "instagram.com" | "www.instagram.com" => Some("eeinstagram.com"),
        // Reddit
        "reddit.com" | "www.reddit.com" | "redd.it" => Some("rxddit.com"),
        // Bluesky
        "bsky.app" | "www.bsky.app" => Some("fxbsky.app"),
        // TikTok
        "tiktok.com" | "www.tiktok.com" | "vm.tiktok.com" => Some("tiktokez.com"),
        _ => None,
    };
    if let Some(h) = new_host {
        url.set_host(Some(h)).ok()?;
    }

    // Use HTTPS
    if url.scheme() == "http" {
        url.set_scheme("https").ok()?;
    }

    Some(url.to_string())
}
