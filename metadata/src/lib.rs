use settings::{OriginKind, SettingsJson};
use snafu::{OptionExt, ResultExt};
use sqlx::{Sqlite, SqlitePool, query_as};
use std::collections::HashSet;
use utils::{
    FuckNest, FuckWrap, Range, VerReq, Version,
    errors::{SQLSnafu, SystemSnafu, WhereError},
    get_pool,
};

use crate::{
    depend_kind::DependKind,
    installed::{InstalledInstallKind, InstalledMetaData},
    parsers::{MetaDataKind, dew},
    processed::ProcessedMetaData,
    versioning::{DepVer, Specific},
};

pub mod depend_kind;
pub mod installed;
pub mod parsers;
pub mod processed;
pub mod versioning;

async fn get_installed_metadata(
    name: &str,
    pool: &SqlitePool,
) -> Result<Option<InstalledMetaData>, WhereError> {
    query_as::<Sqlite, InstalledMetaData>("SELECT * FROM installed WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await
        .context(SQLSnafu)
        .wrap()
}

#[derive(Debug)]
pub struct QueuedChanges {
    pub primary: HashSet<Specific>,
    pub secondary: HashSet<Specific>,
}

impl QueuedChanges {
    pub fn new() -> Self {
        Self {
            primary: HashSet::new(),
            secondary: HashSet::new(),
        }
    }
    pub fn extend(&mut self, other: Self) {
        self.primary.extend(other.primary);
        self.secondary.extend(other.secondary);
    }
    pub fn insert_primary(&mut self, other: Specific) -> bool {
        self.primary.insert(other)
    }
    pub fn insert_secondary(&mut self, other: Specific) {
        self.secondary.insert(other);
    }
    pub fn is_empty(&self) -> bool {
        self.primary.is_empty()
    }
    pub fn has_deps(&self) -> bool {
        !self.secondary.is_empty()
    }
    pub async fn dependents(&mut self, pool: &SqlitePool) -> Result<(), WhereError> {
        let mut items = self.primary.iter().cloned().collect::<Vec<Specific>>();
        items.extend_from_slice(&self.secondary.iter().cloned().collect::<Vec<Specific>>());
        for item in items {
            item.get_dependents(self, pool)
                .await
                .nest("Get Package Dependents")?;
        }
        Ok(())
    }
}

impl Default for QueuedChanges {
    fn default() -> Self {
        Self::new()
    }
}
/* #region Install */
#[derive(Debug)]
pub struct InstallPackage {
    pub metadata: ProcessedMetaData,
    pub build_deps: Vec<InstallPackage>,
    pub run_deps: Vec<InstallPackage>,
}

impl InstallPackage {
    pub fn list_deps(&self, top: bool) -> HashSet<String> {
        let mut data = HashSet::new();
        if !top {
            data.insert(self.metadata.name.to_string());
        }
        for dep in &self.run_deps {
            data.extend(dep.list_deps(false));
        }
        data
    }
    pub async fn install(self) -> Result<(), WhereError> {
        let pool = get_pool().await.nest("Get Sqlite Pool")?;
        let mut collected: Vec<ProcessedMetaData> = self.collect()?;
        let depends = collected
            .iter()
            .map(|x| {
                x.build_dependencies
                    .0
                    .iter()
                    .chain(x.runtime_dependencies.0.iter())
                    .collect::<Vec<&DependKind>>()
            })
            .collect::<Vec<Vec<&DependKind>>>()
            .into_iter()
            .flatten()
            .flat_map(|x| x.as_dep_ver())
            .collect::<Vec<DepVer>>();
        let mut sets = Vec::new();
        while let Some(metadata) = &collected.first() {
            let name = metadata.name.to_string();
            let set = collected
                .extract_if(.., |x| x.name == name)
                .collect::<Vec<ProcessedMetaData>>();
            sets.push(set);
        }
        let sets = sets.into_iter();
        let mut filtered: Vec<ProcessedMetaData> = Vec::new();
        for mut set in sets {
            if set.is_empty() {
                continue;
            } else if set.len() == 1
                && let Some(metadata) = set.first()
            {
                filtered.push(metadata.clone());
            } else if let Some(name) = set.first().map(|x| x.name.to_string()) {
                let range = depends
                    .iter()
                    .filter(|x| x.name == name)
                    .try_fold(
                        Range {
                            lower: VerReq::NoBound,
                            upper: VerReq::NoBound,
                        },
                        |acc, x| x.range.negotiate(Some(acc)),
                    )
                    .context(SystemSnafu {
                        message: "Common version of dependent could not be negotiated by dependencies",
                        package: name.to_string(),
                    })
                    .wrap()?;
                set.sort_by_key(|x| Version::parse(&x.version).ok());
                set.reverse();
                let mut chosen = None;
                for metadata in set {
                    let ver_req = VerReq::Eq(Version::parse(&metadata.version).wrap()?);
                    let new_range = Range {
                        lower: ver_req.clone(),
                        upper: ver_req,
                    };
                    if new_range.negotiate(Some(range.clone())).is_some() {
                        chosen = Some(metadata);
                        break;
                    }
                }
                let chosen = chosen
                    .context(SystemSnafu {
                        message: "No version of dependent could be agreed by dependencies",
                        package: name.to_string(),
                    })
                    .wrap()?;
                filtered.push(chosen);
            }
        }
        for metadata in filtered {
            metadata
                .install_package(&pool)
                .await
                .nest("Install Package")?;
        }
        Ok(())
    }
    pub fn collect(self) -> Result<Vec<ProcessedMetaData>, WhereError> {
        let mut result = Vec::new();
        for dep in self.build_deps {
            let data = dep.collect()?;
            result.extend_from_slice(&data);
        }
        for dep in self.run_deps {
            let data = dep.collect()?;
            result.extend_from_slice(&data);
        }
        result.push(self.metadata);
        Ok(result)
    }
}

pub async fn get_packages(
    args: &[(&String, Option<&String>)],
) -> Result<Vec<InstallPackage>, WhereError> {
    let pool = get_pool().await.nest("Get Sqlite Pool")?;
    print!("\x1B[2K\rBuilding dependency tree... 0%");
    let settings = SettingsJson::get_settings().wrap()?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let count = args.len();
    for (i, package) in args.iter().enumerate() {
        if let Some(data) = get_package(&settings.sources, package, false, &mut seen, &pool).await?
        {
            result.push(data);
        }
        print!("\rBuilding dependency tree... {}%", i * 100 / count);
    }
    print!("\rBuilding dependency tree... Done!");
    Ok(result)
}

async fn get_package(
    sources: &[OriginKind],
    dep: &(&String, Option<&String>),
    dependent: bool,
    prior: &mut HashSet<Specific>,
    pool: &SqlitePool,
) -> Result<Option<InstallPackage>, WhereError> {
    let (app, version) = dep;
    let metadata =
        ProcessedMetaData::get_metadata(app, version.map(|x| x.as_str()), sources, dependent, pool)
            .await
            .context(SystemSnafu {
                message: "Discovery failed",
                package: app.to_string(),
            })
            .nest("Download Package Metadata")?;
    if let Ok(Some(installed)) = InstalledMetaData::open(&metadata.name, pool).await
        && installed.version == metadata.version
    {
        return Ok(None);
    };
    Ok(Some(
        ProcessedMetaData::get_depends(&metadata, sources, prior, pool)
            .await
            .nest("Get Package Dependencies")?,
    ))
}

/* #endregion Install */
/* #region Remove/Purge */
pub async fn get_local_pkgs(
    args: &[(&String, Option<&String>)],
) -> Result<QueuedChanges, WhereError> {
    let pool = get_pool().await.nest("Get Sqlite Pool")?;
    print!("\x1B[2K\rCollecting packages... 0%");
    let mut seen = HashSet::new();
    let count = args.len();
    let mut result = QueuedChanges::new();
    for (i, dep) in args.iter().enumerate() {
        print!("\rCollecting packages... {}% ", i * 100 / count);
        result.extend(get_local_pkg(dep, &mut seen, true, &pool).await?);
    }
    print!("\rCollecting packages... Done!");
    result
        .dependents(&pool)
        .await
        .nest("Get Package Dependents")?;
    Ok(result)
}

async fn get_local_pkg(
    dep: &(&String, Option<&String>),
    prior: &mut HashSet<String>,
    root: bool,
    pool: &SqlitePool,
) -> Result<QueuedChanges, WhereError> {
    let (dep, ver) = *dep;
    let data = match InstalledMetaData::open(dep, pool)
        .await
        .nest("Locate Package Metadata")?
        .context(SystemSnafu {
            message: "Discovery failed",
            package: dep.to_string(),
        })
        .wrap()
    {
        Ok(data) => data,
        fault => {
            if root {
                fault.nest("Attempted to remove a package that isn't installed!")?
            } else {
                fault?
            }
        }
    };
    let mut working = Vec::new();
    if let Some(ver) = ver {
        if data.version == *ver {
            working.push(data);
        }
    } else if !data.dependent {
        working.push(data);
    }
    let mut result = QueuedChanges::new();
    for version in working {
        for dependency in &version.dependencies.0 {
            if prior.contains(&dependency.name) {
                continue;
            } else {
                prior.insert(dependency.name.to_string());
            }
            let Ok(dep) = dependency.get_installed_specific(pool).await else {
                continue;
            };
            let items = Box::pin(get_local_pkg(
                &(&dependency.name, Some(&dep.version.to_string())),
                prior,
                false,
                pool,
            ))
            .await?;
            result.extend(items);
            result.insert_secondary(dep);
        }
        if root {
            result.insert_primary(Specific {
                name: version.name.to_string(),
                version: Version::parse(&version.version).wrap()?,
            });
        }
    }
    Ok(result)
}

/* #endregion Remove/Purge */
/* #region Update */
pub async fn collect_updates() -> Result<(), WhereError> {
    let pool = get_pool().await.nest("Get Sqlite Pool")?;
    let _settings = SettingsJson::get_settings().wrap()?;
    print!("\x1B[2K\rReading package lists... 0%");
    // let path = get_metadata_dir().nest("Get Metadata Directory")?;
    // let dir = fs::read_dir(&path)
    //     .context(IOSnafu {
    //         action: IOAction::ReadDir,
    //         loc: path.display().to_string(),
    //     })
    //     .wrap()?;
    // let mut children = Vec::new();
    // for file in dir.flatten() {
    //     children.push(collect_update(file.path(), &settings.sources));
    // }
    // let path = get_update_dir().wrap()?;
    let _children = query_as::<Sqlite, InstalledMetaData>("SELECT * FROM installed WHERE kind = 0")
        .fetch_all(&pool)
        .await
        .context(SQLSnafu)
        .wrap()?;
    // let mut result = Vec::new();
    // let count = children.len();
    // for (i, child) in children.into_iter().enumerate() {
    //     print!("\rReading package lists... {}%", i * 100 / count);
    //     result.extend(child.await?);
    // }
    print!("\rReading package lists... Done!\nSaving upgrade data... 0%");
    // let dir = fs::read_dir(&path)
    //     .context(IOSnafu {
    //         action: IOAction::ReadDir,
    //         loc: path.display().to_string(),
    //     })
    //     .wrap()?;
    // let mut old = Vec::new();
    // for file in dir.flatten() {
    //     if let Some(name) = file.path().file_prefix() {
    //         old.push(
    //             ProcessedMetaData::open(&name.to_string_lossy())
    //                 .nest("Locate Package Upgrade Metadata")?
    //                 .name,
    //         );
    //     }
    // }
    // let count = result.len();
    // for (i, data) in result
    //     .into_iter()
    //     .filter(|x| !old.contains(&x.name))
    //     .enumerate()
    // {
    //     print!("\rSaving upgrade data... {}%", i * 100 / count);
    //     data.write(&path).nest("Write Package Upgrade Cache")?;
    // }
    println!("\rSaving upgrade data... Done!");
    Ok(())
}

// async fn collect_update(
//     path: PathBuf,
//     sources: &[OriginKind],
// ) -> Result<Vec<ProcessedMetaData>, WhereError> {
//     let mut result = Vec::new();
//     if path.extension().is_none_or(|x| x != "json") {
//         return Ok(Vec::new());
//     }
//     let name = if let Some(name) = path.file_prefix() {
//         name.to_string_lossy()
//     } else {
//         return Ok(Vec::new());
//     };
//     let metadata = InstalledMetaData::open(&name).nest("Locate Package Metadata")?;
//     let name = metadata.name;
//     let name = name.to_string();
//     if metadata.dependents.is_empty()
//         && !metadata.dependent
//         && let Some(data) =
//             ProcessedMetaData::get_metadata(&name, None, sources, metadata.dependent).await
//         && Version::parse(&data.version).wrap()? > Version::parse(&metadata.version).wrap()?
//     {
//         result.push(data);
//     }

//     Ok(result)
// }
/* #endregion Update */
/* #region Upgrade */
pub fn upgrade_all() -> Result<Vec<ProcessedMetaData>, WhereError> {
    // let path = get_update_dir().wrap()?;
    // let dir = fs::read_dir(&path)
    //     .context(IOSnafu {
    //         action: IOAction::ReadDir,
    //         loc: path.display().to_string(),
    //     })
    //     .wrap()?;
    // let mut result = Vec::new();
    // for file in dir.flatten() {
    //     let path = file.path();
    //     if path.extension().is_some_and(|x| x == "json")
    //         && let Some(name) = path.file_prefix()
    //     {
    //         let name = name.to_string_lossy();
    //         result.push(ProcessedMetaData::open(&name).nest("Locate Package Upgrade Metadata")?);
    //     }
    // }
    // Ok(result)
    Ok(Vec::new())
}

pub fn upgrade_only(
    pkgs: &[(&String, Option<&String>)],
) -> Result<Vec<ProcessedMetaData>, WhereError> {
    let base = upgrade_all()?;
    let base = base.iter();
    let mut result = HashSet::new();
    for pkg in pkgs {
        let (pkg, ver) = *pkg;
        if let Some(ver) = ver {
            let found = base
                .as_ref()
                .iter()
                .filter(|x| x.name == *pkg && x.version == *ver)
                .cloned()
                .collect::<Vec<ProcessedMetaData>>();
            result.extend(found);
        } else {
            let found = base
                .as_ref()
                .iter()
                .filter(|x| x.name == *pkg)
                .cloned()
                .collect::<Vec<ProcessedMetaData>>();
            result.extend(found);
        }
    }
    Ok(result.into_iter().collect())
}

pub async fn upgrade_packages(packages: &[ProcessedMetaData]) -> Result<(), WhereError> {
    let pool = get_pool().await.nest("Get Sqlite Pool")?;
    let settings = SettingsJson::get_settings().wrap()?;
    for package in packages {
        println!("Upgrading {}...", package.name);
        package
            .upgrade_package(&settings.sources, &pool)
            .await
            .nest("Upgrade Package")?;
        package
            .remove_update_cache(&pool)
            .await
            .nest("Remove Stale Upgrade Package Cache")?;
    }
    println!("Done!");
    Ok(())
}

/* #endregion Upgrade */
/* #region Unbind */
pub async fn unbind(data: &[(&String, Option<&String>)]) -> Result<(), WhereError> {
    let pool = get_pool().await.nest("Get Sqlite Pool")?;
    for bit in data {
        let (dep, ver) = *bit;
        let data = get_installed_metadata(dep, &pool)
            .await
            .nest("Get Installed Metadata")?;
        let mut data = data
            .context(SystemSnafu {
                message: "Cannot find data",
                package: dep.to_string(),
            })
            .wrap()?;
        if let Some(ver) = ver {
            println!("Emancipating `{dep}` version {ver}...",);
            if data.version == *ver {
                if !data.dependent {
                    println!(
                        "\x1B[33m[WARN] `{dep}` version {ver} is already independent!\x1B[0m..."
                    );
                    continue;
                }
                data.dependent = false;
            }
        } else {
            println!("Emancipating `{dep}`...",);
            if data.dependent {
                data.dependent = false;
            } else {
                println!(
                    "\x1B[33m[WARN] All versions of `{dep}` are already independent!\x1B[0m...",
                );
            }
        };
        data.write(&pool)
            .await
            .nest("Write Changes to Package Metadata")?;
    }
    Ok(())
}
/* #endregion Unbind */
