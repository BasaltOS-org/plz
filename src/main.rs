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
        .unwrap_or("dew");
    // Main command
    let main_command = commands::Command::new(
        name,
        Vec::new(),
        "The DEW package manager.",
        vec![],
        Some(vec![
            commands::configure::build,
            commands::unbind::build,
            commands::install::build,
            commands::dew_init::build,
            commands::remove::build_purge,
            commands::remove::build_remove,
            commands::update::build,
            commands::upgrade::build,
        ]),
        |_, _command, _args| utils::PostAction::GetHelp,
        &[],
    );
    // Run the command with the provided arguments
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            println!("Tokio is not supported on this system. This program cannot run. {error:?}");
            return;
        }
    };
    commands::Command::run(main_command, &rt, args).await;
}
