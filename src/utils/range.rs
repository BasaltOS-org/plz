use serde::{Deserialize, Serialize};
use serde_json::json;
use snafu::OptionExt;
use std::fmt::Display;

use crate::errors::{OtherSnafu, StatefulError, Wrapped};
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
    pub fn parse(input: &str) -> Result<Self, StatefulError> {
        let mut cause = json!({"action": "parsing bytes to Range"});
        let (lower, upper) = input
            .split_once(' ')
            .context(OtherSnafu {
                error: "Missing Range field `upper`!",
            })
            .wrap(&cause)?;
        let lower = VerReq::parse(lower).wrap({
            cause
                .as_object_mut()
                .map(|x| x.insert(String::from("constraint"), json!("lower")));
            &cause
        })?;
        let upper = VerReq::parse(upper).wrap({
            cause
                .as_object_mut()
                .map(|x| x.insert(String::from("constraint"), json!("upper")));
            &cause
        })?;
        Ok(Self { lower, upper })
    }
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{} {}", self.lower, self.upper))
    }
}
