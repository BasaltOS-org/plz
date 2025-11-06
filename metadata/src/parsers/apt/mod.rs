use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write},
};

use fancy_regex::Regex;
use serde::Deserialize;
use settings::Arch;
use utils::{Version, err, tmpdir};

use crate::processed::ProcessedMetaData;

#[derive(Debug, Deserialize)]
#[allow(unused, non_snake_case)]
pub struct RawApt {
    Package: String,
    Version: String,
    Description: String,
    Arch: Option<String>,
    Depends: Option<String>,
    Recommends: Option<String>,
    Suggests: Option<String>,
}
impl RawApt {
    pub async fn get_vers(mirror: &str, name: &str) -> HashSet<(String, Version, Arch)> {
        // example mirror: https://mirror.aarnet.edu.au/pub/ubuntu/archive/pool/universe
        let vers = HashSet::new();
        let folder = if name.starts_with("lib") && name.len() > 3 {
            name[0..4].to_string()
        } else if !name.is_empty() {
            name[0..1].to_string()
        } else {
            return vers;
        };
        let endpoint = format!("{mirror}/{folder}/{name}");
        let Ok(response) = reqwest::get(endpoint).await else {
            return vers;
        };
        let Ok(body) = response.text().await else {
            return vers;
        };
        let Ok(re) = Regex::new(r#"[\d\.\-\+\_a-zA-Z]+(?=_(.*?)\.deb\")"#) else {
            return vers;
        };
        let captures = re
            .captures_iter(&body)
            .flatten()
            .collect::<Vec<fancy_regex::Captures>>();
        captures
            .iter()
            .flat_map(|x| {
                let version = format!("{}_{}", &x[0], &x[1]);
                Some((
                    version,
                    Version::parse(x[0].trim_start_matches(&format!("{name}_"))).ok()?,
                    Self::get_arch(&x[1]),
                ))
            })
            .collect::<HashSet<(String, Version, Arch)>>()
    }
    pub async fn to_raw_apt(mirror: &str, name: &str, version: &str) -> Result<RawApt, String> {
        let folder = if name.starts_with("lib") && name.len() > 3 {
            name[0..4].to_string()
        } else if !name.is_empty() {
            name[0..1].to_string()
        } else {
            return err!("`{name}` is not a valid APT package name!");
        };
        let endpoint = format!("{mirror}/{folder}/{name}/{version}.deb");
        let Ok(response) = reqwest::get(&endpoint).await else {
            return err!("Failed to pull APT package data for package `{name}`!");
        };
        let Ok(body) = response.bytes().await else {
            return err!("Failed to read pulled APT data for package `{name}`!");
        };
        let Some(path) = dbg!(tmpdir()) else {
            return err!("Failed to allocate a tmp file for package `{name}`!");
        };
        let deb = path.0.join("deb");
        let Ok(mut file) = File::create(&deb) else {
            return err!("Failed to open tmp file for package `{name}`!");
        };
        if file.write_all(&body).is_err() {
            return err!("Failed to write package `{name}`!");
        };
        let result = utils::command(
            "/usr/bin/ar",
            &["-x", &deb.to_string_lossy()],
            Some(&path.1),
        );
        if result.is_none_or(|x| x != 0) {
            return err!("Failed to unpack `{name}`!");
        }
        if let Ok(dir) = path.0.read_dir() {
            for entry in dir.flatten() {
                let file_path = entry.path();
                if let Some(Some(ext)) = file_path.extension().map(|x| x.to_str()) {
                    let arg = match ext {
                        "gz" => "-xzf",
                        "xz" => "-xJf",
                        "bz2" => "-xjf",
                        _ => continue,
                    };
                    let result = utils::command(
                        "/usr/bin/tar",
                        &[arg, &file_path.to_string_lossy()],
                        Some(&path.1),
                    );
                    if result.is_none_or(|x| x != 0) {
                        return err!("Failed to untar `{}`!", file_path.to_string_lossy());
                    }
                }
            }
        } else {
            return err!("{} is not a directory!", &path.0.display());
        }
        let Ok(mut control) = File::open(path.0.join("control")) else {
            return err!("Missing control file for package `{name}`!");
        };
        let mut c_data = String::new();
        if control.read_to_string(&mut c_data).is_err() {
            return err!("Failed to read control file for package `{name}`!");
        }
        let Ok(re) = Regex::new("^ ") else {
            return err!("Malformed regex @`/metadata/src/parsers/apt/mod.rs:to_raw_apt`!");
        };
        let data = re.replace_all(&c_data, " ");
        if let Ok(raw_apt) = serde_norway::from_str::<Self>(&data) {
            Ok(raw_apt)
        } else {
            err!("Invalid control file for package `{name}`!")
        }
        // ar -x deb.deb
        // with tar -x_f command, -z for .tar.gz, -J for .tar.xz, and -j for .tar.bz2
        // (match file ending)
        // tar -xzf data.tar.gz
        // mkdir DEBIAN
        // tar -xzf control.tar.gz -C DEBIAN control
    }
    pub fn to_processed(self) -> Option<ProcessedMetaData> {
        None
    }
    fn get_arch(arch: &str) -> Arch {
        match arch {
            "all" => Arch::Any,
            "amd64" => Arch::X86_64v1,
            "arm64" => Arch::Aarch64,
            _ => Arch::NoArch,
        }
    }
}
