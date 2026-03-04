use snafu::location;
use tokio::runtime::Runtime;

use crate::commands::Command;
use crate::errors::{Wrapped, WrappedError};
use crate::metadata::get_packages;
use crate::settings::{SettingsJson, acquire_lock};
use crate::statebox::StateBox;
use crate::utils::{PostAction, choice, specific_flag, yes_flag};

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

fn run(rt: &Runtime, states: &StateBox, args: Option<&[String]>) -> PostAction {
    match rt.block_on(async {
        if let Some(action) = acquire_lock().await.wrap()? {
            return Ok(action);
        };
        let mut args = match args {
            None => return Ok(PostAction::NothingToDo),
            Some(args) => args.iter(),
        };
        print!("Reading sources...");
        let sources = SettingsJson::get_settings().await.wrap()?.sources;
        if sources.is_empty() {
            return Ok(PostAction::PullSources);
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
        let data = get_packages(&data).await.wrap()?;
        println!();
        if data.is_empty() {
            return Ok(PostAction::NothingToDo);
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
            if states.get("yes").is_none_or(|x: &bool| !*x) && !choice("Continue?", true).wrap()? {
                return Err(WrappedError::Other {
                    error: "Operation aborted by user.".into(),
                    loc: location!(),
                });
            }
        }
        for data in data {
            data.install().await.wrap()?;
        }
        Ok(PostAction::Return)
    }) {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
