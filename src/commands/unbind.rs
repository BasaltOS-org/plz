use snafu::location;

use crate::commands::Command;
use crate::errors::{Wrapped, WrappedError};
use crate::metadata::unbind;
use crate::settings::acquire_lock;
use crate::statebox::StateBox;
use crate::utils::{PostAction, specific_flag};

pub fn build(hierarchy: &[String]) -> Command {
    Command::new(
        "unbind",
        vec![String::from("e")],
        "Marks a dependent package as independent.",
        vec![specific_flag()],
        None,
        crate::commands::CommandFunc::Unbind,
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
    states: &StateBox,
    args: Option<&[String]>,
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
    unbind(&data).await.wrap(location!())?;
    Ok(PostAction::Return)
}
