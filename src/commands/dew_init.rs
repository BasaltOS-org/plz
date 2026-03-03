use flags::Flag;
use settings::OriginKind;
use settings::SettingsJson;
use settings::acquire_lock;
use snafu::ResultExt;
use statebox::StateBox;
use tokio::runtime::Runtime;
use utils::FuckWrap;
use utils::PostAction;
use utils::errors::HowError;
use utils::errors::NetSnafu;
use utils::errors::RuntimeSnafu;
use utils::errors::WhatError;
use utils::errors::WhereError;

use crate::commands::Command;

static LONG_NAME: &str = "force";

pub fn build(hierarchy: &[String]) -> Command {
    let force = Flag::new(
        None,
        LONG_NAME,
        "bypasses the warning before running the command",
        false,
        false,
        |states, _args| {
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

fn get_endpoints(states: &StateBox, _args: Option<&[String]>) -> PostAction {
    match acquire_lock() {
        Ok(Some(PostAction::PullSources)) => (),
        Ok(Some(action)) => return action,
        Err(source) => {
            return PostAction::Fuck(WhatError::Init {
                source: WhereError::WrappedError { source },
            });
        }
        _ => (),
    }
    if states.get::<bool>("force").is_none_or(|x| !*x) {
        println!(
            "\x1B[33m===== WARNING! WARNING! WARNING! =====\x1B[0m
This command should \x1B[31mNOT\x1B[0m be run as part of a standard update procedure.
To continue anyway, run with flag `\x1B[35m--{LONG_NAME}\x1B[0m`."
        );
    } else {
        println!("Pulling sources...");
        let runtime = match Runtime::new().context(RuntimeSnafu).wrap() {
            Ok(runtime) => runtime,
            Err(source) => return PostAction::Fuck(WhatError::Init { source }),
        };
        if let Err(source) = runtime.block_on(gen_sources()) {
            return PostAction::Fuck(WhatError::Init {
                source: WhereError::WrappedError { source },
            });
        } else {
            println!("Done!");
        }
    }
    PostAction::Return
}

async fn gen_sources() -> Result<(), HowError> {
    let url = "about:blank#blocked";
    let sources = reqwest::get(url).await.context(NetSnafu { loc: url })?;
    let sources = sources.text().await.context(NetSnafu { loc: url })?;
    let mut settings = SettingsJson::get_settings()?;
    for source in sources.trim().split('\n') {
        // thingy; make this actually detect the source type
        let source = OriginKind::Dew(source.to_string());
        settings.sources.push(source);
    }
    settings.set_settings()
}
