use debian_control::{
    Binary,
    lossless::{Control, Relations},
};
use lazy_regex::regex_captures_iter;
use serde_json::json;
use snafu::{OptionExt, ResultExt};
use sqlx::SqlitePool;
use std::collections::HashSet;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::errors::{NetSnafu, OtherSnafu, StatefulError, StdIOSnafu, TokioIOSnafu, Wrapped};
use crate::metadata::parsers::MetaDataKind;
use crate::metadata::{
    depend_kind::{self, DependKind},
    processed,
    processed::{PreBuilt, ProcessedMetaData},
    versioning::DepVer,
};
use crate::settings::{Arch, originkind::OriginKind};
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
        endpoint: &str,
        prefer: Option<&str>,
        name: &str,
    ) -> HashSet<(String, Version, Arch)> {
        let vers = HashSet::new();
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
        source: &OriginKind,
        // source: &str,
        // kind: &AptKind,
        endpoint: &str,
        name: &str,
        version: &str,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<ProcessedMetaData, StatefulError> {
        let cause = json!({"action": "parsing package into processable metadata", "package": name});
        let endpoint = format!("{endpoint}/{version}.deb");
        let response = reqwest::get(&endpoint)
            .await
            .context(NetSnafu)
            .wrap(&cause)?;
        let body = response.bytes().await.context(NetSnafu).wrap(&cause)?;
        let path = tmpdir().await.wrap(&cause)?;
        let deb = path.0.join("deb");
        let mut file = File::create(&deb)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        file.write_all(&body)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let result = utils::command("ar", &["-x", &deb.to_string_lossy()], Some(&path.1)).await;
        if result.is_none_or(|x| x != 0) {
            return Err(StatefulError::new(
                format!("Failed to unpack package `{name}`!"),
                &cause,
            ));
        }
        let dir = path.0.read_dir().context(StdIOSnafu).wrap(&cause)?;
        for entry in dir.flatten() {
            let file_path = entry.path();
            if let Some(Some(ext)) = file_path.extension().map(|x| x.to_str()) {
                let arg = match ext {
                    "gz" => "-xzf",
                    "xz" => "-xJf",
                    "bz2" => "-xjf",
                    "zst" => "-xf",
                    _ => continue,
                };
                let result =
                    utils::command("tar", &[arg, &file_path.to_string_lossy()], Some(&path.1))
                        .await;
                if result.is_none_or(|x| x != 0) {
                    return Err(StatefulError::new(
                        format!("Failed to untar package `{}`!", file_path.display()),
                        &cause,
                    ));
                }
            }
        }
        let control_p = path.0.join("control");
        let mut control = File::open(&control_p)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let mut c_data = String::new();
        control
            .read_to_string(&mut c_data)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let Ok(control) = Control::parse(&c_data).to_result() else {
            return Err(StatefulError::new(
                format!(
                    // "File `{}` is not a valid DEB Control file!",
                    // control_p.display()
                    "Not a valid DEB Control file for package `{name}`."
                ),
                &cause,
            ))?;
        };
        let binary = control
            .binaries()
            .next()
            .context(OtherSnafu {
                error: format!("Missing data in control file for package `{name}`."),
            })
            .wrap(&cause)?;
        let arch = Self::get_arch(&binary.architecture().unwrap_or_default());
        if !arch.is_compatible(name).await.wrap(&cause)? {
            return Err(StatefulError::new(
                format!("Incompatible machine architecture required by package `{name}`."),
                &cause,
            ));
        }
        Self::to_processed(&binary, version, source, dependent, pool)
            .await
            .wrap(&cause)
    }
    pub async fn to_processed(
        binary: &Binary,
        version: &str,
        source: &OriginKind,
        // kind: &AptKind,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<ProcessedMetaData, StatefulError> {
        let cause = json!({"action": "parsing APT binary package into processable metadata", "package": binary.name()});
        let package = binary
            .name()
            .context(OtherSnafu {
                error: "Unnamed binary",
            })
            .wrap(&cause)?;
        let description = binary.description().unwrap_or_default();
        let depends = binary.depends();
        let recommends = binary.recommends();
        let _suggests = binary.suggests();
        let deps = {
            let mut deps = HashSet::new();
            if let Some(depends) = depends {
                deps.extend(Self::to_depends(&depends, pool).await.wrap(&cause)?);
            }
            if let Some(recommends) = recommends {
                deps.extend(Self::to_depends(&recommends, pool).await.wrap(&cause)?);
            }
            // if let Some(suggests) = _suggests {
            //     deps.extend(Self::to_depends(&suggests)?);
            // }
            DependKind::collapse(deps).context(OtherSnafu{error: "Dependency conflict! The developer wishes you 'Good Luck' on your quest to figure out which dependency it is."}).wrap(&cause)?
        };
        Ok(ProcessedMetaData {
            name: package,
            kind: MetaDataKind::Apt,
            description,
            version: version.to_string(),
            origin: source.clone(),
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
            "amd64" => Arch::X86_64,
            "arm64" => Arch::Aarch64,
            _ => Arch::NoArch,
        }
    }
    async fn to_depends(
        relations: &Relations,
        pool: &SqlitePool,
    ) -> Result<HashSet<DependKind>, StatefulError> {
        let cause = json!({"action": "converting APT relations to DependKinds"});
        let mut depends = HashSet::new();
        for versions in relations.to_string().split(",") {
            let mut choices = HashSet::new();
            for version in versions.split("|") {
                let (version, arch) = version.split_once(":").unwrap_or((version, "any"));
                let arch = Self::get_arch(arch);
                if !arch.is_compatible(version).await.wrap(&cause)? {
                    return Err(StatefulError::new(
                        "The architecture of this package is incompatible with your hardware.",
                        &cause,
                    ));
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
                        return Err(StatefulError::new(
                            format!("Version \"{}\" is not a valid Version!", ver),
                            &cause,
                        ));
                    };
                    match op {
                        ">>" => prior = VerReq::Gt(ver).negotiate(prior),
                        ">=" => prior = VerReq::Ge(ver).negotiate(prior),
                        "=" => prior = VerReq::Eq(ver).negotiate(prior),
                        "<<" => prior = VerReq::Lt(ver).negotiate(prior),
                        "<=" => prior = VerReq::Le(ver).negotiate(prior),
                        _ => {
                            return Err(StatefulError::new(
                                format!("`{}` is not a valid Version opcode!", op),
                                &cause,
                            ));
                        }
                    }
                    let range = prior
                        .context(OtherSnafu {
                            error: "No mutually agreeable version found!",
                        })
                        .wrap(&cause)?;
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
