use snafu::Snafu;
use std::{borrow::Cow, fmt::Debug};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum WhatError {
    #[snafu(display("Emancipate -> {source}"))]
    Emancipate { source: WhereError },
    #[snafu(display("Init -> {source}"))]
    Init { source: WhereError },
    #[snafu(display("Install -> {source}"))]
    Install { source: WhereError },
    #[snafu(display("Remove/Purge -> {source}"))]
    Remove { source: WhereError },
    #[snafu(display("Update -> {source}"))]
    Update { source: WhereError },
    #[snafu(display("Upgrade -> {source}"))]
    Upgrade { source: WhereError },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum WhereError {
    #[snafu(display("{loc} -> {source}"))]
    NestedError { source: HowError, loc: MyStr },
    #[snafu(display("{loc} -> {source}"))]
    BoxedError { source: Box<WhereError>, loc: MyStr },
    #[snafu(display("{source}"))]
    WrappedError { source: HowError },
}
impl WhereError {
    pub fn debug(location: snafu::Location) -> Self {
        Self::WrappedError {
            source: HowError::DebugError { location },
        }
    }
    pub fn other<T: Into<MyStr>>(message: T) -> Self {
        Self::WrappedError {
            source: HowError::OtherError {
                message: message.into(),
            },
        }
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum HowError {
    #[snafu(display(
        "This error should not be seen on production builds. For reference, it was called here: {location:?}"
    ))]
    DebugError { location: snafu::Location },
    #[snafu(display("Failed to {action:?} for `{loc}`! (source)"))]
    IOError {
        source: std::io::Error,
        action: IOAction,
        loc: MyStr,
    },
    #[snafu(display("Failed to pull data situated at `{loc}`! ({source})"))]
    NetError { loc: MyStr, source: reqwest::Error },
    #[snafu(display("{message}"))]
    OtherError { message: MyStr },
    #[snafu(display("Parser `{util:?}` failed with error `{message}`!"))]
    ParseError { message: MyStr, util: Parsers },
    #[snafu(display("Error creating runtime! ({source})"))]
    RuntimeError { source: tokio::io::Error },
    #[snafu(display("{message} for package `{package}`!"))]
    SystemError { message: MyStr, package: MyStr },
    #[snafu(display("SQL Error! ({source})"))]
    SQLError { source: sqlx::Error },
    #[snafu(display("Deserialization failed for {loc}! ({source})"))]
    JSONError {
        #[snafu(implicit)]
        source: serde_json::Error,
        loc: MyStr,
    },
}

type MyStr = Cow<'static, str>;

pub enum IOAction {
    TermRead,
    TermStatus,
    CreateFile,
    OpenFile,
    ReadFile,
    WriteFile,
    CorruptedFile,
    RemoveFile,
    CreateDir,
    ReadDir,
    AssertPath,
}
impl Debug for IOAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TermRead => "read input from terminal",
            Self::TermStatus => "read process exit status",
            Self::CreateFile => "create file",
            Self::OpenFile => "open file",
            Self::ReadFile => "read file",
            Self::CorruptedFile => "read corrupted file",
            Self::WriteFile => "write to file",
            Self::RemoveFile => "remove file",
            Self::CreateDir => "create directory",
            Self::ReadDir => "read directory",
            Self::AssertPath => "assert path",
        })
    }
}

pub enum Parsers {
    DependKind,
    DepVer,
    InstalledCompilable,
    InstalledInstallKind,
    MetaDataKind,
    OriginKind,
    PreBuilt,
    ProcessedCompilable,
    ProcessedInstallKind,
    Range,
    Specific,
    Version,
    VerReq,
}
impl Debug for Parsers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DependKind => "DependKind",
            Self::DepVer => "DepVer",
            Self::InstalledCompilable => "InstalledCompilable",
            Self::InstalledInstallKind => "InstalledInstallKind",
            Self::MetaDataKind => "MetaDataKind",
            Self::OriginKind => "OriginKind",
            Self::PreBuilt => "PreBuilt",
            Self::ProcessedCompilable => "ProcessedCompilable",
            Self::ProcessedInstallKind => "ProcessedInstallKind",
            Self::Range => "Range",
            Self::Specific => "Specific",
            Self::Version => "Version",
            Self::VerReq => "VerReq",
        })
    }
}
