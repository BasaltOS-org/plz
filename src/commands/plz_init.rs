use snafu::ResultExt;
use tokio::runtime::Runtime;

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
        |_rt, states, _args| {
            states.shove("force", true);
        },
    );
    Command::new(
        "pax-init",
        Vec::new(),
        "Initializes the endpoints for pax",
        vec![force],
        None,
        get_endpoints,
        hierarchy,
    )
}

fn get_endpoints(rt: &Runtime, states: &StateBox, _args: Option<&[String]>) -> PostAction {
    match rt.block_on(async {
        if let Some(action) = acquire_lock().await.wrap()? {
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
            gen_sources().await.wrap()?;
            println!("Done!");
        }
        Ok(PostAction::Return)
    }) {
        Ok(action) => action,
        Err(error) => PostAction::Fuck(error),
    }
}

async fn gen_sources() -> Result<(), WrappedError> {
    let url = "about:blank#blocked";
    let sources = reqwest::get(url).await.context(NetSnafu)?;
    let sources = sources.text().await.context(NetSnafu)?;
    let mut settings = SettingsJson::get_settings().await.wrap()?;
    for source in sources.trim().split('\n') {
        // thingy; make this actually detect the source type
        let source = OriginKind::Plz(source.to_string());
        settings.sources.push(source);
    }
    settings.set_settings().await.wrap()
}
