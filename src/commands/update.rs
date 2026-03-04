use tokio::runtime::Runtime;

use crate::commands::Command;
use crate::errors::Wrapped;
use crate::metadata::collect_updates;
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::PostAction;

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

fn run(rt: &Runtime, _states: &StateBox, _args: Option<&[String]>) -> PostAction {
    match rt.block_on(async {
        if let Some(action) = acquire_lock().await.wrap()? {
            return Ok(action);
        };
        collect_updates().await.wrap()?;
        Ok(PostAction::Return)
    }) {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
