use snafu::{ResultExt, location};

use crate::commands::Command;
use crate::errors::{NetSnafu, Wrapped, WrappedError};
use crate::flags::Flag;
use crate::settings::{OriginKind, SettingsJson, acquire_lock};
use crate::statebox::StateBox;
use crate::utils::PostAction;

static LONG_NAME: &str = "force";

pub fn build(hierarchy: &[String]) -> Command {
    let force = Flag::new(
        None,
        LONG_NAME,
        "bypasses the warning before running the command",
        false,
        false,
        crate::flags::FlagFunc::ShoveForce,
    );
    Command::new(
        "plz-init",
        Vec::new(),
        "Initializes the endpoints for plz",
        vec![force],
        None,
        crate::commands::CommandFunc::Init,
        // get_endpoints,
        hierarchy,
    )
}

pub async fn get_endpoints(states: &StateBox, args: Option<&[String]>) -> PostAction {
    match internal_get_endpoints(states, args).await {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}
async fn internal_get_endpoints(
    states: &StateBox,
    _args: Option<&[String]>,
) -> Result<PostAction, WrappedError> {
    if let Some(action) = acquire_lock().await.wrap(location!())? {
        return Ok(action);
    };
    if states.get::<bool>("force").is_none_or(|x| !*x) {
        println!(
            "\x1B[33m===== WARNING! WARNING! WARNING! =====\x1B[0m
This command should \x1B[31mNOT\x1B[0m be run as part of a standard update procedure.
To continue anyway, run with flag `\x1B[35m--{LONG_NAME}\x1B[0m`."
        );
    } else {
        println!("Pulling sources...");
        gen_sources().await.wrap(location!())?;
        println!("Done!");
    }
    Ok(PostAction::Return)
}

async fn gen_sources() -> Result<(), WrappedError> {
    let url = "about:blank#blocked";
    let sources = reqwest::get(url).await.context(NetSnafu)?;
    let sources = sources.text().await.context(NetSnafu)?;
    let mut settings = SettingsJson::get_settings().await.wrap(location!())?;
    for source in sources.trim().split('\n') {
        // thingy; make this actually detect the source type
        let source = OriginKind::Plz(source.to_string());
        settings.sources.push(source);
    }
    settings.set_settings().await.wrap(location!())
}
