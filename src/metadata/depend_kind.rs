use serde::{Deserialize, Serialize};
use snafu::{OptionExt, location};
use sqlx::{Decode, Encode, Sqlite, SqlitePool, Type};
use std::{
    collections::HashSet,
    fmt::{self, Display, Formatter},
};

use crate::errors::{OtherSnafu, Wrapped, WrappedError};
use crate::metadata::{
    DepVer, InstallPackage, Specific, get_installed_metadata, processed::ProcessedMetaData,
};
use crate::settings::OriginKind;
use crate::utils::{range::Range, verreq::VerReq, version::Version, which};

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
                if which(volatile) {
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
        deps: &DependKindVec,
        sources: &[OriginKind],
        prior: &mut HashSet<Specific>,
        pool: &SqlitePool,
    ) -> Result<Vec<InstallPackage>, WrappedError> {
        let mut result = Vec::new();
        for dep in deps.0.iter() {
            let dep = match dep {
                Self::Latest(latest) => {
                    ProcessedMetaData::get_metadata(latest, None, sources, true, pool)
                        .await
                        .wrap()?
                }
                Self::Specific(dep_ver) => {
                    let specific = dep_ver
                        .clone()
                        .pull_metadata(Some(sources), true, pool)
                        .await
                        .wrap()?;
                    ProcessedMetaData::get_metadata(
                        &specific.name,
                        Some(&specific.version.to_string()),
                        sources,
                        true,
                        pool,
                    )
                    .await
                    .wrap()?
                }
                Self::Volatile(volatile) => {
                    if which(volatile) {
                        continue;
                    } else {
                        ProcessedMetaData::get_metadata(volatile, None, sources, true, pool)
                            .await
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
                let child = Box::pin(ProcessedMetaData::get_depends(&dep, sources, prior, pool))
                    .await
                    .wrap()?;
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
    pub async fn choose<T: IntoIterator<Item = Self>>(
        choices: T,
        pool: &SqlitePool,
    ) -> Option<Self> {
        let mut first = None;
        for choice in choices {
            if choice.is_installed(pool).await {
                return None;
            } else if first.is_none() {
                first = Some(choice);
            }
        }
        first
    }
    async fn is_installed(&self, pool: &SqlitePool) -> bool {
        match self {
            Self::Latest(latest) => match get_installed_metadata(latest, pool).await {
                Ok(data) => data.is_some(),
                Err(_) => false,
            },
            Self::Specific(specific) => match get_installed_metadata(&specific.name, pool).await {
                Ok(data) => {
                    if let Some(data) = data {
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
                if which(volatile) {
                    true
                } else {
                    match get_installed_metadata(volatile, pool).await {
                        Ok(value) => value.is_some(),
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
    fn parse(input: &str) -> Result<Self, WrappedError> {
        let mut chars = input.chars();
        let kind = chars.next().context(OtherSnafu {
            error: "Missing type identifier!",
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            1 => Ok(Self::Latest(data)),
            2 => Ok(Self::Specific(DepVer::parse(&data).wrap()?)),
            3 => Ok(Self::Volatile(data)),
            kind => Err(WrappedError::Other {
                error: format!("Invalid kind identifier `{kind}`!").into(),
                loc: location!(),
            }),
        }
    }
}

impl Display for DependKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            Self::Latest(latest) => format!("\x01{latest}"),
            Self::Specific(specific) => format!("\x02{specific}"),
            Self::Volatile(volatile) => format!("\x03{volatile}"),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DependKindVec(pub Vec<DependKind>);

impl DependKindVec {
    fn parse(input: &str) -> Result<Self, WrappedError> {
        if input.is_empty() {
            return Ok(Self(Vec::new()));
        }
        let mut vers = Vec::new();
        for ver in input.split('\x00') {
            vers.push(DependKind::parse(ver)?);
        }
        Ok(Self(vers))
    }
}

impl Display for DependKindVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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

impl Type<Sqlite> for DependKindVec {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for DependKindVec {
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

impl<'a> Decode<'a, Sqlite> for DependKindVec {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}
