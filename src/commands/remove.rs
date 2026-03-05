use snafu::location;

use crate::commands::Command;
use crate::errors::{Wrapped, WrappedError};
use crate::metadata::get_local_pkgs;
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{PostAction, choice, specific_flag, yes_flag};

pub fn build_remove(hierarchy: &[String]) -> Command {
    Command::new(
        "remove",
        vec![String::from("r")],
        "Removes a package, whilst maintaining any user-made configurations",
        vec![specific_flag(), yes_flag()],
        None,
        crate::commands::CommandFunc::Remove,
        hierarchy,
    )
}

pub fn build_purge(hierarchy: &[String]) -> Command {
    Command::new(
        "purge",
        vec![String::from("p")],
        "Removes a package, WITHOUT maintaining any user-made configurations",
        vec![specific_flag(), yes_flag()],
        None,
        crate::commands::CommandFunc::Purge,
        hierarchy,
    )
}

// fn remove(rt: &Runtime, states: &StateBox, args: Option<&[String]>) -> PostAction {
//     run(rt, states, args, false)
// }

// fn purge(rt: &Runtime, states: &StateBox, args: Option<&[String]>) -> PostAction {
//     run(rt, states, args, true)
// }

pub async fn run(states: &StateBox, args: Option<&[String]>, purge: bool) -> PostAction {
    match internal_run(states, args, purge).await {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
async fn internal_run(
    states: &StateBox,
    args: Option<&[String]>,
    purge: bool,
) -> Result<PostAction, WrappedError> {
    if let Some(action) = acquire_lock().await.wrap(location!())? {
        return Ok(action);
    };
    let mut args = match args {
        None => return Ok(PostAction::NothingToDo),
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
    let metadatas = get_local_pkgs(&data).await.wrap(location!())?;
    println!();
    if metadatas.is_empty() {
        return Ok(PostAction::NothingToDo);
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
                Err(source) => return Err(source),
                Ok(false) => {
                    return Err(WrappedError::Other {
                        error: "Operation aborted by user.".into(),
                        loc: location!(),
                    });
                }
                Ok(true) => (),
            };
        }
    }
    for package in metadatas.primary {
        package.remove(purge, None).await.wrap(location!())?;
    }
    Ok(PostAction::Return)
}
