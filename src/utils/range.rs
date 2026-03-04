use serde::{Deserialize, Serialize};
use snafu::OptionExt;
use std::fmt::Display;

use crate::errors::{OtherSnafu, Wrapped, WrappedError};
use crate::utils::verreq::VerReq;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Range {
    pub lower: VerReq,
    pub upper: VerReq,
}

impl Range {
    pub fn is_sane(&self) -> bool {
        match &self.lower {
            VerReq::Gt(gt) => match &self.upper {
                VerReq::Eq(o_eq) => gt == o_eq,
                VerReq::Le(o) | VerReq::Lt(o) => gt < o,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::Ge(ge) => match &self.upper {
                VerReq::Eq(o_eq) => ge == o_eq,
                VerReq::Le(o_le) => ge <= o_le,
                VerReq::Lt(o_lt) => ge < o_lt,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::Eq(eq) => match &self.upper {
                VerReq::Eq(o_eq) => eq == o_eq,
                VerReq::NoBound => true,
                _ => false,
            },
            VerReq::NoBound => true,
            _ => false,
        }
    }
    pub fn negotiate(&self, prior: Option<Self>) -> Option<Self> {
        self.upper.negotiate(self.lower.negotiate(prior))
    }
    pub fn parse(input: &str) -> Result<Self, WrappedError> {
        let (lower, upper) = input.split_once(' ').context(OtherSnafu {
            error: "Missing Range field `upper`!",
        })?;
        let lower = VerReq::parse(lower).wrap()?;
        let upper = VerReq::parse(upper).wrap()?;
        Ok(Self { lower, upper })
    }
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} {}", self.lower, self.upper))
    }
}
