use serde::{Deserialize, Serialize};
use snafu::location;
use std::cmp::Ordering;

use crate::errors::WrappedError;

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    pub pre: String,
    pub build: Option<String>,
}

impl Version {
    pub fn parse(src: &str) -> Result<Self, WrappedError> {
        let (src, build) = src
            .split_once('+')
            .map(|x| (x.0, Some(x.1.to_string())))
            .unwrap_or((src, None));
        let (src, pre) = src
            .split_once('-')
            .map(|x| (x.0, x.1.to_string()))
            .unwrap_or_else(|| (src, String::new()));
        let split = src.split('.').collect::<Vec<&str>>();
        if !split.is_empty() {
            if let Ok(major) = split[0].parse::<usize>() {
                if split.len() >= 2 {
                    if let Ok(minor) = split[1].parse::<usize>() {
                        if split.len() >= 3 {
                            if let Ok(patch) = split[2].parse::<usize>() {
                                if split.len() > 3 {
                                    Err(WrappedError::Other {
                                        error: "Two many segments in version!".into(),
                                        loc: location!(),
                                    })
                                } else {
                                    Ok(Self {
                                        major,
                                        minor,
                                        patch,
                                        pre,
                                        build,
                                    })
                                }
                            } else {
                                Err(WrappedError::Other {
                                    error: format!(
                                        "Expected patch to be a number, got `{}`!",
                                        split[1]
                                    )
                                    .into(),
                                    loc: location!(),
                                })
                            }
                        } else {
                            Ok(Self {
                                major,
                                minor,
                                patch: 0,
                                pre,
                                build,
                            })
                        }
                    } else {
                        Err(WrappedError::Other {
                            error: format!("Expected minor to be a number, got `{}`!", split[1])
                                .into(),
                            loc: location!(),
                        })
                    }
                } else {
                    Ok(Self {
                        major,
                        minor: 0,
                        patch: 0,
                        pre,
                        build,
                    })
                }
            } else {
                Err(WrappedError::Other {
                    error: format!("Expected major to be a number, got `{}`!", split[0]).into(),
                    loc: location!(),
                })
            }
        } else {
            Err(WrappedError::Other {
                error: "A version must be specified!".into(),
                loc: location!(),
            })
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut tail = if self.pre.is_empty() {
            String::new()
        } else {
            format!("-{}", self.pre)
        };
        if let Some(build) = &self.build {
            tail.push_str(&format!("+{}", build));
        }
        f.write_str(&format!(
            "{}.{}.{}{}",
            self.major, self.minor, self.patch, tail
        ))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => match self.minor.cmp(&other.minor) {
                Ordering::Equal => match self.patch.cmp(&other.patch) {
                    Ordering::Equal => match self.pre.cmp(&other.pre) {
                        Ordering::Equal => self.build.cmp(&other.build),
                        order => order,
                    },
                    order => order,
                },
                order => order,
            },
            order => order,
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
