use serde::{Deserialize, Serialize};
use serde_json::json;
use snafu::{OptionExt, ResultExt};
use sqlx::{Decode, Encode, Sqlite, Type};
use std::{fmt::Display, io::Write, path::PathBuf, thread::sleep};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    time::Duration,
};

use crate::errors::{
    JSONSnafu, OtherSnafu, StatefulError, StdIOSnafu, TokioIOSnafu, Wrapped, WrappedWith,
};
use crate::utils::{PostAction, get_dir, is_root, which};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsJson {
    pub locked: bool,
    pub shell: ShellType,
    pub version: String,
    pub arch: Arch,
    pub exec: Option<String>,
    pub sources: Vec<OriginKind>,
}

impl SettingsJson {
    pub fn new() -> Result<Self, StatefulError> {
        let shell = if which("fish") {
            ShellType::Fish
        } else if which("bash") {
            ShellType::Bash
        } else if which("zsh") {
            ShellType::Zsh
        } else if which("ash") {
            ShellType::Ash
        } else if which("sh") {
            ShellType::Sh
        } else {
            return Err(StatefulError::new(
                "No compatible shell interpreters installed!",
            ));
        };
        let mut command = std::process::Command::new("uname");
        let arch = if let Ok(output) = command.arg("-m").output() {
            match String::from_utf8_lossy(&output.stdout)
                .to_string()
                .as_str()
                .trim()
            {
                "x86_64" => {
                    let mut command = std::process::Command::new("cat");
                    command.arg("/proc/cpuinfo");
                    let output = command.output().context(StdIOSnafu).wrap()?;
                    let splits = String::from_utf8_lossy(&output.stdout);
                    let splits = splits.split_whitespace().collect::<Vec<&str>>();
                    if ["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"]
                        .iter()
                        .all(|x| splits.contains(x))
                    {
                        Arch::X86_64v4
                    } else if [
                        "avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "abm", "movbe", "xsave",
                    ]
                    .iter()
                    .all(|x| splits.contains(x))
                    {
                        Arch::X86_64v3
                    } else if ["cx16", "lahf", "popcnt", "sse4_1", "sse4_2", "ssse3"]
                        .iter()
                        .all(|x| splits.contains(x))
                    {
                        Arch::X86_64v2
                    } else {
                        Arch::X86_64
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
        Ok(Self {
            locked: false,
            shell,
            version: env!("SETTINGS_JSON_VERSION").to_string(),
            arch,
            exec: None,
            sources: Vec::new(),
        })
    }
    pub async fn set_settings(self) -> Result<(), StatefulError> {
        let mut file = File::create(
            affirm_path()
                .await
                .wrap(&json!({"action": "writing settings to disk"}))?,
        )
        .await
        .context(TokioIOSnafu)
        .wrap()?;
        let settings = serde_json::to_string(&self).context(JSONSnafu).wrap()?;
        file.write_all(settings.as_bytes())
            .await
            .context(TokioIOSnafu)
            .wrap()
    }
    pub async fn get_settings() -> Result<Self, StatefulError> {
        let mut file = File::open(
            affirm_path()
                .await
                .wrap(&json!({"action": "reading settings from disk"}))?,
        )
        .await
        .context(TokioIOSnafu)
        .wrap()?;
        let mut sources = String::new();
        file.read_to_string(&mut sources)
            .await
            .context(TokioIOSnafu)
            .wrap()?;
        serde_json::from_str(&sources).context(JSONSnafu).wrap()
    }
    // async fn shell(&mut self) -> Result<String, StatefulError> {
    //     match self.shell {
    //         ShellType::Fish => Ok(String::from("fish")),
    //         ShellType::Bash => Ok(String::from("bash")),
    //         ShellType::Ash => Ok(String::from("ash")),
    //         ShellType::Sh => Ok(String::from("sh")),
    //         ShellType::None => {

    //         }
    //     }
    // }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum OriginKind {
    Apt {
        source: String,
        code: String,
        kind: AptKind,
    },
    Plz(String),
    Github {
        user: String,
        repo: String,
    },
}

impl OriginKind {
    fn parse(input: &str) -> Result<Self, StatefulError> {
        let mut chars = input.chars();
        let kind = chars
            .next()
            .ok_or(StatefulError::new("Missing type identifier!"))?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => {
                let mut splits = data.split(' ');
                let ((source, code), kind) = splits
                    .next()
                    .zip(splits.next())
                    .zip(splits.next())
                    .context(OtherSnafu {
                        error: "Missing required APT fields!",
                    })
                    .wrap()?;
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
            1 => Ok(Self::Plz(data.to_string())),
            2 => {
                let (user, repo) = data
                    .split_once(' ')
                    .context(OtherSnafu {
                        error: "Missing GH field `repo`!",
                    })
                    .wrap()?;
                Ok(Self::Github {
                    user: user.to_string(),
                    repo: repo.to_string(),
                })
            }
            kind => Err(StatefulError::new(format!(
                "Invalid kind identifier `{kind}`!"
            ))),
        }
    }
}

impl Display for OriginKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::Apt { source, code, kind } => {
                format!("\x00{source} {code} {kind}")
            }
            Self::Plz(plz) => format!("\x01{plz}"),
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
        Ok(Self::parse(&data)
            .wrap(&json!({"action": "decoding APT origins from settings"}))
            .map_err(|e| {
                OtherSnafu {
                    error: e.to_string(),
                }
                .build()
            })?)
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
    X86_64,
    X86_64v2,
    X86_64v3,
    X86_64v4,
    Aarch64,
    Armv7l,
    Armv8l,
    NoArch,
}

impl Arch {
    pub async fn is_compatible(&self, name: &str) -> Result<bool, StatefulError> {
        let installed = SettingsJson::get_settings()
            .await
            .wrap(&json!({"action": "asserting CPU architecture compatibility"}))?
            .arch;
        match self {
            Self::Any => Ok(true),
            Self::X86_64 => Ok(
                [Self::X86_64, Self::X86_64v2, Self::X86_64v3, Self::X86_64v4].contains(&installed),
            ),
            Self::NoArch => Err(StatefulError::new(format!(
                "Unrecognized architecture in package {name}!"
            ))),
            other => Ok(installed == *other),
        }
    }
}

async fn affirm_path() -> Result<PathBuf, StatefulError> {
    let cause = json!({"action": "affirming settings file exists"});
    let mut path = get_dir().await.wrap(&cause)?;
    path.push("settings.json");
    if !path.exists() {
        let mut file = File::create(&path).await.context(TokioIOSnafu).wrap()?;
        let new_settings = serde_json::to_string(&SettingsJson::new().wrap(&cause)?)
            .context(JSONSnafu)
            .wrap()?;

        file.write_all(new_settings.as_bytes())
            .await
            .context(TokioIOSnafu)
            .wrap()?;
        Ok(path)
    } else if path.is_file() {
        Ok(path)
    } else {
        Err(StatefulError::new(format!(
            "Path {} is not of the expected type. Is it a real file?",
            path.display()
        )))
    }
}

pub async fn acquire_lock() -> Result<Option<PostAction>, StatefulError> {
    let cause = json!({"action": "locking the settings file to our process"});
    if !is_root() {
        return Ok(Some(PostAction::Elevate));
    }
    let mut settings = SettingsJson::get_settings().await.wrap(&cause)?;
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
            settings = SettingsJson::get_settings().await.wrap(&cause)?;
        } else {
            break;
        }
    }
    settings.locked = true;
    settings.set_settings().await.wrap(&cause)?;
    Ok(None)
}

pub async fn remove_lock() -> Result<(), StatefulError> {
    let cause = json!({"action": "unlocking the settings file from our process"});
    let mut settings = SettingsJson::get_settings().await.wrap(&cause)?;
    settings.locked = false;
    settings.set_settings().await.wrap(&cause)
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub enum ShellType {
    Fish,
    Zsh,
    Bash,
    Ash,
    Sh,
    Custom(String),
}
impl Display for ShellType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fish => "fish",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Ash => "ash",
            Self::Sh => "sh",
            Self::Custom(custom) => custom,
        })
    }
}
