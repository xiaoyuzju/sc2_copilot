use std::{ffi::OsString, fs, io};

use sc2_copilot_app::Sc2Normalizer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = std::env::args_os().skip(1).collect::<Vec<_>>();
    if inputs.is_empty() || !inputs.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "用法：sc2-replay <game.json> <ui.json> [<game.json> <ui.json> ...]",
        )
        .into());
    }

    replay(inputs)
}

fn replay(inputs: Vec<OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let mut normalizer = Sc2Normalizer::default();
    for (index, pair) in inputs.chunks_exact(2).enumerate() {
        let game = fs::read(&pair[0])?;
        let ui = fs::read(&pair[1])?;
        let observation = normalizer.normalize(&game, &ui)?;
        println!("{index}: {observation:?}");
    }
    Ok(())
}
