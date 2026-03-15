use serde_json::json;

use crate::commands::Command;
use crate::errors::{StatefulError, WrappedWith};
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
        crate::commands::CommandFunc::Update,
        hierarchy,
    )
}

pub async fn run(states: &StateBox, args: Option<&[String]>) -> PostAction {
    match internal_run(states, args).await {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
async fn internal_run(
    _states: &StateBox,
    _args: Option<&[String]>,
) -> Result<PostAction, StatefulError> {
    let cause = json!({"runner": "update packages"});
    if let Some(action) = acquire_lock().await.wrap(&cause)? {
        return Ok(action);
    };
    collect_updates().await.wrap(&cause)?;
    Ok(PostAction::Return)
}
