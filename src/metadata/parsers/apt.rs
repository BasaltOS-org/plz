use debian_control::{
    Binary,
    lossless::{Control, Relations},
};
use lazy_regex::regex_captures_iter;
use snafu::{OptionExt, ResultExt, location};
use sqlx::SqlitePool;
use std::collections::HashSet;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::errors::{NetSnafu, OtherSnafu, StdIOSnafu, TokioIOSnafu, Wrapped, WrappedError};
use crate::metadata::{
    depend_kind::{self, DependKind},
    processed,
    processed::{PreBuilt, ProcessedMetaData},
    versioning::DepVer,
};
use crate::settings::{self, AptKind, Arch};
use crate::utils::{self, range::Range, tmpdir, verreq::VerReq, version::Version};

pub struct RawApt {
    package: String,
    version: String,
    installed_size: String,
    depends: String,
    filename: String,
    size: String,
    sha512: String,
    description: String,
}
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
        source: &str,
        code: &str,
        kind: &AptKind,
        name: &str,
        version: &str,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<ProcessedMetaData, WrappedError> {
        let folder = if name.starts_with("lib") && name.len() > 3 {
            name[0..4].to_string()
        } else if !name.is_empty() {
            name[0..1].to_string()
        } else {
            return Err(WrappedError::Other {
                error: format!("Invalid requested package name `{name}`!").into(),
                loc: location!(),
            });
        };
        let origin = format!("{source}/pool/{kind}/{folder}/{name}");
        let endpoint = format!("{origin}/{version}.deb");
        let response = reqwest::get(&endpoint).await.context(NetSnafu)?;
        let body = response.bytes().await.context(NetSnafu)?;
        let path = tmpdir().await.wrap()?;
        let deb = path.0.join("deb");
        let mut file = File::create(&deb).await.context(TokioIOSnafu)?;
        file.write_all(&body).await.context(TokioIOSnafu)?;
        let result = utils::command(
            "/usr/bin/ar",
            &["-x", &deb.to_string_lossy()],
            Some(&path.1),
        )
        .await;
        if result.is_none_or(|x| x != 0) {
            return Err(WrappedError::Other {
                error: format!("Failed to unpack package `{name}`!").into(),
                loc: location!(),
            });
        }
        let dir = path.0.read_dir().context(StdIOSnafu)?;
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
                )
                .await;
                if result.is_none_or(|x| x != 0) {
                    return Err(WrappedError::Other {
                        error: format!("Failed to untar package `{}`!", file_path.display()).into(),
                        loc: location!(),
                    });
                }
            }
        }
        let control_p = path.0.join("control");
        let mut control = File::open(&control_p).await.context(TokioIOSnafu)?;
        let mut c_data = String::new();
        control
            .read_to_string(&mut c_data)
            .await
            .context(TokioIOSnafu)?;
        let Ok(control) = Control::parse(&c_data).to_result() else {
            return Err(WrappedError::Other {
                error: format!(
                    // "File `{}` is not a valid DEB Control file!",
                    // control_p.display()
                    "Not a valid DEB Control file for package `{name}`."
                )
                .into(),
                loc: location!(),
            })?;
        };
        let binary = control.binaries().next().context(OtherSnafu {
            error: format!("Missing data in control file for package `{name}`."),
        })?;
        let arch = Self::get_arch(&binary.architecture().unwrap_or_default());
        if !arch.is_compatible(name).await.wrap()? {
            return Err(WrappedError::Other {
                error: format!("Incompatible machine architecture required by package `{name}`.")
                    .into(),
                loc: location!(),
            });
        }
        Self::to_processed(&binary, version, source, code, kind, dependent, pool)
            .await
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
    ) -> Result<ProcessedMetaData, WrappedError> {
        let package = binary.name().context(OtherSnafu {
            error: "Unnamed binary",
        })?;
        let description = binary.description().unwrap_or_default();
        let depends = binary.depends();
        let recommends = binary.recommends();
        let _suggests = binary.suggests();
        let deps = {
            let mut deps = HashSet::new();
            if let Some(depends) = depends {
                deps.extend(Self::to_depends(&depends, pool).await.wrap()?);
            }
            if let Some(recommends) = recommends {
                deps.extend(Self::to_depends(&recommends, pool).await.wrap()?);
            }
            // if let Some(suggests) = _suggests {
            //     deps.extend(Self::to_depends(&suggests)?);
            // }
            DependKind::collapse(deps).context(OtherSnafu{error: "Dependency conflict! The developer wishes you 'Good Luck' on your quest to figure out which dependency it is."})?
        };
        Ok(ProcessedMetaData {
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
            install_kind: processed::ProcessedInstallKind::PreBuilt(PreBuilt {
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
    async fn to_depends(
        relations: &Relations,
        pool: &SqlitePool,
    ) -> Result<HashSet<DependKind>, WrappedError> {
        let mut depends = HashSet::new();
        for versions in relations.to_string().split(",") {
            let mut choices = HashSet::new();
            for version in versions.split("|") {
                let (version, arch) = version.split_once(":").unwrap_or((version, "any"));
                let arch = Self::get_arch(arch);
                if !arch.is_compatible(version).await.wrap()? {
                    return Err(WrappedError::Other {
                        error:
                            "The architecture of this package is incompatible with your hardware."
                                .into(),
                        loc: location!(),
                    });
                };
                let version = version.trim();
                if let Some((name, ver)) = version.split_once(" )") {
                    let full_ver = ver.trim_end_matches(")");
                    let mut prior = Some(Range {
                        lower: VerReq::NoBound,
                        upper: VerReq::NoBound,
                    });
                    let (op, ver) = full_ver.split_at(2);
                    let Ok(ver) = Version::parse(ver) else {
                        return Err(WrappedError::Other {
                            error: format!("Version \"{}\" is not a valid Version!", ver).into(),
                            loc: location!(),
                        });
                    };
                    match op {
                        ">>" => prior = VerReq::Gt(ver).negotiate(prior),
                        ">=" => prior = VerReq::Ge(ver).negotiate(prior),
                        "=" => prior = VerReq::Eq(ver).negotiate(prior),
                        "<<" => prior = VerReq::Lt(ver).negotiate(prior),
                        "<=" => prior = VerReq::Le(ver).negotiate(prior),
                        _ => {
                            return Err(WrappedError::Other {
                                error: format!("`{}` is not a valid Version opcode!", op).into(),
                                loc: location!(),
                            });
                        }
                    }
                    let range = prior.context(OtherSnafu {
                        error: "No mutually agreeable version found!",
                    })?;
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
        Ok(depends)
    }
}
