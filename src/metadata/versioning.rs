use crate::errors::{HowError, Parsers, SQLSnafu, SystemSnafu, WhereError};
use crate::metadata::{
    FuckNest, FuckWrap, QueuedChanges, get_installed_metadata,
    installed::{InstalledInstallKind, InstalledMetaData},
    processed::ProcessedMetaData,
};
use crate::settings::{OriginKind, SettingsJson};
use crate::utils::{Range, VerReq, Version, get_pool};

use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, location};
use sqlx::{Decode, Encode, Sqlite, SqlitePool, Type, query, query_as};
use std::{fmt::Display, process::Command};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DepVer {
    pub name: String,
    pub range: Range,
}

impl DepVer {
    pub async fn get_installed_specific(&self, pool: &SqlitePool) -> Result<Specific, WhereError> {
        let metadata = InstalledMetaData::open(&self.name, pool)
            .await
            .nest("Locate Package Metadata")?
            .context(SystemSnafu {
                message: "Discovery failed",
                package: self.name.to_string(),
            })
            .wrap()?;
        Ok(Specific {
            name: metadata.name,
            version: Version::parse(&metadata.version).wrap()?,
        })
    }
    pub async fn pull_metadata(
        self,
        sources: Option<&[OriginKind]>,
        dependent: bool,
        pool: &SqlitePool,
    ) -> Result<ProcessedMetaData, WhereError> {
        let sources = match sources {
            Some(sources) => sources,
            None => &SettingsJson::get_settings().wrap()?.sources,
        };
        let mut versions = None;
        let mut g_source = None;
        let name = self.name;
        for source in sources {
            match source {
                OriginKind::Dew(dew) => {
                    let endpoint = format!("{dew}/package/{name}");
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
        ProcessedMetaData::get_metadata(&name, Some(&ver), &[source], dependent, pool)
            .await
            .context(SystemSnafu {
                message: format!("Failed to locate version {ver}"),
                package: name,
            })
            .wrap()
    }
    pub fn parse(input: &str) -> Result<Self, HowError> {
        let (name, range) = input.split_once(' ').ok_or(HowError::ParseError {
            message: "Missing DepVer field `range`!".into(),
            util: Parsers::DepVer,
        })?;
        let range = Range::parse(range)?;
        Ok(Self {
            name: name.to_string(),
            range,
        })
    }
}

impl Display for DepVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} {}", self.name, self.range))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DepVerVec(pub Vec<DepVer>);

impl DepVerVec {
    fn parse(input: &str) -> Result<Self, HowError> {
        if input.is_empty() {
            return Ok(Self(Vec::new()));
        }
        let mut vers = Vec::new();
        for ver in input.split("\x00\x00") {
            vers.push(DepVer::parse(ver)?);
        }
        Ok(Self(vers))
    }
}

impl Display for DepVerVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.0.iter().fold(String::new(), |mut acc, x| {
            if !acc.is_empty() {
                acc.push_str("\x00\x00");
            }
            acc.push_str(&x.to_string());
            acc
        });
        f.write_str(&data)
    }
}

impl Type<Sqlite> for DepVerVec {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for DepVerVec {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError>
    where
        Self: Sized,
    {
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

impl<'a> Decode<'a, Sqlite> for DepVerVec {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Specific {
    pub name: String,
    pub version: Version,
}

impl Specific {
    pub async fn write_dependent(
        &self,
        their_name: &str,
        their_ver: &str,
        pool: &SqlitePool,
    ) -> Result<(), WhereError> {
        let mut data =
            query_as::<Sqlite, InstalledMetaData>("SELECT * FROM installed WHERE name = ?")
                .bind(&self.name)
                .fetch_one(pool)
                .await
                .context(SQLSnafu)
                .wrap()?;
        if data.version == self.version.to_string() {
            let their_dep = Self {
                name: their_name.to_string(),
                version: Version::parse(their_ver).wrap()?,
            };
            if let Some(found) = data
                .dependents
                .0
                .iter_mut()
                .find(|x| x.name == their_dep.name)
            {
                found.version = their_dep.version;
            } else if !data.dependents.0.contains(&their_dep)
                && let Ok(Some(their_metadata)) = InstalledMetaData::open(their_name, pool).await
                && their_metadata.version == their_ver
            {
                data.dependents.0.push(their_dep);
            }
            query("UPDATE installed SET dependents = ? WHERE name = ?")
                .bind(data.dependents)
                .bind(data.name)
                .execute(pool)
                .await
                .context(SQLSnafu)
                .wrap()?;
        }
        Ok(())
        // let (path, data) = get_metadata_path(&self.name)?;
        // if path.exists()
        //     && path.is_file()
        //     && let Some(mut data) = data
        // {

        //     }
        //     let mut file = File::create(&path)
        //         .context(IOSnafu {
        //             action: IOAction::CreateFile,
        //             loc: path.display().to_string(),
        //         })
        //         .wrap()?;
        //     let data = serde_json::to_string(&data)
        //         .context(JSONSnafu {
        //             loc: data.name.to_string(),
        //         })
        //         .wrap()?;
        //     file.write_all(data.as_bytes())
        //         .context(IOSnafu {
        //             action: IOAction::WriteFile,
        //             loc: path.display().to_string(),
        //         })
        //         .wrap()
        // } else {
        //     Err(HowError::SystemError {
        //         message: format!("Failed to find data for dependency `{}`", self.name).into(),
        //         package: their_name.to_string().into(),
        //     })
        //     .wrap()
        // }
    }
    pub async fn get_dependents(
        &self,
        queued: &mut QueuedChanges,
        pool: &SqlitePool,
    ) -> Result<(), WhereError> {
        let data = InstalledMetaData::open(&self.name, pool)
            .await
            .nest("Locate Package Metadata")?
            .context(SystemSnafu {
                message: "Discovery failed",
                package: self.name.to_string(),
            })
            .wrap()?;
        if data.version == self.version.to_string() {
            for dependent in &data.dependents.0 {
                if queued.insert_primary(dependent.clone()) {
                    Box::pin(dependent.get_dependents(queued, pool))
                        .await
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
    pub async fn remove(&self, purge: bool, pool: Option<&SqlitePool>) -> Result<(), WhereError> {
        let msg = if purge { "Purging" } else { "Removing" };
        let pool = match pool {
            Some(pool) => pool,
            None => &get_pool().await.nest("Get Sqlite Pool")?,
        };
        println!("{} {} version {}...", msg, self.name, self.version);
        let data = get_installed_metadata(&self.name, pool)
            .await
            .nest("Get Installed Metadata")?;
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
        for dep in &data.dependencies.0
        // .iter()
        // .flat_map(|x| x.get_installed_specific(pool).await)
        // .collect::<Vec<Specific>>()
        {
            let Ok(dep) = dep.get_installed_specific(pool).await else {
                continue;
            };
            data.clear_dependencies(&dep, pool)
                .await
                .nest("Remove Dependency from Package")?;
            Box::pin(dep.remove(purge, Some(pool)))
                .await
                .nest("Remove/Purge Package")?;
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
        query("DELETE FROM installed WHERE name = ?")
            .bind(&self.name)
            .execute(pool)
            .await
            .context(SQLSnafu)
            .wrap()?;
        Ok(())
        // fs::remove_file(&path)
        //     .context(IOSnafu {
        //         action: IOAction::RemoveFile,
        //         loc: path.display().to_string(),
        //     })
        //     .wrap()
    }
    fn parse(input: &str) -> Result<Self, HowError> {
        let (name, version) = input.split_once(' ').ok_or(HowError::ParseError {
            message: "Missing Specific field `version`!".into(),
            util: Parsers::Specific,
        })?;
        let version = Version::parse(version)?;
        Ok(Self {
            name: name.to_string(),
            version,
        })
    }
}

impl Display for Specific {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} {}", self.name, self.version))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SpecificVec(pub Vec<Specific>);

impl SpecificVec {
    fn parse(input: &str) -> Result<Self, HowError> {
        if input.is_empty() {
            return Ok(Self(Vec::new()));
        }
        let mut vers = Vec::new();
        for ver in input.split('\x00') {
            vers.push(Specific::parse(ver)?);
        }
        Ok(Self(vers))
    }
}

impl Display for SpecificVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.0.iter().fold(String::new(), |mut acc, x| {
            if !acc.is_empty() {
                acc.push('\x00');
            }
            acc.push_str(&x.to_string());
            acc
        });
        f.write_str(&data)
    }
}

impl Type<Sqlite> for SpecificVec {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for SpecificVec {
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

impl<'a> Decode<'a, Sqlite> for SpecificVec {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}
