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

            let original_url = if original_url.contains("://") {
                original_url
            } else {
                format!("https://{original_url}")
            };

            if let Some(sanitized_link) = sanitize_link(&original_url) {
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

/// Params that are trackers on essentially every site and never functional.
const TRACKER_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "msclkid", "yclid", "twclid", "igshid", "mc_cid", "mc_eid", "_ga",
    "_gl",
];
const TRACKER_PREFIXES: &[&str] = &["utm_"];

fn is_global_tracker(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    TRACKER_PARAMS.contains(&key.as_str()) || TRACKER_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Per-site allowed / disallowed params
enum ParamPolicy {
    Global,                          // Only remove global trackers
    StripAll,                        // Remove all params
    Keep(&'static [&'static str]),   // Keep only these
    Remove(&'static [&'static str]), // Remove only these
}

impl ParamPolicy {
    fn should_remove(&self, key: &str) -> bool {
        is_global_tracker(key)
            || match self {
                ParamPolicy::Global => false,
                ParamPolicy::StripAll => true,
                ParamPolicy::Keep(keep) => !keep.contains(&key),
                ParamPolicy::Remove(remove) => remove.contains(&key),
            }
    }
}

struct Rule {
    hosts: &'static [&'static str],
    matches: fn(&[&str]) -> bool,
    params: ParamPolicy,
    rewrite_host: Option<&'static str>,
}

fn any_path(_: &[&str]) -> bool {
    true
}

fn twitter_post(segments: &[&str]) -> bool {
    // /{user}/status/{id} and /i/status/{id}
    matches!(segments, [_, "status", ..])
}

const RULES: &[Rule] = &[
    Rule {
        hosts: &["x.com", "twitter.com", "mobile.twitter.com"],
        matches: twitter_post,
        params: ParamPolicy::StripAll,
        rewrite_host: Some("fxtwitter.com"),
    },
    Rule {
        hosts: &["instagram.com"],
        matches: any_path,
        params: ParamPolicy::StripAll,
        rewrite_host: Some("eeinstagram.com"),
    },
    Rule {
        hosts: &["reddit.com", "redd.it"],
        matches: any_path,
        params: ParamPolicy::StripAll,
        rewrite_host: Some("rxddit.com"),
    },
    Rule {
        hosts: &["bsky.app"],
        matches: any_path,
        params: ParamPolicy::StripAll,
        rewrite_host: Some("fxbsky.app"),
    },
    Rule {
        hosts: &["tiktok.com", "vm.tiktok.com"],
        matches: any_path,
        params: ParamPolicy::StripAll,
        rewrite_host: Some("tiktokez.com"),
    },
    Rule {
        hosts: &[
            "youtube.com",
            "m.youtube.com",
            "music.youtube.com",
            "youtu.be",
        ],
        matches: any_path,
        params: ParamPolicy::Remove(&["si", "feature", "pp"]),
        rewrite_host: None,
    },
    Rule {
        hosts: &["open.spotify.com"],
        matches: any_path,
        params: ParamPolicy::Remove(&["si", "context"]),
        rewrite_host: None,
    },
];

fn find_rule(host: &str, segments: &[&str]) -> Option<&'static Rule> {
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    RULES
        .iter()
        .find(|rule| rule.hosts.contains(&host) && (rule.matches)(segments))
}

fn strip_params(url: &mut Url, policy: &ParamPolicy) {
    let Some(query) = url.query() else { return };

    let kept = query
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(k, _)| k);
            !policy.should_remove(key)
        })
        .collect::<Vec<_>>();

    let new_query = kept.join("&");
    if new_query != query {
        url.set_query((!new_query.is_empty()).then_some(new_query.as_str()));
    }
}

fn sanitize_link(original_link: &str) -> Option<String> {
    let original = Url::parse(original_link).ok()?;
    let mut url = original.clone();

    let rule = url.host_str().and_then(|host| {
        let segments: Vec<&str> = url.path_segments()?.collect();
        find_rule(host, &segments)
    });

    let policy = rule.map_or(&ParamPolicy::Global, |r| &r.params);
    strip_params(&mut url, policy);

    if let Some(target) = rule.and_then(|r| r.rewrite_host) {
        url.set_host(Some(target)).ok()?;
        if url.scheme() == "http" {
            url.set_scheme("https").ok()?;
        }
    }

    (url != original).then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::sanitize_link;

    #[test]
    fn ignores_bare_domain() {
        assert_eq!(sanitize_link("https://example.com"), None);
    }

    #[test]
    fn ignores_twitter_hashtag() {
        assert_eq!(
            sanitize_link("https://x.com/hashtag/rust?src=hashtag_click"),
            None
        );
    }

    #[test]
    fn rewrites_tweet() {
        assert_eq!(
            sanitize_link("https://x.com/user/status/123?s=20").as_deref(),
            Some("https://fxtwitter.com/user/status/123")
        );
    }

    #[test]
    fn keeps_functional_params_on_unknown_sites() {
        assert_eq!(
            sanitize_link("https://example.com/p?id=5&utm_source=x").as_deref(),
            Some("https://example.com/p?id=5")
        );
        assert_eq!(sanitize_link("https://example.com/p?id=5"), None);
    }

    #[test]
    fn preserves_encoding_of_kept_params() {
        assert_eq!(
            sanitize_link("https://example.com/s?q=a%20b&fbclid=1").as_deref(),
            Some("https://example.com/s?q=a%20b")
        );
    }
}
