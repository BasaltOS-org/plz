use tokio::runtime::Runtime;

use crate::commands::Command;
use crate::errors::Wrapped;
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
        unbind(&data).await.wrap()?;
        Ok(PostAction::Return)
    }) {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
