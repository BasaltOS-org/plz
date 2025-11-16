use std::{
    collections::HashSet,
    fs::File,
    io::{ErrorKind, Read, Write},
};

use debian_control::{
    Binary,
    lossless::{Control, Relations},
};
use lazy_regex::regex_captures_iter;
use settings::{AptKind, Arch};
use snafu::{OptionExt, ResultExt};
use sqlx::SqlitePool;
use utils::{
    Range, VerReq, Version,
    errors::{HowError, IOAction, IOSnafu, NetSnafu, SystemSnafu, WhereError},
    tmpdir,
};

use crate::{
    FuckWrap,
    depend_kind::{self, DependKind},
    processed::{PreBuilt, ProcessedMetaData},
    versioning::DepVer,
};

pub struct RawApt {}
impl RawApt {
    pub async fn get_vers(
        source: &str,
        _code: &str,
        kind: &str,
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
        let endpoint = format!("{source}/pool/{kind}/{folder}/{name}");
        let Ok(response) = reqwest::get(dbg!(endpoint)).await else {
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
        source: &str,
        code: &str,
        kind: &AptKind,
        name: &str,
        version: &str,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<ProcessedMetaData, WhereError> {
        let folder = if name.starts_with("lib") && name.len() > 3 {
            name[0..4].to_string()
        } else if !name.is_empty() {
            name[0..1].to_string()
        } else {
            return Err(HowError::SystemError {
                message: "Invalid requested package name".into(),
                package: name.to_string().into(),
            })
            .wrap();
        };
        let origin = format!("{source}/pool/{kind}/{folder}/{name}");
        let endpoint = format!("{origin}/{version}.deb");
        let response = reqwest::get(&endpoint)
            .await
            .context(NetSnafu {
                loc: endpoint.to_string(),
            })
            .wrap()?;
        let body = response
            .bytes()
            .await
            .context(NetSnafu { loc: endpoint })
            .wrap()?;
        let path = tmpdir()
            .context(SystemSnafu {
                message: "Failed to reserve a directory",
                package: name.to_string(),
            })
            .wrap()?;
        let deb = path.0.join("deb");
        let mut file = File::create(&deb)
            .context(IOSnafu {
                action: IOAction::CreateFile,
                loc: deb.display().to_string(),
            })
            .wrap()?;
        file.write_all(&body)
            .context(IOSnafu {
                action: IOAction::WriteFile,
                loc: deb.display().to_string(),
            })
            .wrap()?;
        let result = utils::command(
            "/usr/bin/ar",
            &["-x", &deb.to_string_lossy()],
            Some(&path.1),
        );
        if result.is_none_or(|x| x != 0) {
            return Err(HowError::SystemError {
                message: "Unpack failed".into(),
                package: name.to_string().into(),
            })
            .wrap();
        }
        let dir = path
            .0
            .read_dir()
            .context(IOSnafu {
                action: IOAction::ReadDir,
                loc: path.0.display().to_string(),
            })
            .wrap()?;
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
                    return Err(HowError::SystemError {
                        message: "Untar failed".into(),
                        package: file_path.display().to_string().into(),
                    })
                    .wrap();
                }
            }
        }
        let control_p = path.0.join("control");
        let mut control = File::open(&control_p)
            .context(IOSnafu {
                action: IOAction::OpenFile,
                loc: control_p.display().to_string(),
            })
            .wrap()?;
        let mut c_data = String::new();
        control
            .read_to_string(&mut c_data)
            .context(IOSnafu {
                action: IOAction::ReadFile,
                loc: control_p.display().to_string(),
            })
            .wrap()?;
        let Ok(control) = Control::parse(&c_data).to_result() else {
            return Err(HowError::IOError {
                source: ErrorKind::InvalidData.into(),
                action: IOAction::CorruptedFile,
                loc: control_p.display().to_string().into(),
            })
            .wrap();
        };
        let binary = control
            .binaries()
            .next()
            .context(SystemSnafu {
                message: "Missing data in control file",
                package: name.to_string(),
            })
            .wrap()?;
        dbg!(binary.to_string());
        let arch = Self::get_arch(&binary.architecture().unwrap_or_default());
        if !arch.is_compatible(name).wrap()? {
            return Err(HowError::SystemError {
                message: "Incompatible machine architecture".into(),
                package: name.to_string().into(),
            })
            .wrap();
        }
        Self::to_processed(&binary, version, source, code, kind, dependent, pool)
            .await
            .context(SystemSnafu {
                message: "Invalid control file",
                package: name.to_string(),
            })
            .wrap()
    }
    pub async fn to_processed(
        binary: &Binary,
        version: &str,
        // origin: &str,
        source: &str,
        code: &str,
        kind: &AptKind,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Option<ProcessedMetaData> {
        let package = binary.name()?;
        let description = binary.description().unwrap_or_default();
        let depends = binary.depends();
        let recommends = binary.recommends();
        let _suggests = binary.suggests();
        let deps = {
            let mut deps = HashSet::new();
            if let Some(depends) = depends {
                deps.extend(Self::to_depends(&depends, pool).await?);
            }
            if let Some(recommends) = recommends {
                deps.extend(Self::to_depends(&recommends, pool).await?);
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
            origin: settings::OriginKind::Apt {
                source: source.to_string(),
                code: code.to_string(),
                kind: kind.clone(),
            },
            dependent,
            build_dependencies: depend_kind::DependKindVec(Vec::new()),
            runtime_dependencies: depend_kind::DependKindVec(deps),
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
    async fn to_depends(relations: &Relations, pool: &SqlitePool) -> Option<HashSet<DependKind>> {
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
            depends.extend(DependKind::choose(choices, pool).await);
        }
        Some(depends)
    }
}
