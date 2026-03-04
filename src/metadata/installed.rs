use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, location};
use sqlx::{Decode, Encode, FromRow, Sqlite, SqlitePool, Type, query, query_as};
use std::fmt::{self, Display, Formatter};

use crate::errors::{OtherSnafu, SQLSnafu, Wrapped, WrappedError};
use crate::metadata::{
    MetaDataKind, Specific,
    processed::PreBuilt,
    versioning::{DepVerVec, SpecificVec},
};
use crate::settings::OriginKind;

#[derive(Clone, Debug, Encode, FromRow, PartialEq)]
pub struct InstalledMetaData {
    pub name: String,
    pub kind: MetaDataKind,
    pub version: String,
    pub origin: OriginKind,
    pub dependent: bool,
    pub dependencies: DepVerVec,
    pub dependents: SpecificVec,
    pub install_kind: InstalledInstallKind,
    pub hash: String,
}

impl InstalledMetaData {
    pub async fn open(name: &str, pool: &SqlitePool) -> Result<Option<Self>, WrappedError> {
        query_as::<Sqlite, Self>("SELECT * FROM installed WHERE name = ?")
            .bind(name)
            .fetch_optional(pool)
            .await
            .context(SQLSnafu)
    }
    pub async fn write(self, pool: &SqlitePool) -> Result<Option<Self>, WrappedError> {
        query::<Sqlite>("INSERT INTO installed VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&self.name)
            .bind(&self.kind)
            .bind(&self.version)
            .bind(&self.origin)
            .bind(self.dependent)
            .bind(&self.dependencies)
            .bind(&self.dependents)
            .bind(&self.install_kind)
            .bind(&self.hash)
            .execute(pool)
            .await
            .context(SQLSnafu)
            .wrap()?;
        Ok(Some(self))
    }
    pub async fn clear_dependencies(
        &self,
        specific: &Specific,
        pool: &SqlitePool,
    ) -> Result<(), WrappedError> {
        let mut data = self.clone();
        // let index = &data
        //     .dependencies
        //     .0
        //     .iter()
        //     .position(|x| {
        //         x.get_installed_specific(pool)
        //             .await
        //             .is_ok_and(|x| x == *specific)
        //     })
        //     .context(SystemSnafu {
        //         message: format!("Dependent `{}` {} not found", data.name, data.version),
        //         package: specific.name.to_string(),
        //     })
        //     .wrap()?;
        let index = {
            let mut e_index = None;
            for index in 0..data.dependencies.0.len() {
                if data.dependencies.0[index]
                    .get_installed_specific(pool)
                    .await
                    .is_ok_and(|x| x == *specific)
                {
                    e_index = Some(index);
                    break;
                }
            }
            e_index.context(OtherSnafu {
                error: format!(
                    "Dependent `{}` {} not found for package `{}`!",
                    data.name, data.version, specific.name
                ),
            })?
        };
        data.dependencies.0.remove(index);
        data.write(pool).await.wrap()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum InstalledInstallKind {
    PreBuilt(PreBuilt),
    Compilable(InstalledCompilable),
}

impl InstalledInstallKind {
    fn parse(input: &str) -> Result<Self, WrappedError> {
        let mut chars = input.chars();
        let kind = chars.next().context(OtherSnafu {
            error: "Missing type identifier!",
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => Ok(Self::PreBuilt(PreBuilt::parse(&data).wrap()?)),
            1 => Ok(Self::Compilable(InstalledCompilable::parse(&data).wrap()?)),
            kind => Err(WrappedError::Other {
                error: format!("Invalid kind identifier `{kind}`!").into(),
                loc: location!(),
            }),
        }
    }
}

impl Type<Sqlite> for InstalledInstallKind {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for InstalledInstallKind {
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

impl<'a> Decode<'a, Sqlite> for InstalledInstallKind {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}

impl Display for InstalledInstallKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            Self::PreBuilt(prebuilt) => format!("\x00{prebuilt}"),
            Self::Compilable(compilable) => format!("\x01{compilable}"),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InstalledCompilable {
    pub uninstall: String,
    pub purge: String,
}

impl InstalledCompilable {
    fn parse(input: &str) -> Result<Self, WrappedError> {
        let (uninstall, purge) = input.split_once('\x00').context(OtherSnafu {
            error: "Missing InstalledCompilable field `purge`!",
        })?;
        Ok(Self {
            uninstall: uninstall.to_string(),
            purge: purge.to_string(),
        })
    }
}

impl Display for InstalledCompilable {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&format!("{}\x00{}", self.uninstall, self.purge))
    }
}
