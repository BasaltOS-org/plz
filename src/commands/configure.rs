use snafu::{OptionExt, location};

use crate::commands::Command;
use crate::errors::{OtherSnafu, Wrapped, WrappedError};
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
) -> Result<(), WrappedError> {
    if acquire_lock().await.wrap(location!())?.is_some() {
        return Err(WrappedError::Other {
            error: "Did not expect a `PostAction` at this time.".into(),
            loc: location!(),
        });
    };
    let settings = SettingsJson::get_settings().await.wrap(location!())?;
    set_func(states, arg, settings).await.wrap(location!())?;
    remove_lock().await.wrap(location!())
}

async fn set_func(
    states: &mut StateBox,
    arg: Option<String>,
    mut settings: SettingsJson,
) -> Result<(), WrappedError> {
    // let arg = arg.WrappedEver_context("Missing an argument!")?;
    let arg = arg.context(OtherSnafu {
        error: "Missing an argument!",
    })?;
    let (key, value) = arg.split_once('=').context(OtherSnafu {
        error: "Invalid syntax. please use `--set \"key=value\"`.",
    })?;
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
                && !choice("Proceed?", true).wrap(location!())?
            {
                return Err(WrappedError::Other {
                    error: "Operation aborted by user.".into(),
                    loc: location!(),
                });
            }
            settings.exec = val;
        }
        _ => {
            return Err(WrappedError::Other {
                error: "Unrecognized key {key}!".into(),
                loc: location!(),
            });
        }
    }
    settings.set_settings().await.wrap(location!())?;
    Ok(())
}
