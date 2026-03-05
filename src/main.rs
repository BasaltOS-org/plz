use std::{env, path::Path};

pub mod commands;
pub mod errors;
pub mod flags;
pub mod metadata;
pub mod settings;
pub mod statebox;
pub mod utils;

#[tokio::main]
pub async fn main() {
    let args: Vec<String> = env::args().collect();
    let mut args = args.iter();
    let name = args
        .next()
        .map(|arg| Path::new(arg).file_name().map(|x| x.to_str()))
        .unwrap_or(None)
        .unwrap_or(None)
        .unwrap_or("plz");
    // Main command
    let main_command = commands::Command::new(
        name,
        Vec::new(),
        "The PLZ package manager.",
        vec![],
        Some(vec![
            commands::configure::build,
            commands::unbind::build,
            commands::install::build,
            commands::plz_init::build,
            commands::remove::build_purge,
            commands::remove::build_remove,
            commands::update::build,
            commands::upgrade::build,
        ]),
        commands::CommandFunc::GetHelp,
        &[],
    );
    // Run the command with the provided arguments
    commands::Command::run(main_command, args).await;
}
