use serde::{Deserialize, Serialize};
use serde_json::json;
use snafu::{OptionExt, ResultExt};
use sqlx::{Decode, Encode, FromRow, Sqlite, SqlitePool, Type, query, query_as};
use std::{
    collections::HashSet,
    fmt::{self, Display, Formatter},
    hash::Hash,
};
use tokio::{fs::File, io::AsyncWriteExt, process::Command as RunCommand};

use crate::errors::{NetSnafu, OtherSnafu, SQLSnafu, StatefulError, TokioIOSnafu, Wrapped};
use crate::metadata::{
    DepVer, DependKind, InstallPackage, InstalledMetaData, MetaDataKind, Specific,
    depend_kind::DependKindVec,
    installed::{InstalledCompilable, InstalledInstallKind},
    parsers::{apt::RawApt, plz::RawPlz},
    versioning::{self, SpecificVec},
};
use crate::utils::{tmpfile, version::Version};

use crate::settings::{Arch, OriginKind, SettingsJson};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ProcessedInstallKind {
    PreBuilt(PreBuilt),
    Compilable(ProcessedCompilable),
}

impl ProcessedInstallKind {
    fn parse(input: &str) -> Result<Self, StatefulError> {
        let cause = json!({"action": "parsing package data"});
        let mut chars = input.chars();
        let kind = chars
            .next()
            .context(OtherSnafu {
                error: "Missing type identifier!",
            })
            .wrap(&cause)?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => Ok(Self::PreBuilt(PreBuilt::parse(&data).wrap(&cause)?)),
            1 => Ok(Self::Compilable(
                ProcessedCompilable::parse(&data).wrap(&cause)?,
            )),
            kind => Err(StatefulError::new(
                format!("Invalid kind identifier `{kind}`!"),
                &cause,
            )),
        }
    }
}

impl Display for ProcessedInstallKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
        Ok(Self::parse(&data)
            .wrap(&json!({"action": "decoding cached package type"}))
            .map_err(|e| {
                OtherSnafu {
                    error: e.to_string(),
                }
                .build()
            })?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PreBuilt {
    pub critical: Vec<String>,
    pub configs: Vec<String>,
}

impl PreBuilt {
    pub fn parse(input: &str) -> Result<Self, StatefulError> {
        let (critical, configs) = input
            .split_once("\x00\x00")
            .context(OtherSnafu {
                error: "Missing PreBuilt field 'configs`!",
            })
            .wrap(&json!({"action": "parsing PreBuilt packages from bytes"}))?;
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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
    fn parse(input: &str) -> Result<Self, StatefulError> {
        let cause = json!({"action": "parsing compilable processable metadata from bytes"});
        let mut splits = input.split('\x00');
        let build = splits
            .next()
            .context(OtherSnafu {
                error: "Missing ProcessedCompilable field `build`!",
            })
            .wrap(&cause)?;
        let install = splits
            .next()
            .context(OtherSnafu {
                error: "Missing ProcessedCompilable field `install`!",
            })
            .wrap(&cause)?;
        let uninstall = splits
            .next()
            .context(OtherSnafu {
                error: "Missing ProcessedCompilable field `uninstall`!",
            })
            .wrap(&cause)?;
        let purge = splits
            .next()
            .context(OtherSnafu {
                error: "Missing ProcessedCompilable field `purge`!",
            })
            .wrap(&cause)?;
        Ok(Self {
            build: build.to_string(),
            install: install.to_string(),
            uninstall: uninstall.to_string(),
            purge: purge.to_string(),
        })
    }
}

impl Display for ProcessedCompilable {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
    pub async fn install_package(self, pool: &SqlitePool) -> Result<(), StatefulError> {
        let cause = json!({"action": "installing package", "package": self.name});
        let name = self.name.to_string();
        println!("Installing `{name}`...");
        let mut metadata = self.to_installed();
        let deps = metadata.dependencies.clone();
        let ver = metadata.version.to_string();
        for dependent in metadata.dependents.0.iter_mut() {
            let their_metadata = InstalledMetaData::open(&dependent.name, pool)
                .await
                .wrap(&cause)?
                .context(OtherSnafu {
                    error: format!("Failed to locate `{}`!", self.name),
                })
                .wrap(&cause)?;
            *dependent = Specific {
                name: dependent.name.to_string(),
                version: Version::parse(&their_metadata.version).wrap(&cause)?,
            }
        }
        let tmpfile = tmpfile().await.wrap(&cause)?;
        let mut file = File::create(&tmpfile.0)
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        let endpoint = match self.origin {
            OriginKind::Plz { source } => format!("{source}?v={}", self.version),
            OriginKind::Github { .. } => {
                return Err(StatefulError::new("debug breakpoint", &cause));
                // thingy
            }
            OriginKind::Apt { .. } => {
                return Err(StatefulError::new("debug breakpoint", &cause));
            }
        };
        let response = reqwest::get(&endpoint)
            .await
            .context(NetSnafu)
            .wrap(&cause)?;
        let body = response.text().await.context(NetSnafu).wrap(&cause)?;
        file.write_all(body.as_bytes())
            .await
            .context(TokioIOSnafu)
            .wrap(&cause)?;
        match self.install_kind {
            ProcessedInstallKind::PreBuilt(_) => {
                return Err(StatefulError::new("debug breakpoint", &cause)); //thingy
            }
            ProcessedInstallKind::Compilable(compilable) => {
                let shell = SettingsJson::get_settings().await.wrap(&cause)?.shell;
                let build = compilable.build.replace("{$~}", &tmpfile.1);
                let mut command = RunCommand::new(shell.to_string());
                command
                    .arg("-c")
                    .arg(build)
                    .status()
                    .await
                    .context(TokioIOSnafu)
                    .wrap(&cause)?;
                let install = compilable.install.replace("{$~}", &tmpfile.1);
                let mut command = RunCommand::new(shell.to_string());
                command
                    .arg("-c")
                    .arg(install)
                    .status()
                    .await
                    .context(TokioIOSnafu)
                    .wrap(&cause)?;
            }
        }
        metadata.write(pool).await.wrap(&cause)?;
        for dep in deps.0 {
            let dep = dep.get_installed_specific(pool).await.wrap(&cause)?;
            dep.write_dependent(&name, &ver, pool).await.wrap(&cause)?;
        }
        Ok(())
    }
    pub async fn write(self, pool: &SqlitePool) -> Result<Self, StatefulError> {
        let cause = json!({"action": "writing metadata to cache database"});
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
        //     .wrap(location!())?;
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
            .wrap(&cause)?;
        // let data = serde_json::to_string(&self)
        //     .context(JSONSnafu {
        //         loc: self.name.to_string(),
        //     })
        //     .wrap(location!())?;
        // file.write_all(data.as_bytes())
        //     .context(IOSnafu {
        //         action: IOAction::WriteFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap(location!())?;
        Ok(self)
    }
    pub async fn open(name: &str, pool: &SqlitePool) -> Result<Self, StatefulError> {
        let cause = json!({"action": "reading metadata to cache database"});
        // let mut path = get_update_dir().wrap(location!())?;
        // path.push(format!("{}.json", name));
        // let mut file = File::open(&path)
        //     .context(IOSnafu {
        //         action: IOAction::OpenFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap(location!())?;
        // let mut metadata = String::new();
        // file.read_to_string(&mut metadata)
        //     .context(IOSnafu {
        //         action: IOAction::ReadFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap(location!())?;
        // serde_json::from_str::<Self>(&metadata)
        //     .context(JSONSnafu {
        //         loc: path.display().to_string(),
        //     })
        //     .wrap(location!())
        query_as::<Sqlite, ProcessedMetaData>("SELECT * FROM updates WHERE name = ?")
            .bind(name)
            .fetch_one(pool)
            .await
            .context(SQLSnafu)
            .wrap(&cause)
    }
    pub async fn get_metadata(
        name: &str,
        version: Option<&str>,
        sources: &[OriginKind],
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<Self, StatefulError> {
        let cause = json!({"action": "pulling package metadata", "package": name});
        let mut metadata = Err(StatefulError::new("No metadata!", &cause));
        for source in sources {
            match source {
                OriginKind::Plz { .. } => {
                    let endpoint = source.to_endpoint(name, version).wrap(&cause)?;
                    let body = reqwest::get(endpoint)
                        .await
                        .context(NetSnafu)
                        .wrap(&cause)?
                        .text()
                        .await
                        .context(NetSnafu)
                        .wrap(&cause)?;
                    if let Ok(rawplz) = serde_json::from_str::<RawPlz>(&body) {
                        metadata = rawplz.to_processed(dependent);
                        break;
                    }
                    //     && let Some(processed) = rawplz.process()
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
                OriginKind::Apt { .. } => {
                    let endpoint = source.to_endpoint(name, version).wrap(&cause)?;
                    let vers = RawApt::get_vers(&endpoint, Some(&endpoint), name).await;
                    let Some(ver) = (if let Some(version) = version {
                        vers.into_iter().find(|x| x.1.to_string() == version)
                    } else {
                        let mut vers = vers.into_iter().collect::<Vec<(String, Version, Arch)>>();
                        vers.sort_by(|a, b| a.1.cmp(&b.1));
                        vers.into_iter().next_back()
                    }) else {
                        continue;
                    };
                    metadata = RawApt::parse(source, &endpoint, name, &ver.0, dependent, pool)
                        .await
                        .wrap(&cause);
                    break;
                }
            }
        }
        metadata
    }
    pub async fn remove_update_cache(&self, pool: &SqlitePool) -> Result<(), StatefulError> {
        // let path = get_update_dir().wrap(location!())?;
        // let dir = fs::read_dir(&path)
        //     .context(IOSnafu {
        //         action: IOAction::ReadDir,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap(location!())?;
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
        //                 .wrap(location!());
        //         }
        //     }
        // }
        query("DELETE FROM updates WHERE name = ?")
            .bind(&self.name)
            .execute(pool)
            .await
            .context(SQLSnafu)
            .wrap(&json!({"action": "removing cached update metadata"}))?;
        // println!(
        //     "\x1B[33m[WARN] cache for {} already cleared!\x1B[0m",
        //     self.name
        // );
        Ok(())
    }
    pub async fn get_depends(
        &self,
        sources: &[OriginKind],
        prior: &mut HashSet<Specific>,
        pool: &SqlitePool,
    ) -> Result<InstallPackage, StatefulError> {
        let cause = json!({"action": "reading dependencies for package", "package": self.name});
        let mut package = InstallPackage {
            metadata: self.clone(),
            build_deps: Vec::new(),
            run_deps: Vec::new(),
        };
        package.build_deps =
            DependKind::batch_as_installed(&self.build_dependencies, sources, prior, pool)
                .await
                .wrap(&cause)?;
        package.run_deps =
            DependKind::batch_as_installed(&self.runtime_dependencies, sources, prior, pool)
                .await
                .wrap(&cause)?;
        Ok(package)
    }
    pub async fn upgrade_package(
        &self,
        sources: &[OriginKind],
        pool: &SqlitePool,
    ) -> Result<(), StatefulError> {
        let cause = json!({"action": "upgrading package", "package": self.name});
        let version = Version::parse(&self.version).wrap(&cause)?;
        let specific = self.as_specific().wrap(&cause)?;
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
                s_children.push(child.await.wrap(&cause)?);
            }
            s_children
        };
        for child in children.into_iter() {
            child.install_package(pool).await.wrap(&cause)?;
        }
        for stale in stale_installed {
            stale
                .get_installed_specific(pool)
                .await
                .wrap(&cause)?
                .remove(false, Some(pool))
                .await
                .wrap(&cause)?;
        }
        for dep in new_deps {
            if let Some(dep_ver) = dep.as_dep_ver() {
                let cause_outer = json!({"action": "installing package dependencies", "package": self.name, "dependency": dep.name()});
                let installed_metadata = InstalledMetaData::open(&dep_ver.name, pool)
                    .await
                    .wrap(&cause)
                    .wrap(&cause_outer)?
                    .context(OtherSnafu {
                        error: format!("Failed to locate `{}`!", self.name),
                    })
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
                let metadata = dep_ver
                    .pull_metadata(Some(sources), installed_metadata.dependent, pool)
                    .await
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
                metadata
                    .install_package(pool)
                    .await
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
            }
        }
        for package in in_place_upgrade {
            if let Some(dep_ver) = package.as_dep_ver() {
                let cause_outer = json!({"action": "upgrading package dependencies", "package": self.name, "dependency": dep_ver.name});
                let name = dep_ver.name.to_string();
                let metadata = InstalledMetaData::open(&name, pool).await.wrap(&cause)?;
                let old_metadata = metadata
                    .context(OtherSnafu {
                        error: "Cannot find data for package `{name}`!",
                    })
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
                let metadata = dep_ver
                    .pull_metadata(Some(sources), old_metadata.dependent, pool)
                    .await
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
                if metadata.version != old_metadata.version {
                    metadata
                        .install_package(pool)
                        .await
                        .wrap(&cause)
                        .wrap(&cause_outer)?;
                }
                let mut metadata = InstalledMetaData::open(&name, pool)
                    .await
                    .wrap(&cause)
                    .wrap(&cause_outer)?
                    .context(OtherSnafu {
                        error: format!("Failed to locate `{}`!", self.name),
                    })
                    .wrap(&cause)
                    .wrap(&cause_outer)?;
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
                metadata.write(pool).await.wrap(&cause).wrap(&cause_outer)?;
            }
        }
        self.clone().install_package(pool).await.wrap(&cause)?;
        Ok(())
    }
    pub fn as_specific(&self) -> Result<Specific, StatefulError> {
        Ok(Specific {
            name: self.name.to_string(),
            version: Version::parse(&self.version).wrap(
                &json!({"action": "parsing as `specific` package type`", "package": self.name}),
            )?,
        })
    }
}
