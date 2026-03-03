use metadata::get_local_pkgs;
use settings::acquire_lock;
use snafu::ResultExt;
use statebox::StateBox;
use tokio::runtime::Runtime;
use utils::{
    FuckWrap, PostAction, choice,
    errors::{RuntimeSnafu, WhatError, WhereError},
};

use crate::commands::Command;

pub fn build_remove(hierarchy: &[String]) -> Command {
    Command::new(
        "remove",
        vec![String::from("r")],
        "Removes a package, whilst maintaining any user-made configurations",
        vec![utils::specific_flag(), utils::yes_flag()],
        None,
        remove,
        hierarchy,
    )
}

pub fn build_purge(hierarchy: &[String]) -> Command {
    Command::new(
        "purge",
        vec![String::from("p")],
        "Removes a package, WITHOUT maintaining any user-made configurations",
        vec![utils::specific_flag(), utils::yes_flag()],
        None,
        purge,
        hierarchy,
    )
}

fn remove(states: &StateBox, args: Option<&[String]>) -> PostAction {
    run(states, args, false)
}

fn purge(states: &StateBox, args: Option<&[String]>) -> PostAction {
    run(states, args, true)
}

fn run(states: &StateBox, args: Option<&[String]>, purge: bool) -> PostAction {
    match acquire_lock() {
        Ok(Some(action)) => return action,
        Err(source) => {
            return PostAction::Fuck(WhatError::Remove {
                source: WhereError::WrappedError { source },
            });
        }
        _ => (),
    }
    let mut args = match args {
        None => return PostAction::NothingToDo,
        Some(args) => args.iter(),
    };
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
        Err(source) => return PostAction::Fuck(WhatError::Remove { source }),
    };
    match runtime.block_on(get_local_pkgs(&data)) {
        Ok(metadatas) => {
            println!();
            if metadatas.is_empty() {
                return PostAction::NothingToDo;
            }
            let msg = if purge { "PURGED: " } else { "REMOVED:" };
            println!(
                "\nThe following package(s) will be {msg}  \x1B[91m{}\x1B[0m",
                metadatas
                    .primary
                    .iter()
                    .fold(String::new(), |acc, x| format!("{acc} {}", x.name))
                    .trim()
            );
            if metadatas.has_deps() {
                println!(
                    "The following package(s) will be MODIFIED: \x1B[93m{}\x1B[0m",
                    metadatas
                        .secondary
                        .iter()
                        .fold(String::new(), |acc, x| format!("{acc} {}", x.name))
                        .trim()
                );
                if states.get("yes").is_none_or(|x: &bool| !*x) {
                    match choice("Continue?", true) {
                        Err(source) => {
                            return PostAction::Fuck(WhatError::Remove {
                                source: WhereError::WrappedError { source },
                            });
                        }
                        Ok(false) => {
                            return PostAction::Fuck(WhatError::Remove {
                                source: WhereError::other("Aborted."),
                            });
                        }
                        Ok(true) => (),
                    };
                }
            }
            for package in metadatas.primary {
                if let Err(source) = runtime.block_on(package.remove(purge, None)) {
                    return PostAction::Fuck(WhatError::Remove { source });
                };
            }
            PostAction::Return
        }
        Err(source) => PostAction::Fuck(WhatError::Remove { source }),
    }
}
