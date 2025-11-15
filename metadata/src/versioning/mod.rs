use std::{
    fs::{self, File},
    io::Write,
    process::Command,
};

use serde::{Deserialize, Serialize};
use settings::{OriginKind, SettingsYaml};
use snafu::{OptionExt, ResultExt, location};
use utils::{
    Range, VerReq, Version,
    errors::{HowError, IOAction, IOSnafu, SystemSnafu, WhereError, YAMLSnafu},
};

use crate::{
    FuckNest, FuckWrap, QueuedChanges, get_metadata_path,
    installed::{InstalledInstallKind, InstalledMetaData},
    processed::ProcessedMetaData,
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DepVer {
    pub name: String,
    pub range: Range,
}

impl DepVer {
    pub fn get_installed_specific(&self) -> Result<Specific, WhereError> {
        let metadata = InstalledMetaData::open(&self.name).nest("Locate Package Metadata")?;
        Ok(Specific {
            name: metadata.name,
            version: Version::parse(&metadata.version).wrap()?,
        })
    }
    pub async fn pull_metadata(
        self,
        sources: Option<&[OriginKind]>,
        dependent: bool,
    ) -> Result<ProcessedMetaData, WhereError> {
        let sources = match sources {
            Some(sources) => sources,
            None => &SettingsYaml::get_settings().wrap()?.sources,
        };
        let mut versions = None;
        let mut g_source = None;
        let name = self.name;
        for source in sources {
            match source {
                OriginKind::Pax(pax) => {
                    let endpoint = format!("{pax}/package/{name}");
                    let Ok(response) = reqwest::get(endpoint).await else {
                        continue;
                    };
                    let Ok(body) = response.text().await else {
                        continue;
                    };
                    let vers = body
                        .split(',')
                        .flat_map(Version::parse)
                        .collect::<Vec<Version>>();
                    if !vers.is_empty() {
                        versions = Some(vers);
                        g_source = Some(source.clone());
                        break;
                    }
                }
                OriginKind::Github { .. } => {
                    // thingy
                    println!("Github is not implemented yet!");
                }
                OriginKind::Apt { .. } => return Err(WhereError::debug(location!())),
            }
        }
        let (Some(mut versions), Some(source)) = (versions, g_source) else {
            return Err(HowError::SystemError {
                message: "Discovery failed".into(),
                package: name.into(),
            })
            .wrap();
        };
        match &self.range.lower {
            VerReq::Gt(gt) => versions.retain(|x| x > gt),
            VerReq::Ge(ge) => versions.retain(|x| x >= ge),
            VerReq::Eq(eq) => versions.retain(|x| x == eq),
            VerReq::NoBound => (),
            fuck => {
                return Err(HowError::SystemError {
                    message: format!("Unexpected `lower` version requirement of {fuck:?}",).into(),
                    package: name.into(),
                })
                .wrap();
            }
        };
        match &self.range.upper {
            VerReq::Le(le) => versions.retain(|x| x <= le),
            VerReq::Lt(lt) => versions.retain(|x| x < lt),
            VerReq::Eq(_) | VerReq::NoBound => (),
            fuck => {
                return Err(HowError::SystemError {
                    message: format!("Unexpected `upper` version requirement of {fuck:?}",).into(),
                    package: name.into(),
                })
                .wrap();
            }
        };
        versions.sort();
        let Some(ver) = versions.last().map(|x| x.to_string()) else {
            return Err(WhereError::debug(location!()));
        };
        ProcessedMetaData::get_metadata(&name, Some(&ver), &[source], dependent)
            .await
            .context(SystemSnafu {
                message: format!("Failed to locate version {ver}"),
                package: name,
            })
            .wrap()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Specific {
    pub name: String,
    pub version: Version,
}

impl Specific {
    pub fn write_dependent(&self, their_name: &str, their_ver: &str) -> Result<(), WhereError> {
        let (path, data) = get_metadata_path(&self.name)?;
        if path.exists()
            && path.is_file()
            && let Some(mut data) = data
        {
            if data.version == self.version.to_string() {
                let their_dep = Self {
                    name: their_name.to_string(),
                    version: Version::parse(their_ver).wrap()?,
                };
                if let Some(found) = data
                    .dependents
                    .iter_mut()
                    .find(|x| x.name == their_dep.name)
                {
                    found.version = their_dep.version;
                } else if !data.dependents.contains(&their_dep)
                    && let Ok(their_metadata) = InstalledMetaData::open(their_name)
                    && their_metadata.version == their_ver
                {
                    data.dependents.push(their_dep);
                }
            }
            let mut file = File::create(&path)
                .context(IOSnafu {
                    action: IOAction::CreateFile,
                    loc: path.display().to_string(),
                })
                .wrap()?;
            let data = serde_norway::to_string(&data)
                .context(YAMLSnafu {
                    loc: data.name.to_string(),
                })
                .wrap()?;
            file.write_all(data.as_bytes())
                .context(IOSnafu {
                    action: IOAction::WriteFile,
                    loc: path.display().to_string(),
                })
                .wrap()
        } else {
            Err(HowError::SystemError {
                message: format!("Failed to find data for dependency `{}`", self.name).into(),
                package: their_name.to_string().into(),
            })
            .wrap()
        }
    }
    pub fn get_dependents(&self, queued: &mut QueuedChanges) -> Result<(), WhereError> {
        let data = InstalledMetaData::open(&self.name).nest("Locate Package Metadata")?;
        if data.version == self.version.to_string() {
            for dependent in &data.dependents {
                if queued.insert_primary(dependent.clone()) {
                    dependent
                        .get_dependents(queued)
                        .nest("Get Package Dependents")?;
                }
            }
            Ok(())
        } else {
            Err(HowError::SystemError {
                message: format!("Version {} not found", self.version).into(),
                package: self.name.to_string().into(),
            })
            .wrap()
        }
    }
    pub fn remove(&self, purge: bool) -> Result<(), WhereError> {
        let msg = if purge { "Purging" } else { "Removing" };
        println!("{} {} version {}...", msg, self.name, self.version);
        let (path, data) = get_metadata_path(&self.name)?;
        let Some(data) = data else {
            // Since packages are interlinked, chances are another package
            // has already removed this one, and therefore we are just holding
            // a stale package `Specific`!
            println!(
                "\x1B[33m[WARN] Skipping `{}`\x1B[0m (This is likely the result of cyclical dependencies)...",
                self.name
            );
            return Ok(());
        };
        for dep in &data
            .dependencies
            .iter()
            .flat_map(|x| x.get_installed_specific())
            .collect::<Vec<Specific>>()
        {
            data.clear_dependencies(dep)
                .nest("Remove Dependency from Package")?;
            dep.remove(purge).nest("Remove/Purge Package")?;
        }
        match data.install_kind {
            InstalledInstallKind::PreBuilt(_) => {
                return Err(WhereError::debug(location!())); //thingy
            }
            InstalledInstallKind::Compilable(compilable) => {
                // I'm not sure if the `purge` script is run IN PLACE OF, or
                // AFTER the `uninstall` script. This is due to change.
                let (script, msg) = if purge {
                    (compilable.purge, "Purge")
                } else {
                    (compilable.uninstall, "Removal")
                };
                let mut command = Command::new("/usr/bin/bash");
                if !command
                    .arg("-c")
                    .arg(script)
                    .status()
                    .is_ok_and(|x| x.code() == Some(0))
                {
                    return Err(HowError::SystemError {
                        message: format!("{msg} failed").into(),
                        package: self.name.to_string().into(),
                    })
                    .wrap()?;
                }
            }
        }
        fs::remove_file(&path)
            .context(IOSnafu {
                action: IOAction::RemoveFile,
                loc: path.display().to_string(),
            })
            .wrap()
    }
}
