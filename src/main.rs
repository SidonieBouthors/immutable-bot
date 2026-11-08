mod db;
mod handlers;
mod utils;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use sqlx::sqlite::SqlitePool;
use teloxide::prelude::*;
use tokio::sync::RwLock;

use crate::handlers::{Command, answer, link_rewrite::handle_link_rewrite};

#[derive(Clone)]
struct BotState {
    db: SqlitePool,
    admin_id: UserId,
    banned_words: Arc<RwLock<HashMap<i64, HashSet<String>>>>,
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting Immutable Bot...");

    // Get Admin ID from environment variable
    let admin_id_str = std::env::var("ADMIN_ID")
        .expect("ADMIN_ID env variable not set. Please set the bot owner's Telegram User ID.");
    let admin_id: UserId = UserId(
        admin_id_str
            .parse::<u64>()
            .expect("Failed to parse ADMIN_ID as a positive integer."),
    );

    // Get Database URL from environment variable
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL env variable not set. Please set the database URL.");

    // Initialize database
    let db = SqlitePool::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    db::create_tables(&db)
        .await
        .expect("Failed to create database tables");

    // Load banned words from the database
    let banned_words = db::load_banned_words(&db)
        .await
        .expect("Failed to load banned words");

    let bot = teloxide::Bot::from_env();
    let bot_state = BotState {
        db,
        admin_id,
        banned_words: Arc::new(RwLock::new(banned_words)),
    };

    log::info!("Bot started successfully!");

    let handler = dptree::entry()
        // Command handlers
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(answer),
        )
        // Banned words filter
        .branch(
            Update::filter_message()
                .inspect_async(handlers::banned_words::handle_banned_words_filter),
        )
        // Link rewriting
        .branch(
            Update::filter_message()
                .filter_map(|msg: Message| msg.text().map(ToOwned::to_owned))
                .endpoint(handle_link_rewrite),
        )
        // Button presses
        .branch(Update::filter_callback_query().endpoint(handlers::voting::handle_vote_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![bot_state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    log::info!("Bot shutdown complete.");
}
