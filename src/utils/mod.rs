use nix::unistd;
use snafu::ResultExt;
use sqlx::{SqlitePool, query, sqlite::SqliteConnectOptions};
use std::{io::Write, path::PathBuf, str::FromStr};
use tokio::{fs::DirBuilder, process::Command};

use crate::errors::{SQLSnafu, StdIOSnafu, TokioIOSnafu, Wrapped, WrappedError};
use crate::flags::Flag;

pub mod range;
pub mod verreq;
pub mod version;

// The action to perform once a command has run
pub enum PostAction {
    Elevate,
    Err(i32),
    Fuck(WrappedError),
    GetHelp,
    NothingToDo,
    PullSources,
    Return,
}

const LOC_DIR: &str = "/etc/plz";

pub async fn get_dir() -> Result<PathBuf, WrappedError> {
    let path = PathBuf::from(LOC_DIR);
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)?;
    Ok(path)
}

pub async fn get_metadata_dir() -> Result<PathBuf, WrappedError> {
    let mut path = get_dir().await.wrap()?;
    path.push("installed");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)?;
    Ok(path)
}

pub async fn get_update_dir() -> Result<PathBuf, WrappedError> {
    let mut path = get_dir().await.wrap()?;
    path.push("updates");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .await
        .context(TokioIOSnafu)?;
    Ok(path)
}

pub fn is_root() -> bool {
    unistd::geteuid().as_raw() == 0
}

pub async fn tmpfile() -> Result<(PathBuf, String), WrappedError> {
    let path = String::from_utf8_lossy(
        &Command::new("mktemp")
            .output()
            .await
            .context(TokioIOSnafu)?
            .stdout,
    )
    .trim()
    .to_string();
    Ok((PathBuf::from(&path), path))
}

pub async fn tmpdir() -> Result<(PathBuf, String), WrappedError> {
    let mut command = Command::new("mktemp");
    let path = String::from_utf8_lossy(
        &command
            .arg("-d")
            .output()
            .await
            .context(TokioIOSnafu)?
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
        |_, states, _| {
            states.shove("yes", true);
        },
    )
}

pub fn specific_flag() -> Flag {
    Flag::new(
        Some('s'),
        "specific",
        "Makes every second argument the target version for the argument prior.",
        false,
        false,
        |_, states, _| {
            states.shove("specific", true);
        },
    )
}

pub fn choice(message: &str, default_yes: bool) -> Result<bool, WrappedError> {
    print!(
        "{} [{}]: ",
        message,
        if default_yes { "Y/n" } else { "y/N" }
    );
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).context(StdIOSnafu)?;
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

pub async fn get_pool() -> Result<SqlitePool, WrappedError> {
    let path = PathBuf::from(format!("{LOC_DIR}/data.db"));
    let options = SqliteConnectOptions::from_str(&path.to_string_lossy())
        .context(SQLSnafu)?
        .create_if_missing(true);
    let db = SqlitePool::connect_with(options).await.context(SQLSnafu)?;
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
    .context(SQLSnafu)?;
    query(
        r"CREATE TABLE IF NOT EXISTS updates (name TEXT, kind TEXT,
        description TEXT, version TEXT, origin BLOB, dependent INTEGER,
        built_dependencies BLOB, runtime_dependents BLOB, install_kind BLOB, hash TEXT)",
    )
    .execute(&db)
    .await
    .context(SQLSnafu)?;
    Ok(db)
    // }
}

pub async fn get_apt_pool(
    source: &str,
    code: &str,
    kind: &str,
) -> Result<SqlitePool, WrappedError> {
    let path = PathBuf::from(format!("{LOC_DIR}/apt.db"));
    let options = SqliteConnectOptions::from_str(&path.to_string_lossy())
        .context(SQLSnafu)?
        .create_if_missing(true);
    let db = SqlitePool::connect_with(options).await.context(SQLSnafu)?;
    query(r"CREATE TABLE IF NOT EXISTS ? ()")
        .execute(&db)
        .await
        .context(SQLSnafu)?;
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

// pub trait FuckWrap<T, E>: Sized {
//     fn wrap<E2: From<WrappedError>>(self) -> Result<T, E2>;
// }

// pub trait FuckNest<T, E>: Sized {
//     fn nest<E2: From<WrappedError>>(self, loc: &'static str) -> Result<T, E2>;
// }

// impl<T> FuckWrap<T, WrappedError> for Result<T, WrappedError> {
//     fn wrap<E2: From<WrappedError>>(self) -> Result<T, E2> {
//         Ok(self.context(WrappedSnafu)?)
//     }
// }

// impl<T> FuckNest<T, WrappedError> for Result<T, WrappedError> {
//     fn nest<E2: From<WrappedError>>(self, loc: &'static str) -> Result<T, E2> {
//         Ok(self.context(NestedSnafu { loc })?)
//     }
// }

// impl<T> FuckNest<T, WrappedError> for Result<T, WrappedError> {
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
