use crate::commands::Command;
use crate::errors::{RuntimeSnafu, WhatError, WhereError};
use crate::metadata::collect_updates;
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{FuckWrap, PostAction};

use snafu::ResultExt;
use tokio::runtime::Runtime;

pub fn build(hierarchy: &[String]) -> Command {
    Command::new(
        "update",
        vec![String::from("d")],
        "Downloads the upgrade metadata for non-phased packages.",
        Vec::new(),
        None,
        run,
        hierarchy,
    )
}

fn run(_states: &StateBox, _args: Option<&[String]>) -> PostAction {
    match acquire_lock() {
        Ok(Some(action)) => return action,
        Err(source) => {
            return PostAction::Fuck(WhatError::Update {
                source: WhereError::WrappedError { source },
            });
        }
        _ => (),
    }
    let runtime = match Runtime::new().context(RuntimeSnafu).wrap() {
        Ok(runtime) => runtime,
        Err(source) => return PostAction::Fuck(WhatError::Update { source }),
    };
    if let Err(source) = runtime.block_on(collect_updates()) {
        PostAction::Fuck(WhatError::Update { source })
    } else {
        PostAction::Return
    }
}
