use std::fmt::Display;

use sqlx::{Decode, Encode, Sqlite, Type, error::BoxDynError};
use utils::errors::{HowError, Parsers};

pub mod apt;
pub mod dew;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MetaDataKind {
    Dew,
    Apt,
}

impl MetaDataKind {
    fn parse(input: &str) -> Result<Self, HowError> {
        let kind = input.chars().next().ok_or(HowError::ParseError {
            message: "Missing type identified!".into(),
            util: Parsers::MetaDataKind,
        })?;
        match kind as u8 {
            0 => Ok(Self::Dew),
            1 => Ok(Self::Apt),
            kind => Err(HowError::ParseError {
                message: format!("Invalid kind identifier `{kind}`!").into(),
                util: Parsers::MetaDataKind,
            }),
        }
    }
}

impl Display for MetaDataKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dew => "\x00",
            Self::Apt => "\x01",
        })
    }
}

impl Type<Sqlite> for MetaDataKind {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for MetaDataKind {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, BoxDynError> {
        <String as Encode<'_, Sqlite>>::encode_by_ref(&self.to_string(), buf)
    }
    fn encode(
        self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, BoxDynError>
    where
        Self: Sized,
    {
        <String as Encode<'_, Sqlite>>::encode(self.to_string(), buf)
    }
}

impl<'a> Decode<'a, Sqlite> for MetaDataKind {
    fn decode(value: <Sqlite as sqlx::Database>::ValueRef<'a>) -> Result<Self, BoxDynError> {
        let data: String = Decode::<Sqlite>::decode(value)?;
        Ok(Self::parse(&data)?)
    }
}
