use crate::errors::{
    HowError, IOAction, IOSnafu, NestedSnafu, Parsers, SQLSnafu, WhatError, WhereError,
    WrappedSnafu,
};
use crate::flags::Flag;

use nix::unistd;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use sqlx::{SqlitePool, query, sqlite::SqliteConnectOptions};
use std::{
    cmp::Ordering, fmt::Display, fs::DirBuilder, io::Write, path::PathBuf, process::Command,
    str::FromStr,
};

// The action to perform once a command has run
pub enum PostAction {
    Elevate,
    Err(i32),
    Fuck(WhatError),
    GetHelp,
    NothingToDo,
    PullSources,
    Return,
}

pub fn get_dir() -> Result<PathBuf, HowError> {
    let loc = "/etc/dew";
    let path = PathBuf::from(loc);
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .context(IOSnafu {
            action: IOAction::CreateDir,
            loc,
        })?;
    Ok(path)
}

pub fn get_metadata_dir() -> Result<PathBuf, HowError> {
    let mut path = get_dir()?;
    path.push("installed");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .context(IOSnafu {
            action: IOAction::CreateDir,
            loc: path.display().to_string(),
        })?;
    Ok(path)
}

pub fn get_update_dir() -> Result<PathBuf, HowError> {
    let mut path = get_dir()?;
    path.push("updates");
    DirBuilder::new()
        .recursive(true)
        .create(&path)
        .context(IOSnafu {
            action: IOAction::CreateDir,
            loc: path.display().to_string(),
        })?;
    Ok(path)
}

pub fn is_root() -> bool {
    unistd::geteuid().as_raw() == 0
}

pub fn tmpfile() -> Option<(PathBuf, String)> {
    let path = String::from_utf8_lossy(&Command::new("mktemp").output().ok()?.stdout)
        .trim()
        .to_string();
    Some((PathBuf::from(&path), path))
}

pub fn tmpdir() -> Option<(PathBuf, String)> {
    let mut command = Command::new("mktemp");
    let path = String::from_utf8_lossy(&command.arg("-d").output().ok()?.stdout)
        .trim()
        .to_string();
    Some((PathBuf::from(&path), path))
}

pub fn yes_flag() -> Flag {
    Flag::new(
        Some('y'),
        "yes",
        "Bypasses applicable confirmation dialogs.",
        false,
        false,
        |states, _| {
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
        |states, _| {
            states.shove("specific", true);
        },
    )
}

pub fn choice(message: &str, default_yes: bool) -> Result<bool, HowError> {
    print!(
        "{} [{}]: ",
        message,
        if default_yes { "Y/n" } else { "y/N" }
    );
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).context(IOSnafu {
        action: IOAction::TermRead,
        loc: "CONSOLE",
    })?;
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

pub fn command(name: &str, args: &[&str], pwd: Option<&str>) -> Option<i32> {
    let mut command = Command::new(name);
    command.args(args);
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    if let Some(pwd) = pwd {
        command.current_dir(pwd);
    }
    command.status().map(|x| x.code()).ok().flatten()
}

pub async fn get_pool() -> Result<SqlitePool, HowError> {
    let path = PathBuf::from("/etc/dew/data.db");
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

pub async fn get_apt_pool(source: &str, code: &str, kind: &str) -> Result<SqlitePool, HowError> {
    let path = PathBuf::from("/etc/dew/apt.db");
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

pub trait FuckWrap<T, E>: Sized {
    fn wrap<E2: From<WhereError>>(self) -> Result<T, E2>;
}

pub trait FuckNest<T, E>: Sized {
    fn nest<E2: From<WhereError>>(self, loc: &'static str) -> Result<T, E2>;
}

impl<T> FuckWrap<T, HowError> for Result<T, HowError> {
    fn wrap<E2: From<WhereError>>(self) -> Result<T, E2> {
        Ok(self.context(WrappedSnafu)?)
    }
}

impl<T> FuckNest<T, HowError> for Result<T, HowError> {
    fn nest<E2: From<WhereError>>(self, loc: &'static str) -> Result<T, E2> {
        Ok(self.context(NestedSnafu { loc })?)
    }
}

impl<T> FuckNest<T, WhereError> for Result<T, WhereError> {
    fn nest<E2: From<WhereError>>(self, loc: &'static str) -> Result<T, E2> {
        match self {
            Ok(t) => Ok(t),
            Err(source) => Err(WhereError::BoxedError {
                source: Box::new(source),
                loc: loc.into(),
            }
            .into()),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    pub pre: String,
    pub build: Option<String>,
}

impl Version {
    pub fn parse(src: &str) -> Result<Self, HowError> {
        let (src, build) = src
            .split_once('+')
            .map(|x| (x.0, Some(x.1.to_string())))
            .unwrap_or((src, None));
        let (src, pre) = src
            .split_once('-')
            .map(|x| (x.0, x.1.to_string()))
            .unwrap_or_else(|| (src, String::new()));
        let split = src.split('.').collect::<Vec<&str>>();
        if !split.is_empty() {
            if let Ok(major) = split[0].parse::<usize>() {
                if split.len() >= 2 {
                    if let Ok(minor) = split[1].parse::<usize>() {
                        if split.len() >= 3 {
                            if let Ok(patch) = split[2].parse::<usize>() {
                                if split.len() > 3 {
                                    Err(HowError::ParseError {
                                        message: "Two many segments in version!".into(),
                                        util: Parsers::Version,
                                    })
                                } else {
                                    Ok(Self {
                                        major,
                                        minor,
                                        patch,
                                        pre,
                                        build,
                                    })
                                }
                            } else {
                                Err(HowError::ParseError {
                                    message: format!(
                                        "Expected patch to be a number, got `{}`!",
                                        split[1]
                                    )
                                    .into(),
                                    util: Parsers::Version,
                                })
                            }
                        } else {
                            Ok(Self {
                                major,
                                minor,
                                patch: 0,
                                pre,
                                build,
                            })
                        }
                    } else {
                        Err(HowError::ParseError {
                            message: format!("Expected minor to be a number, got `{}`!", split[1])
                                .into(),
                            util: Parsers::Version,
                        })
                    }
                } else {
                    Ok(Self {
                        major,
                        minor: 0,
                        patch: 0,
                        pre,
                        build,
                    })
                }
            } else {
                Err(HowError::ParseError {
                    message: format!("Expected major to be a number, got `{}`!", split[0]).into(),
                    util: Parsers::Version,
                })
            }
        } else {
            Err(HowError::ParseError {
                message: "A version must be specified!".into(),
                util: Parsers::Version,
            })
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut tail = if self.pre.is_empty() {
            String::new()
        } else {
            format!("-{}", self.pre)
        };
        if let Some(build) = &self.build {
            tail.push_str(&format!("+{}", build));
        }
        f.write_str(&format!(
            "{}.{}.{}{}",
            self.major, self.minor, self.patch, tail
        ))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => match self.minor.cmp(&other.minor) {
                Ordering::Equal => match self.patch.cmp(&other.patch) {
                    Ordering::Equal => match self.pre.cmp(&other.pre) {
                        Ordering::Equal => self.build.cmp(&other.build),
                        order => order,
                    },
                    order => order,
                },
                order => order,
            },
            order => order,
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum VerReq {
    Gt(Version),
    Ge(Version),
    Eq(Version),
    Le(Version),
    Lt(Version),
    NoBound,
}

impl VerReq {
    pub fn negotiate(&self, prior: Option<Range>) -> Option<Range> {
        let prior = if let Some(mut prior) = prior {
            match self {
                Self::Gt(gt) => match &prior.lower {
                    Self::Gt(p_gt) => {
                        if gt > p_gt {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Ge(p_ge) => {
                        if gt >= p_ge {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::NoBound => {
                        prior.lower = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Ge(ge) => match &prior.lower {
                    Self::Gt(p_gt) => {
                        if ge > p_gt {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Ge(p_ge) => {
                        if ge > p_ge {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Eq(p_eq) => {
                        if ge == p_eq {
                            Some(prior)
                        } else {
                            None
                        }
                    }
                    Self::NoBound => {
                        prior.lower = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Eq(eq) => {
                    match &prior.lower {
                        Self::Gt(p_gt) => {
                            if eq > p_gt {
                                prior.lower = self.clone();
                            } else {
                                return None;
                            }
                        }
                        Self::Ge(p_ge) => {
                            if eq >= p_ge {
                                prior.lower = self.clone();
                            } else {
                                return None;
                            }
                        }
                        Self::Eq(p_eq) => {
                            if eq != p_eq {
                                return None;
                            }
                        }
                        Self::NoBound => {
                            prior.lower = self.clone();
                        }
                        _ => return None,
                    }
                    match &prior.upper {
                        Self::Eq(p_eq) => {
                            if eq != p_eq {
                                return None;
                            }
                        }
                        Self::Le(p_le) => {
                            if eq <= p_le {
                                prior.upper = self.clone();
                            } else {
                                return None;
                            }
                        }
                        Self::Lt(p_lt) => {
                            if eq < p_lt {
                                prior.upper = self.clone();
                            } else {
                                return None;
                            }
                        }
                        Self::NoBound => {
                            prior.upper = self.clone();
                        }
                        _ => return None,
                    }
                    Some(prior)
                }
                Self::Le(le) => match &prior.upper {
                    Self::Lt(p_lt) => {
                        if le < p_lt {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Le(p_le) => {
                        if le < p_le {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Eq(p_eq) => {
                        if le == p_eq {
                            Some(prior)
                        } else {
                            None
                        }
                    }
                    Self::NoBound => {
                        prior.upper = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Lt(lt) => match &prior.upper {
                    Self::Lt(p_lt) => {
                        if lt < p_lt {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Le(p_le) => {
                        if lt <= p_le {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::NoBound => {
                        prior.upper = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::NoBound => Some(prior),
            }
        } else {
            None
        };
        if prior.as_ref().is_some_and(|x| x.is_sane()) {
            prior
        } else {
            None
        }
    }
    fn parse(input: &str) -> Result<Self, HowError> {
        let mut chars = input.chars();
        let kind = chars.next().ok_or(HowError::ParseError {
            message: "Missing type identifier!".into(),
            util: Parsers::VerReq,
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => Ok(Self::NoBound),
            kind => {
                let version = Version::parse(&data)?;
                match kind {
                    1 => Ok(Self::Gt(version)),
                    2 => Ok(Self::Ge(version)),
                    3 => Ok(Self::Eq(version)),
                    4 => Ok(Self::Le(version)),
                    5 => Ok(Self::Lt(version)),
                    kind => Err(HowError::ParseError {
                        message: format!("Invalid kind identifier `{kind}`!").into(),
                        util: Parsers::VerReq,
                    }),
                }
            }
        }
    }
}

impl Display for VerReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::Gt(gt) => format!("\x01{gt}"),
            Self::Ge(ge) => format!("\x02{ge}"),
            Self::Eq(eq) => format!("\x03{eq}"),
            Self::Le(le) => format!("\x04{le}"),
            Self::Lt(lt) => format!("\x05{lt}"),
            Self::NoBound => String::from("\x00"),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Range {
    pub lower: VerReq,
    pub upper: VerReq,
}

impl Range {
    pub fn is_sane(&self) -> bool {
        match &self.lower {
            VerReq::Gt(gt) => match &self.upper {
                VerReq::Eq(o_eq) => gt == o_eq,
                VerReq::Le(o) | VerReq::Lt(o) => gt < o,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::Ge(ge) => match &self.upper {
                VerReq::Eq(o_eq) => ge == o_eq,
                VerReq::Le(o_le) => ge <= o_le,
                VerReq::Lt(o_lt) => ge < o_lt,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::Eq(eq) => match &self.upper {
                VerReq::Eq(o_eq) => eq == o_eq,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::NoBound => true,
            _ => false,
        }
    }
    pub fn negotiate(&self, prior: Option<Self>) -> Option<Self> {
        self.upper.negotiate(self.lower.negotiate(prior))
    }
    pub fn parse(input: &str) -> Result<Self, HowError> {
        let (lower, upper) = input.split_once(' ').ok_or(HowError::ParseError {
            message: "Missing Range field `upper`!".into(),
            util: Parsers::Range,
        })?;
        let lower = VerReq::parse(lower)?;
        let upper = VerReq::parse(upper)?;
        Ok(Self { lower, upper })
    }
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} {}", self.lower, self.upper))
    }
}
