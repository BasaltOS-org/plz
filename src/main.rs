use std::{env, path::Path};

pub mod commands;

pub fn main() {
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
        |_command, _args| utils::PostAction::GetHelp,
        &[],
    );
    // Run the command with the provided arguments
    main_command.run(args);
}
