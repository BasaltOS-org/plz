use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    thread::sleep,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Whatever, whatever};
use utils::{PostAction, get_dir, is_root};

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
    pub fn set_settings(self) -> Result<(), Whatever> {
        let mut file =
            File::create(affirm_path()?).whatever_context("Failed to open SettingsYaml as WO!")?;
        let settings = serde_norway::to_string(&self)
            .whatever_context("Failed to parse SettingsYaml to string!")?;
        file.write_all(settings.as_bytes())
            .whatever_context("Failed to write to file!")
    }
    pub fn get_settings() -> Result<Self, Whatever> {
        let mut file =
            File::open(affirm_path()?).whatever_context("Failed to open SettingsYaml as RO!")?;
        let mut sources = String::new();
        file.read_to_string(&mut sources)
            .whatever_context("Failed to read file!")?;
        serde_norway::from_str(&sources).whatever_context("Failed to parse data into SettingsYaml!")
    }
}

impl Default for SettingsYaml {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum OriginKind {
    Apt(String),
    Pax(String),
    Github { user: String, repo: String },
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
    pub fn is_compatible(&self, name: &str) -> Result<bool, Whatever> {
        let installed = SettingsYaml::get_settings()?.arch;
        match self {
            Self::Any => Ok(true),
            Self::X86_64v1 => Ok([Self::X86_64v1, Self::X86_64v3].contains(&installed)),
            Self::NoArch => {
                whatever!("The requested package `{name}` is of an unrecognized architecture!")
            }
            other => Ok(installed == *other),
        }
    }
}

fn affirm_path() -> Result<PathBuf, Whatever> {
    let mut path = get_dir()?;
    path.push("settings.yaml");
    if !path.exists() {
        let mut file = File::create(&path).whatever_context("Failed to create settings file!")?;
        if let Ok(new_settings) = serde_norway::to_string(&SettingsYaml::new()) {
            file.write_all(new_settings.as_bytes())
                .whatever_context("Failed to write to file!")?;
            Ok(path)
        } else {
            whatever!("Failed to serialize settings!")
        }
    } else if path.is_file() {
        File::open(&path).whatever_context("Failed to read settings file!")?;
        Ok(path)
    } else {
        whatever!("Settings file is of unexpected type!")
    }
}

pub fn acquire_lock() -> Result<Option<PostAction>, Whatever> {
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

pub fn remove_lock() -> Result<(), Whatever> {
    let mut settings = SettingsYaml::get_settings()?;
    settings.locked = false;
    settings.set_settings()
}
