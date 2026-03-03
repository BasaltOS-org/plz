use crate::commands::Command;
use crate::errors::{RuntimeSnafu, WhatError, WhereError};
use crate::metadata::{upgrade_all, upgrade_only, upgrade_packages};
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{FuckWrap, PostAction, choice, yes_flag};

use snafu::ResultExt;
use tokio::runtime::Runtime;

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

fn run(states: &StateBox, args: Option<&[String]>) -> PostAction {
    match acquire_lock() {
        Ok(Some(action)) => return action,
        Err(source) => {
            return PostAction::Fuck(WhatError::Upgrade {
                source: WhereError::WrappedError { source },
            });
        }

        _ => (),
    }
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
    let data = match if args.is_empty() {
        upgrade_all()
    } else {
        upgrade_only(&args)
    } {
        Ok(data) => data,
        Err(source) => return PostAction::Fuck(WhatError::Upgrade { source }),
    };
    if data.is_empty() {
        return PostAction::NothingToDo;
    }
    println!(
        "The following package(s) will be UPGRADED: \x1B[94m{}\x1B[0m",
        data.iter()
            .fold(String::new(), |acc, x| format!("{acc} {}", x.name))
            .trim()
    );
    if states.get("yes").is_none_or(|x: &bool| !*x) {
        match choice("Continue?", true) {
            Err(source) => {
                return PostAction::Fuck(WhatError::Upgrade {
                    source: WhereError::WrappedError { source },
                });
            }
            Ok(false) => {
                return PostAction::Fuck(WhatError::Upgrade {
                    source: WhereError::other("Aborted."),
                });
            }
            Ok(true) => (),
        };
    }
    let runtime = match Runtime::new().context(RuntimeSnafu).wrap() {
        Ok(runtime) => runtime,
        Err(source) => return PostAction::Fuck(WhatError::Install { source }),
    };
    if let Err(fault) = runtime.block_on(upgrade_packages(&data)) {
        return PostAction::Fuck(WhatError::Upgrade { source: fault });
    }
    PostAction::Return
}
