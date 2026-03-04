use serde::{Deserialize, Serialize};
use snafu::{OptionExt, location};
use std::fmt::Display;

use crate::errors::{OtherSnafu, Wrapped, WrappedError};
use crate::utils::{range::Range, version::Version};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum VerReq {
    Gt(Version),
    Ge(Version),
    Eq(Version),
    Le(Version),
    Lt(Version),
    NoBound,
}

impl VerReq {
    pub fn negotiate(&self, prior: Option<Range>) -> Option<Range> {
        let prior = if let Some(mut prior) = prior {
            match self {
                Self::Gt(gt) => match &prior.lower {
                    Self::Gt(p_gt) => {
                        if gt > p_gt {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Ge(p_ge) => {
                        if gt >= p_ge {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::NoBound => {
                        prior.lower = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Ge(ge) => match &prior.lower {
                    Self::Gt(p_gt) => {
                        if ge > p_gt {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Ge(p_ge) => {
                        if ge > p_ge {
                            prior.lower = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Eq(p_eq) => {
                        if ge == p_eq {
                            Some(prior)
                        } else {
                            None
                        }
                    }
                    Self::NoBound => {
                        prior.lower = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Eq(eq) => {
                    match &prior.lower {
                        Self::Gt(p_gt) if eq > p_gt => {
                            prior.lower = self.clone();
                        }
                        Self::Ge(p_ge) if eq >= p_ge => {
                            prior.lower = self.clone();
                        }
                        Self::Eq(p_eq) => {
                            if eq != p_eq {
                                return None;
                            }
                        }
                        Self::NoBound => {
                            prior.lower = self.clone();
                        }
                        _ => return None,
                    }
                    match &prior.upper {
                        Self::Eq(p_eq) => {
                            if eq != p_eq {
                                return None;
                            }
                        }
                        Self::Le(p_le) if eq <= p_le => {
                            prior.upper = self.clone();
                        }
                        Self::Lt(p_lt) if eq < p_lt => {
                            prior.upper = self.clone();
                        }
                        Self::NoBound => {
                            prior.upper = self.clone();
                        }
                        _ => return None,
                    }
                    Some(prior)
                }
                Self::Le(le) => match &prior.upper {
                    Self::Lt(p_lt) => {
                        if le < p_lt {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Le(p_le) => {
                        if le < p_le {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Eq(p_eq) => {
                        if le == p_eq {
                            Some(prior)
                        } else {
                            None
                        }
                    }
                    Self::NoBound => {
                        prior.upper = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::Lt(lt) => match &prior.upper {
                    Self::Lt(p_lt) => {
                        if lt < p_lt {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::Le(p_le) => {
                        if lt <= p_le {
                            prior.upper = self.clone();
                        }
                        Some(prior)
                    }
                    Self::NoBound => {
                        prior.upper = self.clone();
                        Some(prior)
                    }
                    _ => None,
                },
                Self::NoBound => Some(prior),
            }
        } else {
            None
        };
        if prior.as_ref().is_some_and(|x| x.is_sane()) {
            prior
        } else {
            None
        }
    }
    pub fn parse(input: &str) -> Result<Self, WrappedError> {
        let mut chars = input.chars();
        let kind = chars.next().context(OtherSnafu {
            error: "Missing type identifier!",
        })?;
        let data = chars.collect::<String>();
        match kind as u8 {
            0 => Ok(Self::NoBound),
            kind => {
                let version = Version::parse(&data).wrap()?;
                match kind {
                    1 => Ok(Self::Gt(version)),
                    2 => Ok(Self::Ge(version)),
                    3 => Ok(Self::Eq(version)),
                    4 => Ok(Self::Le(version)),
                    5 => Ok(Self::Lt(version)),
                    kind => Err(WrappedError::Other {
                        error: format!("Invalid kind identifier `{kind}`!").into(),
                        loc: location!(),
                    }),
                }
            }
        }
    }
}

impl Display for VerReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            Self::Gt(gt) => format!("\x01{gt}"),
            Self::Ge(ge) => format!("\x02{ge}"),
            Self::Eq(eq) => format!("\x03{eq}"),
            Self::Le(le) => format!("\x04{le}"),
            Self::Lt(lt) => format!("\x05{lt}"),
            Self::NoBound => String::from("\x00"),
        })
    }
}
