use snafu::location;
use tokio::runtime::Runtime;

use crate::commands::Command;
use crate::errors::{Wrapped, WrappedError};
use crate::metadata::{upgrade_all, upgrade_only, upgrade_packages};
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{PostAction, choice, yes_flag};

pub fn build(hierarchy: &[String]) -> Command {
    Command::new(
        "upgrade",
        vec![String::from("g")],
        "Upgrades a non-phased package from its upgrade metadata.",
        vec![yes_flag()],
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
        let args = if let Some(args) = args {
            let mut args = args.iter();
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
            data
        } else {
            Vec::new()
        };
        let data = if args.is_empty() {
            upgrade_all()
        } else {
            upgrade_only(&args)
        }
        .wrap()?;
        if data.is_empty() {
            return Ok(PostAction::NothingToDo);
        }
        println!(
            "The following package(s) will be UPGRADED: \x1B[94m{}\x1B[0m",
            data.iter()
                .fold(String::new(), |acc, x| format!("{acc} {}", x.name))
                .trim()
        );
        if states.get("yes").is_none_or(|x: &bool| !*x) && !choice("Continue?", true).wrap()? {
            return Err(WrappedError::Other {
                error: "Operation aborted by user.".into(),
                loc: location!(),
            });
        };
        upgrade_packages(&data).await.wrap()?;
        Ok(PostAction::Return)
    }) {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
