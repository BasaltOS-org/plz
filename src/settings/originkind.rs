use std::{
    ffi::OsStr,
    fmt::Display,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, Serialize,
    de::{self, Visitor},
    ser::SerializeMap,
};
use serde_json::json;
use sqlx::{Decode, Encode, Sqlite, Type};

use crate::errors::{OtherSnafu, StatefulError, Wrapped, WrappedError};

// pub type OriginKindVec = Vec<OriginKind>;

#[derive(Debug, Default, PartialEq)]
pub struct OriginKindVec(pub Vec<OriginKind>);

#[derive(Deserialize, Serialize)]
struct InnerApt<'a> {
    uri: &'a str,
    suites: Vec<&'a str>,
    components: Vec<AptComponent>,
    signature_path: &'a Path,
}

// impl<'a> Serialize for InnerApt<'a> {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: serde::Serializer,
//     {
//         let mut map = serializer.serialize_map(Some(4))?;
//         map.serialize_entry("uri", self.uri)?;
//         map.serialize_entry("suites", &self.suites)?;
//         map.serialize_entry("components", &self.components)?;
//         map.serialize_entry("signature_path", self.signature_path)?;
//         map.end()
//     }
// }

#[derive(Deserialize, Serialize)]
struct InnerGithub<'a> {
    user: &'a str,
    repo: &'a str,
}

#[derive(Deserialize, Serialize)]
struct InnerPlz<'a> {
    source: &'a str,
}

impl Serialize for OriginKindVec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut apts: Vec<InnerApt> = Vec::new();
        let mut map = serializer.serialize_map(None)?;
        for origin in &self.0 {
            match origin {
                OriginKind::Apt {
                    uri,
                    suite,
                    component,
                    signature_path,
                } => {
                    if let Some(apt) = apts
                        .iter_mut()
                        .find(|x| x.uri == uri && x.signature_path == signature_path)
                    {
                        apt.suites.push(suite);
                        apt.components.push(component.clone());
                    } else {
                        apts.push(InnerApt {
                            uri,
                            suites: vec![suite],
                            components: vec![component.clone()],
                            signature_path,
                        });
                    }
                }
                OriginKind::Github { user, repo } => {
                    map.serialize_entry("Github", &InnerGithub { user, repo })?;
                }
                OriginKind::Plz { source } => {
                    map.serialize_entry("Plz", &InnerPlz { source })?;
                }
            };
        }
        apts.iter_mut().for_each(|x| {
            x.suites.sort();
            x.suites.dedup();
            x.components.sort();
            x.components.dedup();
        });
        for apt in apts {
            map.serialize_entry("Apt", &apt)?;
        }
        map.end()
    }
}

struct OriginKindVecVisitor;

impl<'de> Visitor<'de> for OriginKindVecVisitor {
    type Value = OriginKindVec;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an OriginKind entry.")
    }
    // fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    // where
    //     A: serde::de::SeqAccess<'de>,
    // {
    //     let mut origins = Self::default();
    //     while let Some(data) = seq.next_element::<T>()? {
    //         match data {
    //             (kind, user, repo) if kind == "Github" => {
    //                 origins.0.push(OriginKind::Github { user, repo });
    //             }
    //             (kind, source) if kind == "Pax" => {
    //                 origins.0.push(OriginKind::Plz { source });
    //             }
    //         }
    //     }
    //     panic!()
    // }
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut origins = Self::Value::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "Apt" => {
                    let inner = map.next_value::<InnerApt>()?;
                    let uri = inner.uri;
                    let signature_path = inner.signature_path;
                    for suite in inner.suites {
                        for component in &inner.components {
                            origins.0.push(OriginKind::Apt {
                                uri: uri.to_string(),
                                suite: suite.to_string(),
                                component: component.clone(),
                                signature_path: signature_path.to_path_buf(),
                            });
                        }
                    }
                }
                "Github" => {
                    let inner = map.next_value::<InnerGithub>()?;
                    origins.0.push(OriginKind::Github {
                        user: inner.user.to_string(),
                        repo: inner.repo.to_string(),
                    });
                }
                "Plz" => {
                    let inner = map.next_value::<InnerPlz>()?;
                    origins.0.push(OriginKind::Plz {
                        source: inner.source.to_string(),
                    });
                }
                _ => return Err(de::Error::custom("Unknown key")),
            }
        }
        Ok(origins)
    }
}

impl<'de> Deserialize<'de> for OriginKindVec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(OriginKindVecVisitor)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum OriginKind {
    Apt {
        uri: String,
        suite: String,
        component: AptComponent,
        signature_path: PathBuf,
    },
    Github {
        user: String,
        repo: String,
    },
    Plz {
        source: String,
    },
}

impl OriginKind {
    pub fn to_endpoint(&self, name: &str, version: Option<&str>) -> Result<String, StatefulError> {
        let cause = json!({"action": "converting OriginKind to source", "package": name});
        match self {
            Self::Apt { .. } => {
                Err(StatefulError::new(
                    "The code for this section is being rewritten. Please update PLZ.",
                    &cause,
                ))
                // // example mirror: https://au.archive.ubuntu.com/ubuntu/pool/universe/
                // // example prefer: https://au.archive.ubuntu.com/ubuntu/pool/universe/n/node/
                // let folder = if name.starts_with("lib") && name.len() > 3 {
                //     name[0..4].to_string()
                // } else if !name.is_empty() {
                //     name[0..1].to_string()
                // } else {
                //     return Err(StatefulError::new(
                //         format!("Invalid requested package name `{name}`!"),
                //         &cause,
                //     ));
                // };
                // Ok(format!("{source}/{kind}/{folder}/{name}"))
            }
            Self::Plz { source } => Ok(if let Some(version) = version {
                format!("{source}/packages/metadata/{name}?v={version}")
            } else {
                format!("{source}/packages/metadata/{name}")
            }),
            Self::Github { .. } => Err(StatefulError::new("Debug breakpoint!", &cause)),
        }
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        match self {
            Self::Apt {
                uri,
                suite,
                component,
                signature_path,
            } => {
                bytes.push(0);
                bytes.extend_from_slice(uri.as_bytes());
                bytes.push(b'\n');
                bytes.push(suite.len() as u8);
                bytes.extend_from_slice(suite.as_bytes());
                let component = component.to_string();
                bytes.push(component.len() as u8);
                bytes.extend_from_slice(component.as_bytes());
                bytes.extend_from_slice(signature_path.as_os_str().as_bytes());
            }
            Self::Github { user, repo } => {
                bytes.push(1);
                bytes.extend_from_slice(user.as_bytes());
                bytes.push(b'\n');
                bytes.extend_from_slice(repo.as_bytes());
            }
            Self::Plz { source } => {
                bytes.push(2);
                bytes.extend_from_slice(source.as_bytes());
            }
        }
        bytes
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WrappedError> {
        let Some((ident, remaining)) = bytes.split_first() else {
            return Err(WrappedError::Other {
                error: String::from("Missing enum identifier!"),
            });
        };
        match ident {
            0 => {
                let Some((uri, remaining)) = remaining
                    .iter()
                    .position(|x| *x == b'\n')
                    .map(|i| (&remaining[..i], &remaining[i..]))
                else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing OriginKind identifier!"),
                    });
                };
                let Some((len, remaining)) = remaining.split_first() else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing `suite` length!"),
                    });
                };
                let Some((suite, remaining)) = remaining.split_at_checked(*len as usize) else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing data after `suite` field!"),
                    });
                };
                let Some((len, remaining)) = remaining.split_first() else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing `component` length!"),
                    });
                };
                let Some((component, signature_path)) = remaining.split_at_checked(*len as usize)
                else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing data after `component` field!"),
                    });
                };
                Ok(Self::Apt {
                    uri: String::from_utf8_lossy(uri).into(),
                    suite: String::from_utf8_lossy(suite).into(),
                    component: AptComponent::from_bytes(component),
                    signature_path: PathBuf::from(OsStr::from_bytes(signature_path)),
                })
            }
            1 => {
                let Some((user, repo)) = remaining
                    .iter()
                    .position(|x| *x == b'\n')
                    .map(|i| (&remaining[..i], &remaining[i..]))
                else {
                    return Err(WrappedError::Other {
                        error: String::from("Missing OriginKind identifier!"),
                    });
                };
                Ok(Self::Github {
                    user: String::from_utf8_lossy(user).into(),
                    repo: String::from_utf8_lossy(repo).into(),
                })
            }
            2 => Ok(Self::Plz {
                source: String::from_utf8_lossy(remaining).into(),
            }),
            x => Err(WrappedError::Other {
                error: format!("Unknown identifier {x:?}!"),
            }),
        }
    }
}

impl Type<Sqlite> for OriginKind {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'a> Encode<'a, Sqlite> for OriginKind {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        <Vec<u8> as Encode<'_, Sqlite>>::encode_by_ref(&self.as_bytes(), buf)
    }
    fn encode(
        self,
        buf: &mut <Sqlite as sqlx::Database>::ArgumentBuffer<'a>,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError>
    where
        Self: Sized,
    {
        <Vec<u8> as Encode<'_, Sqlite>>::encode(self.as_bytes(), buf)
    }
}

impl<'a> Decode<'a, Sqlite> for OriginKind {
    fn decode(
        value: <Sqlite as sqlx::Database>::ValueRef<'a>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let data: &[u8] = Decode::<Sqlite>::decode(value)?;
        Ok(Self::from_bytes(data)
            .wrap(&json!({"action": "decoding APT origins from settings"}))
            .map_err(|e| {
                OtherSnafu {
                    error: e.to_string(),
                }
                .build()
            })?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum AptComponent {
    Custom(String),
    Main,
    Multiverse,
    Restricted,
    Universe,
}

impl AptComponent {
    fn from_bytes(bytes: &[u8]) -> Self {
        let string = String::from_utf8_lossy(bytes);
        let str: &str = &string;
        match str {
            "main" => Self::Main,
            "multiverse" => Self::Multiverse,
            "restricted" => Self::Restricted,
            "universe" => Self::Universe,
            _ => Self::Custom(string.into()),
        }
    }
}

impl Display for AptComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Custom(c) => c,
            Self::Main => "main",
            Self::Multiverse => "multiverse",
            Self::Restricted => "restricted",
            Self::Universe => "universe",
        })
    }
}
