use serde::{Deserialize, Serialize};
use settings::{Arch, OriginKind};
use snafu::{OptionExt, ResultExt, location};
use sqlx::{Decode, Encode, FromRow, Sqlite, SqlitePool, Type, query, query_as};
use std::{
    collections::HashSet, fmt::Display, hash::Hash, io::Write, process::Command as RunCommand,
};
use utils::errors::{
    HowError, IOAction, IOSnafu, NetSnafu, Parsers, SQLSnafu, SystemSnafu, WhereError,
};
use utils::{Version, tmpfile};

use crate::depend_kind::DependKindVec;
use crate::versioning::{self, SpecificVec};
use crate::{
    DepVer, DependKind, FuckNest, FuckWrap, InstallPackage, InstalledInstallKind,
    InstalledMetaData, MetaDataKind, Specific, get_installed_metadata,
    installed::InstalledCompilable, parsers::apt::RawApt, pax::RawPax,
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ProcessedInstallKind {
    PreBuilt(PreBuilt),
    Compilable(ProcessedCompilable),
}

impl ProcessedInstallKind {
    fn parse(input: &str) -> Result<Self, HowError> {
        let mut chars = input.chars();
        let kind = chars.next().ok_or(HowError::ParseError {
            message: "Missing type identifier!".into(),
            util: Parsers::ProcessedInstallKind,
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => Ok(Self::PreBuilt(PreBuilt::parse(&data)?)),
            1 => Ok(Self::Compilable(ProcessedCompilable::parse(&data)?)),
            kind => Err(HowError::ParseError {
                message: format!("Invalid kind identifier `{kind}`!").into(),
                util: Parsers::ProcessedInstallKind,
            }),
        }
    }
}

impl Display for ProcessedInstallKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::PreBuilt(prebuilt) => format!("\x00{prebuilt}"),
            Self::Compilable(compilable) => format!("\x01{compilable}"),
        })
    }
}

impl Type<Sqlite> for ProcessedInstallKind {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for ProcessedInstallKind {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <String as Encode<'_, Sqlite>>::encode_by_ref(&self.to_string(), buf)
    }
    fn encode(
        self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError>
    where
        Self: Sized,
    {
        <String as Encode<'_, Sqlite>>::encode(self.to_string(), buf)
    }
}

impl<'a> Decode<'a, Sqlite> for ProcessedInstallKind {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PreBuilt {
    pub critical: Vec<String>,
    pub configs: Vec<String>,
}

impl PreBuilt {
    pub fn parse(input: &str) -> Result<Self, HowError> {
        let (critical, configs) = input.split_once("\x00\x00").ok_or(HowError::ParseError {
            message: "Missing PreBuilt field 'configs`!".into(),
            util: Parsers::PreBuilt,
        })?;
        let critical = critical
            .split('\x00')
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        let configs = configs
            .split('\x00')
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        Ok(Self { critical, configs })
    }
}

impl Display for PreBuilt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let critical = self.critical.iter().fold(String::new(), |mut acc, x| {
            if !acc.is_empty() {
                acc.push('\x00');
            }
            acc.push_str(x);
            acc
        });
        let configs = self.configs.iter().fold(String::new(), |mut acc, x| {
            if !acc.is_empty() {
                acc.push('\x00');
            }
            acc.push_str(x);
            acc
        });
        f.write_str(&format!("{critical}\x00\x00{configs}"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ProcessedCompilable {
    pub build: String,
    pub install: String,
    pub uninstall: String,
    pub purge: String,
}

impl ProcessedCompilable {
    fn parse(input: &str) -> Result<Self, HowError> {
        let mut splits = input.split('\x00');
        let build = splits.next().ok_or(HowError::ParseError {
            message: "Missing ProcessedCompilable field `build`!".into(),
            util: Parsers::ProcessedCompilable,
        })?;
        let install = splits.next().ok_or(HowError::ParseError {
            message: "Missing ProcessedCompilable field `install`!".into(),
            util: Parsers::ProcessedCompilable,
        })?;
        let uninstall = splits.next().ok_or(HowError::ParseError {
            message: "Missing ProcessedCompilable field `uninstall`!".into(),
            util: Parsers::ProcessedCompilable,
        })?;
        let purge = splits.next().ok_or(HowError::ParseError {
            message: "Missing ProcessedCompilable field `purge`!".into(),
            util: Parsers::ProcessedCompilable,
        })?;
        Ok(Self {
            build: build.to_string(),
            install: install.to_string(),
            uninstall: uninstall.to_string(),
            purge: purge.to_string(),
        })
    }
}

impl Display for ProcessedCompilable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "{}\x00{}\x00{}\x00{}",
            self.build, self.install, self.uninstall, self.purge
        ))
    }
}

#[derive(Clone, Debug, Encode, Eq, FromRow, Hash, PartialEq)]
pub struct ProcessedMetaData {
    pub name: String,
    pub kind: MetaDataKind,
    pub description: String,
    pub version: String,
    pub origin: OriginKind,
    pub dependent: bool,
    pub build_dependencies: DependKindVec,
    pub runtime_dependencies: DependKindVec,
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
                for dep in &self.runtime_dependencies.0 {
                    if let Some(dep) = dep.as_dep_ver() {
                        result.push(dep);
                    }
                }
                versioning::DepVerVec(result)
            },
            dependents: SpecificVec(Vec::new()),
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
    pub async fn install_package(self, pool: &SqlitePool) -> Result<(), WhereError> {
        let name = self.name.to_string();
        println!("Installing {name}...");
        let mut metadata = self.to_installed();
        let deps = metadata.dependencies.clone();
        let ver = metadata.version.to_string();
        for dependent in metadata.dependents.0.iter_mut() {
            let their_metadata = InstalledMetaData::open(&dependent.name, pool)
                .await
                .nest("Locate Package Metadata")?
                .context(SystemSnafu {
                    message: "Discovery failed",
                    package: dependent.name.to_string(),
                })
                .wrap()?;
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
        let mut file = std::fs::File::create(&tmpfile.0)
            .context(IOSnafu {
                action: IOAction::CreateFile,
                loc: tmpfile.0.display().to_string(),
            })
            .wrap()?;
        let endpoint = match self.origin {
            OriginKind::Pax(pax) => format!("{pax}?v={}", self.version),
            OriginKind::Github { .. } => {
                return Err(WhereError::debug(location!()));
                // thingy
            }
            OriginKind::Apt { .. } => return Err(WhereError::debug(location!())),
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
            .write(pool)
            .await
            .nest("Write Changes to Package Metadata")?;
        for dep in deps.0 {
            let dep = dep
                .get_installed_specific(pool)
                .await
                .nest("Convert to Installed `Specific`")?;
            dep.write_dependent(&name, &ver, pool)
                .await
                .nest("Add Dependent to Dependency Metadata")?;
        }
        Ok(())
    }
    pub async fn write(self, pool: &SqlitePool) -> Result<Self, WhereError> {
        // let path = loop {
        //     // let mut path = base.to_path_buf();
        //     path.push(format!("{inc}.json"));
        //     if path.exists() {
        //         *inc += 1;
        //         continue;
        //     }
        //     break path;
        // };
        // let mut file = File::create(&path)
        //     .context(IOSnafu {
        //         action: IOAction::CreateFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()?;
        // query_as!(Self, "INSERT INTO installed VALUES ?", &self)
        //     .execute(&pool)
        //     .await?;
        query("INSERT INTO updates VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&self.name)
            .bind(&self.kind)
            .bind(&self.description)
            .bind(&self.version)
            .bind(&self.origin)
            .bind(self.dependent)
            .bind(&self.build_dependencies)
            .bind(&self.runtime_dependencies)
            .bind(&self.install_kind)
            .bind(&self.hash)
            .execute(pool)
            .await
            .context(SQLSnafu)
            .wrap()?;
        // let data = serde_json::to_string(&self)
        //     .context(JSONSnafu {
        //         loc: self.name.to_string(),
        //     })
        //     .wrap()?;
        // file.write_all(data.as_bytes())
        //     .context(IOSnafu {
        //         action: IOAction::WriteFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()?;
        Ok(self)
    }
    pub async fn open(name: &str, pool: &SqlitePool) -> Result<Self, WhereError> {
        // let mut path = get_update_dir().wrap()?;
        // path.push(format!("{}.json", name));
        // let mut file = File::open(&path)
        //     .context(IOSnafu {
        //         action: IOAction::OpenFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()?;
        // let mut metadata = String::new();
        // file.read_to_string(&mut metadata)
        //     .context(IOSnafu {
        //         action: IOAction::ReadFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()?;
        // serde_json::from_str::<Self>(&metadata)
        //     .context(JSONSnafu {
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()
        query_as::<Sqlite, ProcessedMetaData>("SELECT * FROM updates WHERE name = ?")
            .bind(name)
            .fetch_one(pool)
            .await
            .context(SQLSnafu)
            .wrap()
    }
    pub async fn get_metadata(
        name: &str,
        version: Option<&str>,
        sources: &[OriginKind],
        dependent: bool,
        pool: &SqlitePool,
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
                OriginKind::Github { .. } => {
                    // thingy
                    println!("Github is not implemented yet!");
                }
                OriginKind::Apt { source, code, kind } => {
                    let vers = RawApt::get_vers(source, code, &kind.to_string(), None, name).await;
                    let Some(ver) = (if let Some(version) = version {
                        vers.into_iter().find(|x| x.1.to_string() == version)
                    } else {
                        let mut vers = vers.into_iter().collect::<Vec<(String, Version, Arch)>>();
                        vers.sort_by(|a, b| a.1.cmp(&b.1));
                        vers.into_iter().next_back()
                    }) else {
                        continue;
                    };
                    let processed = match RawApt::parse(
                        source, code, kind, name, &ver.0, dependent, pool,
                    )
                    .await
                    {
                        Ok(data) => data,
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
    pub async fn remove_update_cache(&self, pool: &SqlitePool) -> Result<(), WhereError> {
        // let path = get_update_dir().wrap()?;
        // let dir = fs::read_dir(&path)
        //     .context(IOSnafu {
        //         action: IOAction::ReadDir,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()?;
        // for file in dir.flatten() {
        //     let path = file.path();
        //     if let Some(name) = path.file_prefix() {
        //         let name = name.to_string_lossy();
        //         let data = Self::open(&name, pool).await?;
        //         if data.name == self.name {
        //             return fs::remove_file(&path)
        //                 .context(IOSnafu {
        //                     action: IOAction::RemoveFile,
        //                     loc: path.display().to_string(),
        //                 })
        //                 .wrap();
        //         }
        //     }
        // }
        query("DELETE FROM updates WHERE name = ?")
            .bind(&self.name)
            .execute(pool)
            .await
            .context(SQLSnafu)
            .wrap()?;
        // println!(
        //     "\x1B[33m[WARN] cache for {} already cleared!\x1B[0m",
        //     self.name
        // );
        Ok(())
    }
    pub async fn get_depends(
        metadata: &Self,
        sources: &[OriginKind],
        prior: &mut HashSet<Specific>,
        pool: &SqlitePool,
    ) -> Result<InstallPackage, WhereError> {
        let mut package = InstallPackage {
            metadata: metadata.clone(),
            build_deps: Vec::new(),
            run_deps: Vec::new(),
        };
        package.build_deps =
            DependKind::batch_as_installed(&metadata.build_dependencies, sources, prior, pool)
                .await
                .nest("Batch Convert to InstalledMetadata")?;
        package.run_deps =
            DependKind::batch_as_installed(&metadata.runtime_dependencies, sources, prior, pool)
                .await
                .nest("Batch Convert to InstalledMetadata")?;
        Ok(package)
    }
    pub async fn upgrade_package(
        &self,
        sources: &[OriginKind],
        pool: &SqlitePool,
    ) -> Result<(), WhereError> {
        let version = Version::parse(&self.version).wrap()?;
        let specific = self.as_specific()?;
        let Ok(Some(installed)) = InstalledMetaData::open(&self.name, pool).await else {
            println!(
                "\x1B[33m[WARN] Skipping `{}`\x1B[0m (This is likely the result of a stale cache)...",
                self.name
            );
            return Ok(());
        };
        let children = self
            .build_dependencies
            .0
            .clone()
            .into_iter()
            .flat_map(|x| x.as_dep_ver())
            .map(|x| x.pull_metadata(Some(sources), true, pool));
        let mut stale_installed = installed
            .dependencies
            .0
            .iter()
            .filter(|x| {
                !self
                    .runtime_dependencies
                    .0
                    .iter()
                    .any(|y| y.as_dep_ver().as_ref() == Some(*x))
            })
            .collect::<Vec<&DepVer>>();
        let mut new_deps = self
            .runtime_dependencies
            .0
            .iter()
            .filter(|x| {
                !installed
                    .dependencies
                    .0
                    .iter()
                    .any(|y| Some(y) == x.as_dep_ver().as_ref())
            })
            .collect::<Vec<&DependKind>>();
        let in_place_upgrade = new_deps
            .extract_if(.., |x| stale_installed.iter().any(|y| y.name == x.name()))
            .collect::<Vec<&DependKind>>();
        stale_installed.retain(|x| !in_place_upgrade.iter().any(|y| y.name() == x.name));
        let children = {
            let mut s_children = Vec::new();
            for child in children {
                s_children.push(child.await.nest("Pull Package Metadata")?);
            }
            s_children
        };
        for child in children.into_iter() {
            child.install_package(pool).await.nest("Install Package")?;
        }
        for stale in stale_installed {
            stale
                .get_installed_specific(pool)
                .await
                .nest("Convert to Installed `Specific`")?
                .remove(false, Some(pool))
                .await
                .nest("Remove/Purge Package")?;
        }
        for dep in new_deps {
            if let Some(dep_ver) = dep.as_dep_ver() {
                let installed_metadata = InstalledMetaData::open(&dep_ver.name, pool)
                    .await
                    .nest("Locate Package Metadata")?
                    .context(SystemSnafu {
                        message: "Discovery failed",
                        package: dep_ver.name.to_string(),
                    })
                    .wrap()?;
                let metadata = dep_ver
                    .pull_metadata(Some(sources), installed_metadata.dependent, pool)
                    .await
                    .nest("Pull Package Metadata")?;
                metadata
                    .install_package(pool)
                    .await
                    .nest("Install Package")?;
            }
        }
        for package in in_place_upgrade {
            if let Some(dep_ver) = package.as_dep_ver() {
                let name = dep_ver.name.to_string();
                let metadata = get_installed_metadata(&name, pool).await?;
                let old_metadata = metadata
                    .context(SystemSnafu {
                        message: "Cannot find data",
                        package: name.to_string(),
                    })
                    .wrap()?;
                let metadata = dep_ver
                    .pull_metadata(Some(sources), old_metadata.dependent, pool)
                    .await
                    .nest("Pull Package Metadata")?;
                if metadata.version != old_metadata.version {
                    metadata
                        .install_package(pool)
                        .await
                        .nest("Install Package")?;
                }
                let mut metadata = InstalledMetaData::open(&name, pool)
                    .await
                    .nest("Locate Package Metadata")?
                    .context(SystemSnafu {
                        message: "Discovery failed",
                        package: name.to_string(),
                    })
                    .wrap()?;
                if let Some(found) = metadata
                    .dependents
                    .0
                    .iter_mut()
                    .find(|x| x.name == self.name)
                {
                    found.version = version.clone();
                } else {
                    metadata.dependents.0.push(specific.clone());
                };
                metadata
                    .write(pool)
                    .await
                    .nest("Write Changes to Package Metadata")?;
            }
        }
        self.clone()
            .install_package(pool)
            .await
            .nest("Install Package")?;
        Ok(())
    }
    pub fn as_specific(&self) -> Result<Specific, WhereError> {
        Ok(Specific {
            name: self.name.to_string(),
            version: Version::parse(&self.version).wrap()?,
        })
    }
}
