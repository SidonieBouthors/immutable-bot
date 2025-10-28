use rand::{Rng, SeedableRng};
use teloxide::{
    prelude::{Requester, ResponseResult},
    types::Message,
};

pub async fn handle_hug(bot: teloxide::Bot, msg: Message) -> ResponseResult<()> {
    const HUG_MESSAGES: &[&str] = &[
        "( っ˶´ ˘ `)っ",
        "♡⸜(ˆᗜˆ˵ )⸝♡",
        "(っᵔ◡ᵔ)っ",
        "(づ> v <)づ♡",
        "ʕっ•ᴥ•ʔっ ♡",
        "◝(ᵔᗜᵔ)◜",
        "(૭ ｡•̀ ᵕ •́｡ )૭",
        "(⊙ _ ⊙ )",
        "(◍•ᴗ•◍)♡",
        "≽^•⩊•^≼",
        "ᕙ(  •̀ ᗜ •́  )ᕗ",
        "( ⊃ ◕ _ ◕)⊃",
        "༼つ◕_◕༽つ",
        "(ㅅ´ ˘ `)",
        "(˵ •̀ ᴗ - ˵ ) ✧",
        "(❀❛ ֊ ❛„)♡",
    ];

    let mut rng = rand::rngs::StdRng::from_entropy();
    let index = rng.gen_range(0..HUG_MESSAGES.len());
    let text = HUG_MESSAGES[index];

    bot.send_message(msg.chat.id, text).await?;

    Ok(())
}
