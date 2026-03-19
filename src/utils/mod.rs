use nix::unistd;
use serde_json::json;
use snafu::ResultExt;
use sqlx::{SqlitePool, query, sqlite::SqliteConnectOptions};
use std::{io::Write, path::PathBuf, str::FromStr};
use tokio::{fs::DirBuilder, process::Command};

use crate::errors::{SQLSnafu, StatefulError, StdIOSnafu, TokioIOSnafu, Wrapped};
use crate::flags::Flag;

pub mod range;
pub mod verreq;
pub mod version;

// The action to perform once a command has run
#[derive(PartialEq)]
pub enum PostAction {
    Elevate,
    Err(i32),
    Fuck(StatefulError),
    GetHelp,
    NothingToDo(&'static str),
    PullSources,
    Return,
}

const LOC_DIR: &str = "/etc/plz";

pub async fn get_dir() -> Result<PathBuf, StatefulError> {
    let path = PathBuf::from(LOC_DIR);
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)
        .wrap(&json!({"action": "retrieving application directory"}))?;
    Ok(path)
}

pub async fn get_metadata_dir() -> Result<PathBuf, StatefulError> {
    let cause = json!({"action": "locating metadata directory"});
    let mut path = get_dir().await.wrap(&cause)?;
    path.push("installed");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)
        .wrap(&cause)?;
    Ok(path)
}

pub async fn get_update_dir() -> Result<PathBuf, StatefulError> {
    let cause = json!({"action": "locating updates directory"});
    let mut path = get_dir().await.wrap(&cause)?;
    path.push("updates");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)
        .wrap(&cause)?;
    Ok(path)
}

pub fn is_root() -> bool {
    unistd::geteuid().as_raw() == 0
}

pub async fn tmpfile() -> Result<(PathBuf, String), StatefulError> {
    let path = String::from_utf8_lossy(
        &Command::new("mktemp")
            .output()
            .await
            .context(TokioIOSnafu)
            .wrap(&json!({"action": "allocating temporary file"}))?
            .stdout,
    )
    .trim()
    .to_string();
    Ok((PathBuf::from(&path), path))
}

pub async fn tmpdir() -> Result<(PathBuf, String), StatefulError> {
    let mut command = Command::new("mktemp");
    let path = String::from_utf8_lossy(
        &command
            .arg("-d")
            .output()
            .await
            .context(TokioIOSnafu)
            .wrap(&json!({"action": "allocating temporary directory"}))?
            .stdout,
    )
    .trim()
    .to_string();
    Ok((PathBuf::from(&path), path))
}

pub fn yes_flag() -> Flag {
    Flag::new(
        Some('y'),
        "yes",
        "Bypasses applicable confirmation dialogs.",
        false,
        false,
        crate::flags::FlagFunc::ShoveYes,
    )
}

pub fn specific_flag() -> Flag {
    Flag::new(
        Some('s'),
        "specific",
        "Makes every second argument the target version for the argument prior.",
        false,
        false,
        crate::flags::FlagFunc::ShoveSpecific,
    )
}

pub fn choice(message: &str, default_yes: bool) -> Result<bool, StatefulError> {
    print!(
        "{} [{}]: ",
        message,
        if default_yes { "Y/n" } else { "y/N" }
    );
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context(StdIOSnafu)
        .wrap(&json!({"action": "interactive choice modal"}))?;
    if default_yes {
        if ["no", "n", "false", "f"].contains(&input.to_lowercase().trim()) {
            Ok(false)
        } else {
            Ok(true)
        }
    } else if ["yes", "y", "true", "t"].contains(&input.to_lowercase().trim()) {
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn command(name: &str, args: &[&str], pwd: Option<&str>) -> Option<i32> {
    let mut command = Command::new(name);
    command.args(args);
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    if let Some(pwd) = pwd {
        command.current_dir(pwd);
    }
    command.status().await.map(|x| x.code()).ok().flatten()
}

pub async fn get_pool() -> Result<SqlitePool, StatefulError> {
    let cause = json!({"action": "creating database pool"});
    let path = PathBuf::from(format!("{LOC_DIR}/data.db"));
    let options = SqliteConnectOptions::from_str(&path.to_string_lossy())
        .context(SQLSnafu)
        .wrap(&cause)?
        .create_if_missing(true);
    let db = SqlitePool::connect_with(options)
        .await
        .context(SQLSnafu)
        .wrap(&cause)?;
    // if path.exists() {
    //     Ok(db)
    // } else {
    //     File::create(&path).context(IOSnafu {
    //         action: IOAction::CreateFile,
    //         loc: path.display().to_string(),
    //     })?;
    query(
        r"CREATE TABLE IF NOT EXISTS installed (name TEXT, kind TEXT,
        version TEXT, origin BLOB, dependent INTEGER, dependencies BLOB,
        dependents BLOB, install_kind BLOB, hash TEXT)",
    )
    .execute(&db)
    .await
    .context(SQLSnafu)
    .wrap(&cause)?;
    query(
        r"CREATE TABLE IF NOT EXISTS updates (name TEXT, kind TEXT,
        description TEXT, version TEXT, origin BLOB, dependent INTEGER,
        built_dependencies BLOB, runtime_dependents BLOB, install_kind BLOB, hash TEXT)",
    )
    .execute(&db)
    .await
    .context(SQLSnafu)
    .wrap(&cause)?;
    Ok(db)
    // }
}

pub async fn get_apt_pool(source: &str, kind: &str) -> Result<SqlitePool, StatefulError> {
    let cause = json!({"action": "creating APT database pool"});
    let path = PathBuf::from(format!("{LOC_DIR}/apt.db"));
    let options = SqliteConnectOptions::from_str(&path.to_string_lossy())
        .context(SQLSnafu)
        .wrap(&cause)?
        .create_if_missing(true);
    let db = SqlitePool::connect_with(options)
        .await
        .context(SQLSnafu)
        .wrap(&cause)?;
    query(r"CREATE TABLE IF NOT EXISTS ? ()")
        .execute(&db)
        .await
        .context(SQLSnafu)
        .wrap(&cause)?;
    Ok(db)
}

pub fn which(name: &str) -> bool {
    for p in std::env::split_paths(&std::env::var("PATH").unwrap_or_default()) {
        if p.join(name).is_file() {
            return true;
        }
    }
    false
}

pub trait SmallEncode {
    fn encode(&self) -> String;
    fn decode(s: &str) -> Result<impl ToOwned, StatefulError>;
}

impl SmallEncode for &[u8] {
    fn encode(&self) -> String {
        self.iter().map(|b| format!("{:02x}", b)).collect()
    }
    #[allow(refining_impl_trait)]
    fn decode(s: &str) -> Result<Vec<u8>, StatefulError> {
        let cause = json!({"action": "parsing string to bytes"});
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| StatefulError::new(e, &cause))
            })
            .collect()
    }
}

// pub trait FuckWrap<T, E>: Sized {
//     fn wrap<E2: From<WrappedError>>(self) -> Result<T, E2>;
// }

// pub trait FuckNest<T, E>: Sized {
//     fn nest<E2: From<WrappedError>>(self, loc: &'static str) -> Result<T, E2>;
// }

// impl<T> FuckWrap<T, StatefulError> for Result<T, StatefulError> {
//     fn wrap<E2: From<WrappedError>>(self) -> Result<T, E2> {
//         Ok(self.context(WrappedSnafu)?)
//     }
// }

// impl<T> FuckNest<T, StatefulError> for Result<T, StatefulError> {
//     fn nest<E2: From<WrappedError>>(self, loc: &'static str) -> Result<T, E2> {
//         Ok(self.context(NestedSnafu { loc })?)
//     }
// }

// impl<T> FuckNest<T, StatefulError> for Result<T, StatefulError> {
//     fn nest<E2: From<WrappedError>>(self, loc: &'static str) -> Result<T, E2> {
//         match self {
//             Ok(t) => Ok(t),
//             Err(source) => Err(WrappedError::BoxedError {
//                 source: Box::new(source),
//                 loc: loc.into(),
//             }
//             .into()),
//         }
//     }
// }
