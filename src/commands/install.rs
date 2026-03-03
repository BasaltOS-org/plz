use crate::commands::Command;
use crate::errors::{RuntimeSnafu, WhatError, WhereError};
use crate::metadata::get_packages;
use crate::settings::{SettingsJson, acquire_lock};
use crate::statebox::StateBox;
use crate::utils::{FuckWrap, PostAction, choice, specific_flag, yes_flag};

use snafu::ResultExt;
use tokio::runtime::Runtime;

pub fn build(hierarchy: &[String]) -> Command {
    Command::new(
        "install",
        vec![String::from("i")],
        "Install the application from a specified path",
        vec![specific_flag(), yes_flag()],
        None,
        run,
        hierarchy,
    )
}

fn run(states: &StateBox, args: Option<&[String]>) -> PostAction {
    match acquire_lock() {
        Ok(Some(action)) => return action,
        Err(fault) => {
            return PostAction::Fuck(WhatError::Install {
                source: WhereError::WrappedError { source: fault },
            });
        }
        _ => (),
    }
    let mut args = match args {
        None => return PostAction::NothingToDo,
        Some(args) => args.iter(),
    };
    print!("Reading sources...");
    let sources = match SettingsJson::get_settings() {
        Ok(settings) => settings.sources,
        Err(_) => return PostAction::PullSources,
    };
    if sources.is_empty() {
        return PostAction::PullSources;
    }
    let mut data = Vec::new();
    if states.get("specific").is_some_and(|x| *x) {
        while let Some(name) = args.next()
            && let Some(ver) = args.next()
        {
            data.push((name, Some(ver)));
        }
    } else {
        args.for_each(|x| data.push((x, None)));
    }
    let runtime = match Runtime::new().context(RuntimeSnafu).wrap() {
        Ok(runtime) => runtime,
        Err(source) => return PostAction::Fuck(WhatError::Install { source }),
    };
    let data = match runtime.block_on(get_packages(&data)) {
        Ok(data) => data,
        Err(source) => return PostAction::Fuck(WhatError::Install { source }),
    };
    println!();
    if data.is_empty() {
        return PostAction::NothingToDo;
    }
    println!(
        "\nThe following package(s) will be INSTALLED: \x1B[92m{}\x1B[0m",
        data.iter()
            .fold(String::new(), |acc, x| format!("{acc} {}", x.metadata.name))
            .trim()
    );
    if data.iter().any(|x| !x.run_deps.is_empty()) {
        println!(
            "The following package(s) will be MODIFIED:  \x1B[93m{}\x1B[0m",
            data.iter()
                .flat_map(|x| x.list_deps(true))
                .fold(String::new(), |acc, x| format!("{acc} {x}"))
                .trim()
        );
        if states.get("yes").is_none_or(|x: &bool| !*x) {
            match choice("Continue?", true) {
                Err(source) => {
                    return PostAction::Fuck(WhatError::Install {
                        source: WhereError::WrappedError { source },
                    });
                }
                Ok(false) => {
                    return PostAction::Fuck(WhatError::Install {
                        source: WhereError::other("Aborted."),
                    });
                }
                Ok(true) => (),
            };
        }
    }
    for data in data {
        if let Err(source) = runtime.block_on(data.install()) {
            return PostAction::Fuck(WhatError::Install { source });
        }
    }
    PostAction::Return
}
