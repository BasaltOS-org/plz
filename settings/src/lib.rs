use std::{
    fmt::Display,
    fs::File,
    io::{ErrorKind, Read, Write},
    path::PathBuf,
    thread::sleep,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use utils::{
    PostAction,
    errors::{HowError, IOAction, IOSnafu, YAMLSnafu},
    get_dir, is_root,
};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsYaml {
    pub locked: bool,
    pub version: String,
    pub arch: Arch,
    pub exec: Option<String>,
    pub sources: Vec<OriginKind>,
}

impl SettingsYaml {
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
            version: env!("SETTINGS_YAML_VERSION").to_string(),
            arch,
            exec: None,
            sources: Vec::new(),
        }
    }
    pub fn set_settings(self) -> Result<(), HowError> {
        let loc = "SettingsYAML";
        let mut file = File::create(affirm_path()?).context(IOSnafu {
            action: IOAction::CreateFile,
            loc,
        })?;
        let settings = serde_norway::to_string(&self).context(YAMLSnafu { loc })?;
        file.write_all(settings.as_bytes()).context(IOSnafu {
            action: IOAction::WriteFile,
            loc,
        })
    }
    pub fn get_settings() -> Result<Self, HowError> {
        let loc = "SettingsYAML";
        let mut file = File::open(affirm_path()?).context(IOSnafu {
            action: IOAction::OpenFile,
            loc,
        })?;
        let mut sources = String::new();
        file.read_to_string(&mut sources).context(IOSnafu {
            action: IOAction::ReadFile,
            loc,
        })?;
        serde_norway::from_str(&sources).context(YAMLSnafu { loc: "YAML" })
    }
}

impl Default for SettingsYaml {
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
    Pax(String),
    Github {
        user: String,
        repo: String,
    },
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
    pub fn is_compatible(&self, name: &str) -> Result<bool, HowError> {
        let installed = SettingsYaml::get_settings()?.arch;
        match self {
            Self::Any => Ok(true),
            Self::X86_64v1 => Ok([Self::X86_64v1, Self::X86_64v3].contains(&installed)),
            Self::NoArch => Err(HowError::SystemError {
                message: "Unrecognized architecture".into(),
                package: name.to_string().into(),
            }),
            other => Ok(installed == *other),
        }
    }
}

fn affirm_path() -> Result<PathBuf, HowError> {
    let mut path = get_dir()?;
    path.push("settings.yaml");
    if !path.exists() {
        let mut file = File::create(&path).context(IOSnafu {
            action: IOAction::CreateFile,
            loc: "SettingsYAML",
        })?;
        let new_settings = serde_norway::to_string(&SettingsYaml::new()).context(YAMLSnafu {
            loc: "SettingsYAML",
        })?;

        file.write_all(new_settings.as_bytes()).context(IOSnafu {
            action: IOAction::WriteFile,
            loc: "SettingsYAML",
        })?;
        Ok(path)
    } else if path.is_file() {
        Ok(path)
    } else {
        Err(HowError::IOError {
            source: ErrorKind::NotSeekable.into(),
            action: IOAction::AssertPath,
            loc: path.display().to_string().into(),
        })
    }
}

pub fn acquire_lock() -> Result<Option<PostAction>, HowError> {
    if !is_root() {
        return Ok(Some(PostAction::Elevate));
    }
    let mut settings = SettingsYaml::get_settings()?;
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
            settings = SettingsYaml::get_settings()?;
        } else {
            break;
        }
    }
    if settings.sources.is_empty() {
        return Ok(Some(PostAction::PullSources));
    }
    settings.locked = true;
    settings.set_settings()?;
    Ok(None)
}

pub fn remove_lock() -> Result<(), HowError> {
    let mut settings = SettingsYaml::get_settings()?;
    settings.locked = false;
    settings.set_settings()
}

pub trait FuckExt<T, E>: Sized {
    fn wrap<E2: From<HowError>>(self, loc: &'static str) -> Result<T, E2>;
}
