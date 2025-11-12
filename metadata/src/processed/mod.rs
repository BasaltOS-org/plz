use serde::{Deserialize, Serialize};
use settings::{Arch, OriginKind};
use snafu::{OptionExt, ResultExt, location};
use std::hash::Hash;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process::Command as RunCommand,
};
use tokio::runtime::Runtime;
use utils::errors::{IOAction, IOSnafu, NetSnafu, SystemSnafu, WhereError, YAMLSnafu};
use utils::{Version, get_update_dir, tmpfile};

use crate::{
    DepVer, DependKind, FuckNest, FuckWrap, InstallPackage, InstalledInstallKind,
    InstalledMetaData, MetaDataKind, Specific, get_metadata_path, installed::InstalledCompilable,
    parsers::apt::RawApt, pax::RawPax,
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ProcessedInstallKind {
    PreBuilt(PreBuilt),
    Compilable(ProcessedCompilable),
}
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PreBuilt {
    pub critical: Vec<String>,
    pub configs: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ProcessedCompilable {
    pub build: String,
    pub install: String,
    pub uninstall: String,
    pub purge: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ProcessedMetaData {
    pub name: String,
    pub kind: MetaDataKind,
    pub description: String,
    pub version: String,
    pub origin: OriginKind,
    pub dependent: bool,
    pub build_dependencies: Vec<DependKind>,
    pub runtime_dependencies: Vec<DependKind>,
    pub install_kind: ProcessedInstallKind,
    pub hash: String,
}

impl ProcessedMetaData {
    pub fn to_installed(&self) -> InstalledMetaData {
        InstalledMetaData {
            name: self.name.clone(),
            kind: self.kind.clone(),
            version: self.version.to_string(),
            origin: self.origin.clone(),
            dependent: self.dependent,
            dependencies: {
                let mut result = Vec::new();
                for dep in &self.runtime_dependencies {
                    if let Some(dep) = dep.as_dep_ver() {
                        result.push(dep);
                    }
                }
                result
            },
            dependents: Vec::new(),
            install_kind: match &self.install_kind {
                ProcessedInstallKind::PreBuilt(prebuilt) => {
                    InstalledInstallKind::PreBuilt(prebuilt.clone())
                }
                ProcessedInstallKind::Compilable(comp) => {
                    InstalledInstallKind::Compilable(InstalledCompilable {
                        uninstall: comp.uninstall.clone(),
                        purge: comp.purge.clone(),
                    })
                }
            },
            hash: self.hash.to_string(),
        }
    }
    pub async fn install_package(self) -> Result<(), WhereError> {
        let name = self.name.to_string();
        println!("Installing {name}...");
        let (path, _) = get_metadata_path(&name)?;
        let mut metadata = self.to_installed();
        let deps = metadata.dependencies.clone();
        let ver = metadata.version.to_string();
        for dependent in metadata.dependents.iter_mut() {
            let their_metadata =
                InstalledMetaData::open(&dependent.name).nest("Locate Package Metadata")?;
            *dependent = Specific {
                name: dependent.name.to_string(),
                version: Version::parse(&their_metadata.version).wrap()?,
            }
        }
        let tmpfile = tmpfile()
            .context(SystemSnafu {
                message: "Failed to reserve a file",
                package: name.to_string(),
            })
            .wrap()?;
        let mut file = File::create(&tmpfile.0)
            .context(IOSnafu {
                action: IOAction::CreateFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        let endpoint = match self.origin {
            OriginKind::Pax(pax) => format!("{pax}?v={}", self.version),
            OriginKind::Github { user: _, repo: _ } => {
                return Err(WhereError::debug(location!()));
                // thingy
            }
            OriginKind::Apt(_) => return Err(WhereError::debug(location!())),
        };
        let response = reqwest::get(&endpoint)
            .await
            .context(NetSnafu {
                loc: endpoint.to_string(),
            })
            .wrap()?;
        let body = response
            .text()
            .await
            .context(NetSnafu {
                loc: endpoint.to_string(),
            })
            .wrap()?;
        file.write_all(body.as_bytes())
            .context(IOSnafu {
                action: IOAction::WriteFile,
                loc: tmpfile.0.display().to_string(),
            })
            .wrap()?;
        match self.install_kind {
            ProcessedInstallKind::PreBuilt(_) => {
                return Err(WhereError::debug(location!())); //thingy
            }
            ProcessedInstallKind::Compilable(compilable) => {
                let build = compilable.build.replace("{$~}", &tmpfile.1);
                let mut command = RunCommand::new("/usr/bin/bash");
                command
                    .arg("-c")
                    .arg(build)
                    .status()
                    .context(IOSnafu {
                        action: IOAction::TermStatus,
                        loc: "Build Package Script",
                    })
                    .wrap()?;
                let install = compilable.install.replace("{$~}", &tmpfile.1);
                let mut command = RunCommand::new("/usr/bin/bash");
                command
                    .arg("-c")
                    .arg(install)
                    .status()
                    .context(IOSnafu {
                        action: IOAction::TermStatus,
                        loc: "Install Package Script",
                    })
                    .wrap()?;
            }
        }
        metadata
            .write(&path)
            .nest("Write Changes to Package Metadata")?;
        for dep in deps {
            let dep = dep
                .get_installed_specific()
                .nest("Convert to Installed `Specific`")?;
            dep.write_dependent(&name, &ver)
                .nest("Add Dependent to Dependency Metadata")?;
        }
        Ok(())
    }
    pub fn write(self, base: &Path, inc: &mut usize) -> Result<Self, WhereError> {
        let path = loop {
            let mut path = base.to_path_buf();
            path.push(format!("{inc}.yaml"));
            if path.exists() {
                *inc += 1;
                continue;
            }
            break path;
        };
        let mut file = File::create(&path)
            .context(IOSnafu {
                action: IOAction::CreateFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        let data = serde_norway::to_string(&self)
            .context(YAMLSnafu {
                loc: self.name.to_string(),
            })
            .wrap()?;
        file.write_all(data.as_bytes())
            .context(IOSnafu {
                action: IOAction::WriteFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        Ok(self)
    }
    pub fn open(name: &str) -> Result<Self, WhereError> {
        let mut path = get_update_dir().wrap()?;
        path.push(format!("{}.yaml", name));
        let mut file = File::open(&path)
            .context(IOSnafu {
                action: IOAction::OpenFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        let mut metadata = String::new();
        file.read_to_string(&mut metadata)
            .context(IOSnafu {
                action: IOAction::ReadFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        serde_norway::from_str::<Self>(&metadata)
            .context(YAMLSnafu {
                loc: path.display().to_string(),
            })
            .wrap()
    }
    pub async fn get_metadata(
        name: &str,
        version: Option<&str>,
        sources: &[OriginKind],
        dependent: bool,
    ) -> Option<Self> {
        let mut metadata = None;
        for source in sources {
            match source {
                OriginKind::Pax(source) => {
                    // metadata = {
                    let endpoint = if let Some(version) = version {
                        format!("{source}/packages/metadata/{name}?v={version}")
                    } else {
                        format!("{source}/packages/metadata/{name}")
                    };
                    let body = reqwest::get(endpoint).await.ok()?.text().await.ok()?;
                    if let Ok(raw_pax) = serde_json::from_str::<RawPax>(&body) {
                        metadata = raw_pax.to_process(dependent);
                        break;
                    }
                    //     && let Some(processed) = raw_pax.process()
                    // {
                    //     Some(processed)
                    // } else {
                    //     None
                    // }
                    // };
                }
                OriginKind::Github { user: _, repo: _ } => {
                    // thingy
                    println!("Github is not implemented yet!");
                }
                OriginKind::Apt(apt) => {
                    let vers = RawApt::get_vers(apt, None, name).await;
                    let Some(ver) = (if let Some(version) = version {
                        vers.into_iter().find(|x| x.1.to_string() == version)
                    } else {
                        let mut vers = vers.into_iter().collect::<Vec<(String, Version, Arch)>>();
                        vers.sort_by(|a, b| a.1.cmp(&b.1));
                        vers.into_iter().next_back()
                    }) else {
                        continue;
                    };
                    let processed = match RawApt::parse(apt, name, &ver.0, dependent).await {
                        Ok(data) => dbg!(data),
                        Err(fault) => {
                            println!("{fault}");
                            return None;
                        }
                    };
                    metadata = Some(processed);
                    break;
                }
            }
        }
        metadata
    }
    pub fn remove_update_cache(&self) -> Result<(), WhereError> {
        let path = get_update_dir().wrap()?;
        let dir = fs::read_dir(&path)
            .context(IOSnafu {
                action: IOAction::ReadDir,
                loc: path.display().to_string(),
            })
            .wrap()?;
        for file in dir.flatten() {
            let path = file.path();
            if let Some(name) = path.file_prefix() {
                let name = name.to_string_lossy();
                let data = Self::open(&name)?;
                if data.name == self.name {
                    return fs::remove_file(&path)
                        .context(IOSnafu {
                            action: IOAction::RemoveFile,
                            loc: path.display().to_string(),
                        })
                        .wrap();
                }
            }
        }
        println!(
            "\x1B[33m[WARN] cache for {} already cleared!\x1B[0m",
            self.name
        );
        Ok(())
    }
    pub async fn get_depends(
        metadata: &Self,
        sources: &[OriginKind],
        prior: &mut HashSet<Specific>,
    ) -> Result<InstallPackage, WhereError> {
        let mut package = InstallPackage {
            metadata: metadata.clone(),
            build_deps: Vec::new(),
            run_deps: Vec::new(),
        };
        package.build_deps =
            DependKind::batch_as_installed(&metadata.build_dependencies, sources, prior)
                .await
                .nest("Batch Convert to InstalledMetadata")?;
        package.run_deps =
            DependKind::batch_as_installed(&metadata.runtime_dependencies, sources, prior)
                .await
                .nest("Batch Convert to InstalledMetadata")?;
        Ok(package)
    }
    pub fn upgrade_package(
        &self,
        sources: &[OriginKind],
        runtime: &Runtime,
    ) -> Result<(), WhereError> {
        let version = Version::parse(&self.version).wrap()?;
        let specific = self.as_specific()?;
        let Ok(installed) = InstalledMetaData::open(&self.name) else {
            println!(
                "\x1B[33m[WARN] Skipping `{}`\x1B[0m (This is likely the result of a stale cache)...",
                self.name
            );
            return Ok(());
        };
        let children: Vec<_> = self
            .build_dependencies
            .clone()
            .into_iter()
            .flat_map(|x| x.as_dep_ver())
            .map(|x| x.pull_metadata(Some(sources), true))
            .collect();
        let mut stale_installed = installed
            .dependencies
            .iter()
            .filter(|x| {
                !self
                    .runtime_dependencies
                    .iter()
                    .any(|y| y.as_dep_ver().as_ref() == Some(*x))
            })
            .collect::<Vec<&DepVer>>();
        let mut new_deps = self
            .runtime_dependencies
            .iter()
            .filter(|x| {
                !installed
                    .dependencies
                    .iter()
                    .any(|y| Some(y) == x.as_dep_ver().as_ref())
            })
            .collect::<Vec<&DependKind>>();
        let in_place_upgrade = new_deps
            .extract_if(.., |x| stale_installed.iter().any(|y| y.name == x.name()))
            .collect::<Vec<&DependKind>>();
        stale_installed.retain(|x| !in_place_upgrade.iter().any(|y| y.name() == x.name));
        let children = children
            .into_iter()
            .map(|x| runtime.block_on(x).nest("Pull Package Metadata"))
            .collect::<Result<Vec<ProcessedMetaData>, WhereError>>()?;
        children
            .into_iter()
            .try_for_each(|x| match runtime.block_on(x.install_package()) {
                Ok(_path) => Ok(()),
                Err(fault) => Err(fault).nest("Install Package"),
            })?;
        for stale in stale_installed {
            stale
                .get_installed_specific()?
                .remove(false)
                .nest("Remove/Purge Package")?;
        }
        for dep in new_deps {
            if let Some(dep_ver) = dep.as_dep_ver() {
                let installed_metadata =
                    InstalledMetaData::open(&dep_ver.name).nest("Locate Package Metadata")?;
                let metadata = runtime
                    .block_on(dep_ver.pull_metadata(Some(sources), installed_metadata.dependent))
                    .nest("Locate Package Metadata")?;
                runtime
                    .block_on(metadata.install_package())
                    .nest("Install Package")?;
            }
        }
        for package in in_place_upgrade {
            if let Some(dep_ver) = package.as_dep_ver() {
                let name = dep_ver.name.to_string();
                let (path, metadata) = get_metadata_path(&name)?;
                let old_metadata = metadata
                    .context(SystemSnafu {
                        message: "Cannot find data",
                        package: name.to_string(),
                    })
                    .wrap()?;
                let metadata = runtime
                    .block_on(dep_ver.pull_metadata(Some(sources), old_metadata.dependent))
                    .nest("Locate Package Metadata")?;
                if metadata.version != old_metadata.version {
                    runtime
                        .block_on(metadata.install_package())
                        .nest("Install Package")?;
                }
                let mut metadata =
                    InstalledMetaData::open(&name).nest("Locate Package Metadata")?;
                if let Some(found) = metadata.dependents.iter_mut().find(|x| x.name == self.name) {
                    found.version = version.clone();
                } else {
                    metadata.dependents.push(specific.clone());
                };
                metadata
                    .write(&path)
                    .nest("Write Changes to Package Metadata")?;
            }
        }
        runtime
            .block_on(self.clone().install_package())
            .nest("Install Package")?;
        Ok(())
    }
    pub fn as_specific(&self) -> Result<Specific, WhereError> {
        Ok(Specific {
            name: self.name.to_string(),
            version: dbg!(Version::parse(&self.version)).wrap()?,
        })
    }
}
