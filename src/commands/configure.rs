use serde_json::json;
use snafu::OptionExt;

use crate::commands::Command;
use crate::errors::{OtherSnafu, StatefulError, Wrapped, WrappedWith};
use crate::flags::Flag;
use crate::settings::{SettingsJson, acquire_lock, remove_lock};
use crate::statebox::StateBox;
use crate::utils::{choice, yes_flag};

pub fn build(hierarchy: &[String]) -> Command {
    let setting = Flag::new(
        Some('s'),
        "set",
        "Command to set options in the SettingsJSON file.",
        true,
        true,
        crate::flags::FlagFunc::SetHandle,
    );
    Command::new(
        "configure",
        vec![String::from("c")],
        "Configures internal PLZ settings.",
        vec![setting, yes_flag()],
        None,
        crate::commands::CommandFunc::GetHelp,
        hierarchy,
    )
}

pub async fn set_handle(states: &mut StateBox, arg: Option<String>) {
    if let Err(error) = internal_set_handle(states, arg).await {
        println!("{error}")
    }
}
async fn internal_set_handle(
    states: &mut StateBox,
    arg: Option<String>,
) -> Result<(), StatefulError> {
    let cause = json!({"action": "setting handle"});
    if acquire_lock().await.wrap(&cause)?.is_some() {
        return Err(StatefulError::new(
            "Did not expect a `PostAction` at this time.",
        ));
    };
    let settings = SettingsJson::get_settings().await.wrap(&cause)?;
    set_func(states, arg, settings).await.wrap(&cause)?;
    remove_lock().await.wrap(&cause)
}

async fn set_func(
    states: &mut StateBox,
    arg: Option<String>,
    mut settings: SettingsJson,
) -> Result<(), StatefulError> {
    let cause = json!({"action": "setting function from CLI arguments"});
    // let arg = arg.WrappedEver_context("Missing an argument!")?;
    let arg = arg
        .context(OtherSnafu {
            error: "Missing an argument!",
        })
        .wrap()?;
    let (key, value) = arg
        .split_once('=')
        .context(OtherSnafu {
            error: "Invalid syntax. please use `--set \"key=value\"`.",
        })
        .wrap()?;
    match key {
        "exec" => {
            let val = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
            println!(
                "Will change setting `exec` from \x1B[95m{:?}\x1B[0m to \x1B[95m{val:?}\x1B[0m.",
                settings.exec
            );
            if states.get("yes").is_none_or(|x: &bool| !*x)
                && !choice("Proceed?", true).wrap(&cause)?
            {
                return Err(StatefulError::new("Operation aborted by user."));
            }
            settings.exec = val;
        }
        _ => {
            return Err(StatefulError::new("Unrecognized key {key}!"));
        }
    }
    settings.set_settings().await.wrap(&cause)?;
    Ok(())
}
