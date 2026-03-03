use std::{env, path::Path};

pub mod configure;
pub mod dew_init;
pub mod emancipate;
pub mod install;
pub mod remove;
pub mod update;
pub mod upgrade;

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
            configure::build,
            emancipate::build,
            install::build,
            dew_init::build,
            remove::build_purge,
            remove::build_remove,
            update::build,
            upgrade::build,
        ]),
        |_command, _args| utils::PostAction::GetHelp,
        &[],
    );
    // Run the command with the provided arguments
    main_command.run(args);
}
