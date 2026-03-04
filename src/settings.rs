use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, location};
use sqlx::{Decode, Encode, Sqlite, Type};
use std::{fmt::Display, io::Write, path::PathBuf, thread::sleep};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    time::Duration,
};

use crate::errors::{JSONSnafu, OtherSnafu, TokioIOSnafu, Wrapped, WrappedError};
use crate::utils::{PostAction, get_dir, is_root};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsJson {
    pub locked: bool,
    pub version: String,
    pub arch: Arch,
    pub exec: Option<String>,
    pub sources: Vec<OriginKind>,
}

impl SettingsJson {
    pub fn new() -> Self {
        let mut command = std::process::Command::new("/usr/bin/uname");
        let arch = if let Ok(output) = command.arg("-m").output() {
            match String::from_utf8_lossy(&output.stdout)
                .to_string()
                .as_str()
                .trim()
            {
                "x86_64" => {
                    let mut command = std::process::Command::new("/usr/bin/bash");
                    command.arg("-c").arg("(lscpu|grep -q avx512f&&echo 4&&exit||lscpu|grep -q avx2&&echo 3&&exit||lscpu|grep -q sse4_2&&echo 2&&exit||echo 1)");
                    if let Ok(output) = command.output() {
                        match String::from_utf8_lossy(&output.stdout)
                            .to_string()
                            .as_str()
                            .trim()
                        {
                            "4" | "3" => Arch::X86_64v3,
                            "2" | "1" => Arch::X86_64v1,
                            _ => Arch::NoArch,
                        }
                    } else {
                        Arch::NoArch
                    }
                }
                "aarch64" => Arch::Aarch64,
                "armv7l" => Arch::Armv7l,
                "armv8l" => Arch::Armv8l,
                _ => Arch::NoArch,
            }
        } else {
            Arch::NoArch
        };
        Self {
            locked: false,
            version: env!("SETTINGS_JSON_VERSION").to_string(),
            arch,
            exec: None,
            sources: Vec::new(),
        }
    }
    pub async fn set_settings(self) -> Result<(), WrappedError> {
        let mut file = File::create(affirm_path().await.wrap()?)
            .await
            .context(TokioIOSnafu)?;
        let settings = serde_json::to_string(&self).context(JSONSnafu)?;
        file.write_all(settings.as_bytes())
            .await
            .context(TokioIOSnafu)
    }
    pub async fn get_settings() -> Result<Self, WrappedError> {
        let mut file = File::open(affirm_path().await.wrap()?)
            .await
            .context(TokioIOSnafu)?;
        let mut sources = String::new();
        file.read_to_string(&mut sources)
            .await
            .context(TokioIOSnafu)?;
        serde_json::from_str(&sources).context(JSONSnafu)
    }
}

impl Default for SettingsJson {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum OriginKind {
    Apt {
        source: String,
        code: String,
        kind: AptKind,
    },
    Dew(String),
    Github {
        user: String,
        repo: String,
    },
}

impl OriginKind {
    fn parse(input: &str) -> Result<Self, WrappedError> {
        let mut chars = input.chars();
        let kind = chars.next().ok_or(WrappedError::Other {
            error: "Missing type identifier!".into(),
            loc: location!(),
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => {
                let mut splits = data.split(' ');
                let source = splits.next().context(OtherSnafu {
                    error: "Missing APT field `source`!",
                })?;
                let code = splits.next().context(OtherSnafu {
                    error: "Missing APT field `code`!",
                })?;
                let kind = splits.next().context(OtherSnafu {
                    error: "Missing APT field `kind`!",
                })?;
                let kind = match kind {
                    "main" => AptKind::Main,
                    "multiverse" => AptKind::Multiverse,
                    "restricted" => AptKind::Restricted,
                    "universe" => AptKind::Universe,
                    other => AptKind::Custom(other.to_string()),
                };
                Ok(Self::Apt {
                    source: source.to_string(),
                    code: code.to_string(),
                    kind,
                })
            }
            1 => Ok(Self::Dew(data.to_string())),
            2 => {
                let (user, repo) = data.split_once(' ').context(OtherSnafu {
                    error: "Missing GH field `repo`!",
                })?;
                Ok(Self::Github {
                    user: user.to_string(),
                    repo: repo.to_string(),
                })
            }
            kind => Err(WrappedError::Other {
                error: format!("Invalid kind identifier `{kind}`!").into(),
                loc: location!(),
            }),
        }
    }
}

impl Display for OriginKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::Apt { source, code, kind } => {
                format!("\x00{source} {code} {kind}")
            }
            Self::Dew(dew) => format!("\x01{dew}"),
            Self::Github { user, repo } => format!("\x02{user} {repo}"),
        })
    }
}

impl Type<Sqlite> for OriginKind {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for OriginKind {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <String as Encode<'_, Sqlite>>::encode_by_ref(&self.to_string(), buf)
    }
    fn encode(
        self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError>
    where
        Self: Sized,
    {
        <String as Encode<'_, Sqlite>>::encode(self.to_string(), buf)
    }
}

impl<'a> Decode<'a, Sqlite> for OriginKind {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum AptKind {
    Custom(String),
    Main,
    Multiverse,
    Restricted,
    Universe,
}

impl Display for AptKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Custom(c) => c,
            Self::Main => "main",
            Self::Multiverse => "multiverse",
            Self::Restricted => "restricted",
            Self::Universe => "universe",
        })
    }
}

#[derive(Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Arch {
    Any,
    X86_64v1,
    X86_64v3,
    Aarch64,
    Armv7l,
    Armv8l,
    NoArch,
}

impl Arch {
    pub async fn is_compatible(&self, name: &str) -> Result<bool, WrappedError> {
        let installed = SettingsJson::get_settings().await.wrap()?.arch;
        match self {
            Self::Any => Ok(true),
            Self::X86_64v1 => Ok([Self::X86_64v1, Self::X86_64v3].contains(&installed)),
            Self::NoArch => Err(WrappedError::Other {
                error: format!("Unrecognized architecture in package {name}!").into(),
                loc: location!(),
            }),
            other => Ok(installed == *other),
        }
    }
}

async fn affirm_path() -> Result<PathBuf, WrappedError> {
    let mut path = get_dir().await.wrap()?;
    path.push("settings.json");
    if !path.exists() {
        let mut file = File::create(&path).await.context(TokioIOSnafu)?;
        let new_settings = serde_json::to_string(&SettingsJson::new()).context(JSONSnafu)?;

        file.write_all(new_settings.as_bytes())
            .await
            .context(TokioIOSnafu)?;
        Ok(path)
    } else if path.is_file() {
        Ok(path)
    } else {
        Err(WrappedError::Other {
            error: format!(
                "Path {} is not of the expected type. Is it a real file?",
                path.display()
            )
            .into(),
            loc: location!(),
        })
    }
}

pub async fn acquire_lock() -> Result<Option<PostAction>, WrappedError> {
    if !is_root() {
        return Ok(Some(PostAction::Elevate));
    }
    let mut settings = SettingsJson::get_settings().await.wrap()?;
    loop {
        if settings.locked {
            for i in 0..20 {
                print!(
                    "\x1B[2K\r\x1B[91mAwaiting program lock. Retrying in {:.2}s...\x1B[0m",
                    (100 - i) as f32 / 20f32
                );
                let _ = std::io::stdout().flush();
                sleep(Duration::from_millis(50));
            }
            for i in 0..20 {
                print!(
                    "\x1B[2K\r\x1B[93mAwaiting program lock. Retrying in {:.2}s\x1B[0m...",
                    (80 - i) as f32 / 20f32
                );
                let _ = std::io::stdout().flush();
                sleep(Duration::from_millis(50));
            }
            for i in 0..20 {
                print!(
                    "\x1B[2K\r\x1B[95mAwaiting program lock. Retrying in {:.2}s\x1B[0m...",
                    (60 - i) as f32 / 20f32
                );
                let _ = std::io::stdout().flush();
                sleep(Duration::from_millis(50));
            }
            for i in 0..20 {
                print!(
                    "\x1B[2K\r\x1B[94mAwaiting program lock. Retrying in {:.2}s\x1B[0m...",
                    (40 - i) as f32 / 20f32
                );
                let _ = std::io::stdout().flush();
                sleep(Duration::from_millis(50));
            }
            for i in 0..20 {
                print!(
                    "\x1B[2K\r\x1B[92mAwaiting program lock. Retrying in {:.2}s\x1B[0m...",
                    (20 - i) as f32 / 20f32
                );
                let _ = std::io::stdout().flush();
                sleep(Duration::from_millis(50));
            }
            println!("\x1B[2K\r\x1B[92mAwaiting program lock. Retrying now\x1B[0m...");
            settings = SettingsJson::get_settings().await.wrap()?;
        } else {
            break;
        }
    }
    if settings.sources.is_empty() {
        return Ok(Some(PostAction::PullSources));
    }
    settings.locked = true;
    settings.set_settings().await.wrap()?;
    Ok(None)
}

pub async fn remove_lock() -> Result<(), WrappedError> {
    let mut settings = SettingsJson::get_settings().await.wrap()?;
    settings.locked = false;
    settings.set_settings().await.wrap()
}
