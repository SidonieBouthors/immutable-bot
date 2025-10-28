mod db;
mod handlers;
mod utils;

use sqlx::sqlite::SqlitePool;
use teloxide::prelude::*;

use crate::handlers::{Command, answer, link_rewrite::handle_link_rewrite};

#[derive(Clone)]
struct BotState {
    db: SqlitePool,
    admin_id: UserId,
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    pretty_env_logger::init();
    log::info!("Starting Immutable Bot...");

    // Get Admin ID from environment variable
    let admin_id_str = std::env::var("ADMIN_ID")
        .expect("ADMIN_ID env variable not set. Please set the bot owner's Telegram User ID.");
    let admin_id: UserId = UserId(admin_id_str.parse::<u64>().unwrap_or_else(|_| {
        panic!("Failed to parse ADMIN_ID as a positive integer.");
    }));

    // Initialize database
    let db = SqlitePool::connect("sqlite:data/quotes.db?mode=rwc")
        .await
        .expect("Failed to connect to database");

    db::create_tables(&db)
        .await
        .expect("Failed to create database tables");

    let bot = teloxide::Bot::from_env();
    let bot_state = BotState { db, admin_id };

    log::info!("Bot started successfully!");


    let handler = dptree::entry()
        // Command handlers
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(answer),
        )
        // Message handlers
        .branch(
            Update::filter_message()
                .filter_map(|msg: Message| msg.text().map(ToOwned::to_owned))
                .endpoint(handle_link_rewrite),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![bot_state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    log::info!("Bot shutdown complete.");
}
