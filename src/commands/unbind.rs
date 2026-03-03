use crate::commands::Command;
use crate::errors::{RuntimeSnafu, WhatError, WhereError};
use crate::metadata::unbind;
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{FuckWrap, PostAction, specific_flag};

use snafu::ResultExt;
use tokio::runtime::Runtime;

pub fn build(hierarchy: &[String]) -> Command {
    Command::new(
        "unbind",
        vec![String::from("e")],
        "Marks a dependent package as independent.",
        vec![specific_flag()],
        None,
        run,
        hierarchy,
    )
}

fn run(states: &StateBox, args: Option<&[String]>) -> PostAction {
    match acquire_lock() {
        Ok(Some(action)) => return action,
        Err(source) => {
            return PostAction::Fuck(WhatError::Emancipate {
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
        Err(source) => return PostAction::Fuck(WhatError::Install { source }),
    };
    if let Err(source) = runtime.block_on(unbind(&data)) {
        PostAction::Fuck(WhatError::Emancipate { source })
    } else {
        PostAction::Return
    }
}
