use snafu::{Location, Snafu};
use std::{borrow::Cow, fmt::Debug};

// #[derive(Debug, Snafu)]
// #[snafu(visibility(pub))]
// pub enum WrappedError {
//     #[snafu(display("Emancipate -> {source}"))]
//     Emancipate { source: WrappedError },
//     #[snafu(display("Init -> {source}"))]
//     Init { source: WrappedError },
//     #[snafu(display("Install -> {source}"))]
//     Install { source: WrappedError },
//     #[snafu(display("Remove/Purge -> {source}"))]
//     Remove { source: WrappedError },
//     #[snafu(display("Update -> {source}"))]
//     Update { source: WrappedError },
//     #[snafu(display("Upgrade -> {source}"))]
//     Upgrade { source: WrappedError },
// }

// #[derive(Debug, Snafu)]
// #[snafu(visibility(pub))]
// pub enum WrappedError {
//     #[snafu(display("{loc} -> {source}"))]
//     NestedError { source: WrappedError, loc: MyStr },
//     #[snafu(display("{loc} -> {source}"))]
//     BoxedError { source: Box<WrappedError>, loc: MyStr },
//     #[snafu(display("{source}"))]
//     WrappedError { source: WrappedError },
// }
// impl WrappedError {
//     pub fn debug(location: snafu::Location) -> Self {
//         Self::WrappedError {
//             source: WrappedError::DebugError { location },
//         }
//     }
//     pub fn other<T: Into<MyStr>>(message: T) -> Self {
//         Self::WrappedError {
//             source: WrappedError::OtherError {
//                 message: message.into(),
//             },
//         }
//     }
// }

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum WrappedError {
    // #[snafu(display(
    //     "This error should not be seen on production builds. For reference, it was called here: {location:?}"
    // ))]
    // DebugError { location: snafu::Location },
    // #[snafu(display("Failed to {action:?} for `{loc}`! (source)"))]
    // IOError {
    //     source: std::io::Error,
    //     action: IOAction,
    //     loc: MyStr,
    // },
    // #[snafu(display("Failed to pull data situated at `{loc}`! ({source})"))]
    // NetError { loc: MyStr, source: reqwest::Error },
    // #[snafu(display("{message}"))]
    // OtherError { message: MyStr },
    // #[snafu(display("Parser `{util:?}` failed with error `{message}`!"))]
    // ParseError { message: MyStr, util: Parsers },
    // #[snafu(display("Error creating runtime! ({source})"))]
    // RuntimeError { source: tokio::io::Error },
    // #[snafu(display("{message} for package `{package}`!"))]
    // SystemError { message: MyStr, package: MyStr },
    #[snafu(display("{loc} -> {source}"))]
    Nested {
        source: Box<WrappedError>,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{msg}: {loc} -> {source}"))]
    Wrapped {
        source: Box<WrappedError>,
        msg: MyStr,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{error}@{loc}"))]
    Other {
        error: Cow<'static, str>,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{source}@{loc}"))]
    JSON {
        source: serde_json::Error,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{source}@{loc}"))]
    Net {
        source: reqwest::Error,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{source}@{loc}"))]
    StdIO {
        source: std::io::Error,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{source}@{loc}"))]
    SQL {
        source: sqlx::Error,
        #[snafu(implicit)]
        loc: Location,
    },
    #[snafu(display("{source}@{loc}"))]
    TokioIO {
        source: tokio::io::Error,
        #[snafu(implicit)]
        loc: Location,
    },
}

pub trait Wrapped<T> {
    #[track_caller]
    fn wrap(self) -> Result<T, WrappedError>;
    #[track_caller]
    fn wrap_with(self, msg: MyStr) -> Result<T, WrappedError>;
}

impl<T> Wrapped<T> for Result<T, WrappedError> {
    #[track_caller]
    fn wrap(self) -> Result<T, WrappedError> {
        self.map_err(|e| WrappedError::Nested {
            source: Box::new(e),
            loc: std::panic::Location::caller(),
        })
    }
    #[track_caller]
    fn wrap_with(self, msg: MyStr) -> Result<T, WrappedError> {
        self.map_err(|e| WrappedError::Wrapped {
            source: Box::new(e),
            msg,
            loc: std::panic::Location::caller(),
        })
    }
}

type MyStr = Cow<'static, str>;

// pub enum IOAction {
//     TermRead,
//     TermStatus,
//     CreateFile,
//     OpenFile,
//     ReadFile,
//     WriteFile,
//     CorruptedFile,
//     RemoveFile,
//     CreateDir,
//     ReadDir,
//     AssertPath,
// }
// impl Debug for IOAction {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str(match self {
//             Self::TermRead => "read input from terminal",
//             Self::TermStatus => "read process exit status",
//             Self::CreateFile => "create file",
//             Self::OpenFile => "open file",
//             Self::ReadFile => "read file",
//             Self::CorruptedFile => "read corrupted file",
//             Self::WriteFile => "write to file",
//             Self::RemoveFile => "remove file",
//             Self::CreateDir => "create directory",
//             Self::ReadDir => "read directory",
//             Self::AssertPath => "assert path",
//         })
//     }
// }

// pub enum Parsers {
//     DependKind,
//     DepVer,
//     InstalledCompilable,
//     InstalledInstallKind,
//     MetaDataKind,
//     OriginKind,
//     PreBuilt,
//     ProcessedCompilable,
//     ProcessedInstallKind,
//     Range,
//     Specific,
//     Version,
//     VerReq,
// }
// impl Debug for Parsers {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str(match self {
//             Self::DependKind => "DependKind",
//             Self::DepVer => "DepVer",
//             Self::InstalledCompilable => "InstalledCompilable",
//             Self::InstalledInstallKind => "InstalledInstallKind",
//             Self::MetaDataKind => "MetaDataKind",
//             Self::OriginKind => "OriginKind",
//             Self::PreBuilt => "PreBuilt",
//             Self::ProcessedCompilable => "ProcessedCompilable",
//             Self::ProcessedInstallKind => "ProcessedInstallKind",
//             Self::Range => "Range",
//             Self::Specific => "Specific",
//             Self::Version => "Version",
//             Self::VerReq => "VerReq",
//         })
//     }
// }
