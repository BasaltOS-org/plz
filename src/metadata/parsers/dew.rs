use serde::Deserialize;
use snafu::location;

use crate::errors::{Wrapped, WrappedError};
use crate::metadata::{
    DepVer, DependKind, depend_kind,
    parsers::MetaDataKind,
    processed::{ProcessedCompilable, ProcessedInstallKind, ProcessedMetaData},
};
use crate::settings::OriginKind;
use crate::utils::{range::Range, verreq::VerReq, version::Version};
#[derive(Debug, Deserialize)]
pub struct RawDew {
    name: String,
    description: String,
    version: String,
    origin: String,
    build_dependencies: Vec<String>,
    runtime_dependencies: Vec<String>,
    build: String,
    install: String,
    uninstall: String,
    purge: String,
    hash: String,
}

impl RawDew {
    pub fn to_process(self, dependent: bool) -> Result<ProcessedMetaData, WrappedError> {
        let origin = if self.origin.starts_with("gh/") {
            let split = self
                .origin
                .split('/')
                .skip(1)
                .map(|x| x.to_string())
                .collect::<Vec<String>>();
            if split.len() == 2 {
                OriginKind::Github {
                    user: split[0].clone(),
                    repo: split[1].clone(),
                }
            } else {
                return Err(WrappedError::Other {
                    error: "Invalid `origin` format!".into(),
                    loc: location!(),
                });
            }
        // } else if self.origin.starts_with("https://") {
        //     OriginKind::Url(self.origin.clone())
        // } else {
        //     return None;
        // };
        } else {
            OriginKind::Dew(self.origin.clone())
        };
        let build_dependencies =
            depend_kind::DependKindVec(Self::as_dep_kind(&self.build_dependencies)?);
        let runtime_dependencies =
            depend_kind::DependKindVec(Self::as_dep_kind(&self.runtime_dependencies)?);
        Ok(ProcessedMetaData {
            name: self.name,
            kind: MetaDataKind::Dew,
            description: self.description,
            version: self.version,
            origin,
            dependent,
            build_dependencies,
            runtime_dependencies,
            install_kind: ProcessedInstallKind::Compilable(ProcessedCompilable {
                build: self.build,
                install: self.install,
                uninstall: self.uninstall,
                purge: self.purge,
            }),
            hash: self.hash,
        })
    }
    fn parse_ver(ver: &str) -> Result<Range, WrappedError> {
        let mut lower = VerReq::NoBound;
        let mut upper = VerReq::NoBound;
        if let Some(ver) = ver.strip_prefix(">>") {
            lower = VerReq::Gt(Version::parse(ver).wrap()?);
        } else if let Some(ver) = ver.strip_prefix(">=") {
            lower = VerReq::Ge(Version::parse(ver).wrap()?);
        } else if let Some(ver) = ver.strip_prefix("==") {
            lower = VerReq::Eq(Version::parse(ver).wrap()?);
            upper = VerReq::Eq(Version::parse(ver).wrap()?);
        } else if let Some(ver) = ver.strip_prefix("<=") {
            upper = VerReq::Le(Version::parse(ver).wrap()?);
        } else if let Some(ver) = ver.strip_prefix("<<") {
            upper = VerReq::Lt(Version::parse(ver).wrap()?);
        } else {
            lower = VerReq::Eq(Version::parse(ver).wrap()?);
            upper = VerReq::Eq(Version::parse(ver).wrap()?);
        };
        // Yeah this needs to be done properly, so.....
        // thingy
        Ok(Range { lower, upper })
    }
    fn as_dep_kind(deps: &[String]) -> Result<Vec<DependKind>, WrappedError> {
        let mut result = Vec::new();
        for dep in deps {
            let val = if let Some(dep) = dep.strip_prefix('!') {
                DependKind::Volatile(dep.to_string())
            // } else if let Some((name, ver)) = dep.split_once(':') {
            //     DependKind::Specific(DepVer {
            //         name: name.to_string(),
            //         range: RawDew::parse_ver(ver)?,
            //     })
            } else if let Some(index) = dep.find(['=', '>', '<']) {
                let (name, ver) = dep.split_at(index);
                DependKind::Specific(DepVer {
                    name: name.to_string(),
                    range: RawDew::parse_ver(ver).wrap()?,
                })
            } else {
                DependKind::Latest(dep.to_string())
            };
            result.push(val);
        }
        Ok(result)
    }
}
