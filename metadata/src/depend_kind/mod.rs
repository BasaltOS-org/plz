use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use settings::OriginKind;
use snafu::OptionExt;
use utils::{
    Range, VerReq, Version, command,
    errors::{SystemSnafu, WhereError},
};

use crate::{
    DepVer, FuckNest, FuckWrap, InstallPackage, Specific, get_metadata_path,
    processed::ProcessedMetaData,
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum DependKind {
    Latest(String),
    Specific(DepVer),
    Volatile(String),
}

impl DependKind {
    pub fn as_dep_ver(&self) -> Option<DepVer> {
        match self {
            Self::Latest(latest) => {
                // let version = VerReq::Eq(Version::parse(&get_latest(latest).await.ok()?).ok()?);
                // Maybe set a `lower` to VerReq::Ge(currently_installed_version); `upper` to VerReq::NoBound
                Some(DepVer {
                    name: latest.to_string(),
                    range: Range {
                        lower: VerReq::NoBound,
                        upper: VerReq::NoBound,
                    },
                })
            }
            Self::Specific(specific) => Some(specific.clone()),
            Self::Volatile(volatile) => {
                let result = command("/usr/bin/which", &[volatile], None);
                if result.is_some_and(|x| x == 0) {
                    None
                } else {
                    Some(DepVer {
                        name: volatile.to_string(),
                        range: Range {
                            lower: VerReq::NoBound,
                            upper: VerReq::NoBound,
                        },
                    })
                }
            }
        }
    }
    pub async fn batch_as_installed(
        deps: &[Self],
        sources: &[OriginKind],
        prior: &mut HashSet<Specific>,
    ) -> Result<Vec<InstallPackage>, WhereError> {
        let mut result = Vec::new();
        for dep in deps {
            let dep = match dep {
                Self::Latest(latest) => {
                    ProcessedMetaData::get_metadata(latest, None, sources, true)
                        .await
                        .context(SystemSnafu {
                            message: "Discovery failed",
                            package: latest.to_string(),
                        })
                        .wrap()?
                }
                Self::Specific(dep_ver) => {
                    let specific = dep_ver
                        .clone()
                        .pull_metadata(Some(sources), true)
                        .await
                        .nest("Locate Package Metadata")?;
                    ProcessedMetaData::get_metadata(
                        &specific.name,
                        Some(&specific.version.to_string()),
                        sources,
                        true,
                    )
                    .await
                    .context(SystemSnafu {
                        message: format!("Failed to locate version {}", specific.version),
                        package: specific.name,
                    })
                    .wrap()?
                }
                Self::Volatile(volatile) => {
                    let result = command("/usr/bin/which", &[volatile], None);
                    if result.is_some_and(|x| x == 0) {
                        continue;
                    } else {
                        ProcessedMetaData::get_metadata(volatile, None, sources, true)
                            .await
                            .context(SystemSnafu {
                                message: "Volatile discovery failed",
                                package: volatile.to_string(),
                            })
                            .wrap()?
                    }
                }
            };
            let specific = Specific {
                name: dep.name.to_string(),
                version: Version::parse(&dep.version).wrap()?,
            };
            if !prior.contains(&specific) {
                prior.insert(specific);
                let child = Box::pin(ProcessedMetaData::get_depends(&dep, sources, prior))
                    .await
                    .nest("Get Package Dependencies")?;
                result.push(child);
            }
        }
        Ok(result)
    }
    pub fn collapse<T: IntoIterator<Item = Self>>(deps: T) -> Option<Vec<Self>> {
        let mut collapsed: Vec<Self> = Vec::new();
        for dep in deps {
            if let Some(index) = collapsed.iter().position(|x| x.name() == dep.name()) {
                match &collapsed[index] {
                    Self::Volatile(_) => collapsed[index] = dep,
                    Self::Latest(_) => {
                        if let Self::Specific(_) = dep {
                            collapsed[index] = dep;
                        }
                    }
                    Self::Specific(entry_specific) => {
                        if let Self::Specific(dep_specific) = dep {
                            let entry_range = entry_specific.range.clone();
                            let dep_range = dep_specific.range;
                            let range = dep_range.negotiate(Some(entry_range))?;
                            collapsed[index] = Self::Specific(DepVer {
                                name: dep_specific.name,
                                range,
                            })
                        }
                    }
                }
            } else {
                collapsed.push(dep);
            }
        }
        Some(collapsed)
    }
    pub fn choose<T: IntoIterator<Item = Self>>(choices: T) -> Option<Self> {
        let mut first = None;
        for choice in choices {
            if choice.is_installed() {
                return None;
            } else if first.is_none() {
                first = Some(choice);
            }
        }
        first
    }
    fn is_installed(&self) -> bool {
        match self {
            Self::Latest(latest) => match get_metadata_path(latest) {
                Ok(data) => data.1.is_some(),
                Err(_) => false,
            },
            Self::Specific(specific) => match get_metadata_path(&specific.name) {
                Ok(data) => {
                    if let Some(data) = data.1 {
                        let prior = Version::parse(&data.version).ok().map(|x| {
                            let ver_req = VerReq::Eq(x);
                            Range {
                                upper: ver_req.clone(),
                                lower: ver_req,
                            }
                        });
                        specific.range.negotiate(prior).is_some()
                    } else {
                        false
                    }
                }
                Err(_) => false,
            },
            Self::Volatile(volatile) => {
                let result = command("/usr/bin/which", &[volatile], None);
                if result.is_some_and(|x| x == 0) {
                    true
                } else {
                    match get_metadata_path(volatile) {
                        Ok(value) => value.1.is_some(),
                        Err(_) => false,
                    }
                }
            }
        }
    }
    pub fn name(&self) -> String {
        match self {
            Self::Latest(latest) => latest.to_string(),
            Self::Specific(specific) => specific.name.to_string(),
            Self::Volatile(volatile) => volatile.to_string(),
        }
    }
}
