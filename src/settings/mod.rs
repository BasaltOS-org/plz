use serde::{Deserialize, Serialize};
use serde_json::json;
use snafu::ResultExt;
use std::{fmt::Display, io::Write, path::PathBuf, thread::sleep};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    time::Duration,
};

use crate::utils::{PostAction, get_dir, is_root, which};
use crate::{
    errors::{JSONSnafu, StatefulError, StdIOSnafu, TokioIOSnafu, Wrapped},
    settings::originkind::OriginKindVec,
};

mod originkind;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsJson {
    pub locked: bool,
    pub shell: ShellType,
    pub version: String,
    pub arch: Arch,
    pub exec: Option<String>,
    pub sources: OriginKindVec,
}

impl SettingsJson {
    pub fn new() -> Result<Self, StatefulError> {
        let cause = json!({"action": "constructing settings"});
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
                &cause,
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
                    let output = command.output().context(StdIOSnafu).wrap(&cause)?;
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
        let cause = json!({"action": "writing settings to disk"});
        let mut file = File::create(affirm_path().await.wrap(&cause)?)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let settings = serde_json::to_string(&self)
            .context(JSONSnafu)
            .wrap(&cause)?;
        file.write_all(settings.as_bytes())
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)
    }
    pub async fn get_settings() -> Result<Self, StatefulError> {
        let cause = json!({"action": "reading settings from disk"});
        let mut file = File::open(affirm_path().await.wrap(&cause)?)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let mut sources = String::new();
        file.read_to_string(&mut sources)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        serde_json::from_str(&sources)
            .context(JSONSnafu)
            .wrap(&cause)
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
        let cause = json!({"action": "asserting CPU architecture compatibility"});
        let installed = SettingsJson::get_settings().await.wrap(&cause)?.arch;
        match self {
            Self::Any => Ok(true),
            Self::X86_64 => Ok(
                [Self::X86_64, Self::X86_64v2, Self::X86_64v3, Self::X86_64v4].contains(&installed),
            ),
            Self::NoArch => Err(StatefulError::new(
                format!("Unrecognized architecture in package {name}!"),
                &cause,
            )),
            other => Ok(installed == *other),
        }
    }
}

async fn affirm_path() -> Result<PathBuf, StatefulError> {
    let cause = json!({"action": "affirming settings file exists"});
    let mut path = get_dir().await.wrap(&cause)?;
    path.push("settings.json");
    if !path.exists() {
        let mut file = File::create(&path)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let new_settings = serde_json::to_string(&SettingsJson::new().wrap(&cause)?)
            .context(JSONSnafu)
            .wrap(&cause)?;

        file.write_all(new_settings.as_bytes())
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        Ok(path)
    } else if path.is_file() {
        Ok(path)
    } else {
        Err(StatefulError::new(
            format!(
                "Path {} is not of the expected type. Is it a real file?",
                path.display()
            ),
            &cause,
        ))
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
