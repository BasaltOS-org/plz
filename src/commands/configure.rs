use crate::commands::Command;
use crate::errors::WhereError;
use crate::flags::Flag;
use crate::settings::{SettingsJson, acquire_lock, remove_lock};
use crate::statebox::StateBox;
use crate::utils::{FuckWrap, PostAction, choice, yes_flag};

pub fn build(hierarchy: &[String]) -> Command {
    let setting = Flag::new(
        Some('s'),
        "set",
        "Command to set options in the SettingsJSON file.",
        true,
        true,
        set_handle,
    );
    Command::new(
        "configure",
        vec![String::from("c")],
        "Configures internal dew settings.",
        vec![setting, yes_flag()],
        None,
        |_, _| PostAction::GetHelp,
        hierarchy,
    )
}

fn set_handle(states: &mut StateBox, arg: Option<String>) {
    match acquire_lock() {
        Ok(Some(_)) => {
            println!("Did not expect a PostAction at this time.");
            return;
        }
        Err(fault) => {
            print!("{fault}");
            return;
        }
        _ => (),
    };
    let settings = match SettingsJson::get_settings() {
        Ok(settings) => settings,
        Err(fault) => {
            println!("{fault}");
            return;
        }
    };
    if let Err(fault) = set_func(states, arg, settings) {
        println!("{fault}");
    };
    if let Err(fault) = remove_lock() {
        println!("{fault}");
    }
}

fn set_func(
    states: &mut StateBox,
    arg: Option<String>,
    mut settings: SettingsJson,
) -> Result<(), WhereError> {
    // let arg = arg.whatever_context("Missing an argument!")?;
    let Some(arg) = arg else {
        return Err(WhereError::other("Missing an argument!"));
    };
    let Some((key, value)) = arg.split_once('=') else {
        return Err(WhereError::other(
            "Invalid syntax. please use `--set \"key=value\"`.",
        ));
    };
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
            if states.get("yes").is_none_or(|x: &bool| !*x) {
                match choice("Proceed?", true) {
                    Err(message) => return Err(WhereError::other(message.to_string())),
                    Ok(false) => return Err(WhereError::other("Abort.")),
                    Ok(true) => (),
                }
            }
            settings.exec = val;
        }
        _ => return Err(WhereError::other("Unrecognized key {key}!")),
    }
    settings.set_settings().wrap()?;
    Ok(())
}
