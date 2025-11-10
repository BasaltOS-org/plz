use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write},
};

use debian_control::{
    Binary,
    lossless::{Control, Relations},
};
use lazy_regex::regex_captures_iter;
use settings::Arch;
use snafu::{OptionExt, ResultExt, Whatever, whatever};
use utils::{Range, VerReq, Version, tmpdir};

use crate::{
    depend_kind::DependKind,
    processed::{PreBuilt, ProcessedMetaData},
    versioning::DepVer,
};

pub struct RawApt {}
impl RawApt {
    pub async fn get_vers(
        mirror: &str,
        prefer: Option<&str>,
        name: &str,
    ) -> HashSet<(String, Version, Arch)> {
        // example mirror: https://au.archive.ubuntu.com/ubuntu/pool/universe/
        // example prefer: https://au.archive.ubuntu.com/ubuntu/pool/universe/n/node/
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
        let Ok(mut body) = response.text().await else {
            return vers;
        };
        if let Some(prefer) = prefer {
            let Ok(response) = reqwest::get(prefer).await else {
                return vers;
            };
            let Ok(p_body) = response.text().await else {
                return vers;
            };
            body = format!("{p_body}{body}")
        }
        let captures = regex_captures_iter!(r#"([\d\.\-\+_a-zA-Z]+)_(.*?)\.deb""#, &body);
        captures
            .flat_map(|x| {
                let (_, [name_v, arch]) = x.extract();
                let (just_name, version) = name_v.split_once('_')?;
                if just_name != name {
                    return None;
                }
                let full_name = format!("{name_v}_{arch}");
                Some((
                    full_name,
                    Version::parse(version).ok()?,
                    Self::get_arch(arch),
                ))
            })
            .collect::<HashSet<(String, Version, Arch)>>()
    }
    pub async fn parse(
        mirror: &str,
        name: &str,
        version: &str,
        dependent: bool,
    ) -> Result<ProcessedMetaData, Whatever> {
        let folder = if name.starts_with("lib") && name.len() > 3 {
            name[0..4].to_string()
        } else if !name.is_empty() {
            name[0..1].to_string()
        } else {
            whatever!("`{name}` is not a valid APT package name!")
        };
        let origin = format!("{mirror}/{folder}/{name}");
        let endpoint = format!("{origin}/{version}.deb");
        let response = reqwest::get(&endpoint).await.with_whatever_context(|_| {
            format!("Failed to pull APT package data for package `{name}`!")
        })?;
        let body = response.bytes().await.with_whatever_context(|_| {
            format!("Failed to read pulled APT data for package `{name}`!")
        })?;
        let path = tmpdir().with_whatever_context(|| {
            format!("Failed to allocate a tmp file for package `{name}`!")
        })?;
        let deb = path.0.join("deb");
        let mut file = File::create(&deb)
            .with_whatever_context(|_| format!("Failed to open tmp file for package `{name}`!"))?;
        file.write_all(&body)
            .with_whatever_context(|_| format!("Failed to write package `{name}`!"))?;
        let result = utils::command(
            "/usr/bin/ar",
            &["-x", &deb.to_string_lossy()],
            Some(&path.1),
        );
        if result.is_none_or(|x| x != 0) {
            whatever!("Failed to unpack `{name}`!")
        }
        let dir = path
            .0
            .read_dir()
            .with_whatever_context(|_| format!("{} is not a directory!", &path.0.display()))?;
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
                    whatever!("Failed to untar `{}`!", file_path.to_string_lossy())
                }
            }
        }
        let mut control = File::open(path.0.join("control"))
            .with_whatever_context(|_| format!("Missing control file for package `{name}`!"))?;
        let mut c_data = String::new();
        control
            .read_to_string(&mut c_data)
            .with_whatever_context(|_| {
                format!("Failed to read control file for package `{name}`!")
            })?;
        let control = Control::parse(&c_data)
            .to_result()
            .with_whatever_context(|_| format!("Corrupted control file for package `{name}`!"))?;
        let binary = control.binaries().next().with_whatever_context(|| {
            format!("Control file for package `{name}` is missing binary information!")
        })?;
        dbg!(binary.to_string());
        let arch = Self::get_arch(&binary.architecture().unwrap_or_default());
        if !arch.is_compatible(name)? {
            whatever!("Package `{name}` is not compatible with this machine's architecture!")
        }
        Self::to_processed(&binary, version, &origin, dependent)
            .with_whatever_context(|| format!("Invalid control file for package `{name}`!"))
    }
    pub fn to_processed(
        binary: &Binary,
        version: &str,
        origin: &str,
        dependent: bool,
    ) -> Option<ProcessedMetaData> {
        let package = binary.name()?;
        let description = binary.description().unwrap_or_default();
        let depends = binary.depends();
        let recommends = binary.recommends();
        let _suggests = binary.suggests();
        let deps = {
            let mut deps = HashSet::new();
            if let Some(depends) = depends {
                deps.extend(Self::to_depends(&depends)?);
            }
            if let Some(recommends) = recommends {
                deps.extend(Self::to_depends(&recommends)?);
            }
            // if let Some(suggests) = _suggests {
            //     deps.extend(Self::to_depends(&suggests)?);
            // }
            DependKind::collapse(deps)?
        };
        Some(ProcessedMetaData {
            name: package,
            kind: super::MetaDataKind::Apt,
            description,
            version: version.to_string(),
            origin: settings::OriginKind::Apt(origin.to_string()),
            dependent,
            build_dependencies: Vec::new(),
            runtime_dependencies: deps,
            install_kind: crate::processed::ProcessedInstallKind::PreBuilt(PreBuilt {
                critical: Vec::new(),
                configs: Vec::new(),
            }),
            hash: String::new(),
        })
    }
    fn get_arch(arch: &str) -> Arch {
        match arch {
            "all" | "any" => Arch::Any,
            "amd64" => Arch::X86_64v1,
            "arm64" => Arch::Aarch64,
            _ => Arch::NoArch,
        }
    }
    fn to_depends(relations: &Relations) -> Option<HashSet<DependKind>> {
        let mut depends = HashSet::new();
        for versions in relations.to_string().split(",") {
            let mut choices = HashSet::new();
            for version in versions.split("|") {
                let (version, arch) = version.split_once(":").unwrap_or((version, "any"));
                let arch = Self::get_arch(arch);
                if !arch.is_compatible(version).ok()? {
                    return None;
                }
                let version = version.trim();
                if let Some((name, ver)) = version.split_once(" )") {
                    let full_ver = ver.trim_end_matches(")");
                    let mut prior = Some(Range {
                        lower: VerReq::NoBound,
                        upper: VerReq::NoBound,
                    });
                    let Ok(ver) = Version::parse(full_ver[2..].trim()) else {
                        return None;
                    };
                    match dbg!(&full_ver[..2]) {
                        ">>" => prior = VerReq::Gt(ver).negotiate(prior),
                        ">=" => prior = VerReq::Ge(ver).negotiate(prior),
                        "=" => prior = VerReq::Eq(ver).negotiate(prior),
                        "<<" => prior = VerReq::Lt(ver).negotiate(prior),
                        "<=" => prior = VerReq::Le(ver).negotiate(prior),
                        _ => return None,
                    }
                    let range = prior?;
                    choices.insert(DependKind::Specific(DepVer {
                        name: name.to_string(),
                        range,
                    }));
                } else {
                    choices.insert(DependKind::Latest(version.to_string()));
                }
            }
            depends.extend(DependKind::choose(choices));
        }
        Some(depends)
    }
}
