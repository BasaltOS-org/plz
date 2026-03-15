use serde_json::Value;
use snafu::Snafu;
use std::fmt::{Debug, Display};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum WrappedError {
    #[snafu(display("{error}"))]
    Other { error: String },
    #[snafu(display("{source}"))]
    JSON { source: serde_json::Error },
    #[snafu(display("{source}"))]
    Net { source: reqwest::Error },
    #[snafu(display("{source}"))]
    StdIO { source: std::io::Error },
    #[snafu(display("{source}"))]
    SQL { source: sqlx::Error },
    #[snafu(display("{source}"))]
    TokioIO { source: tokio::io::Error },
}
impl PartialEq for StatefulError {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

pub struct StatefulError {
    error: WrappedError,
    data: Option<Value>,
}

impl StatefulError {
    pub fn new(error: impl ToString) -> Self {
        Self {
            error: WrappedError::Other {
                error: error.to_string(),
            },
            data: None,
        }
    }
}

impl Display for StatefulError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(data) = &self.data {
            f.write_fmt(format_args!("{}: {}", self.error, data))
        } else {
            f.write_fmt(format_args!("{}", self.error))
        }
    }
}

pub trait WrappedWith<T> {
    #[track_caller]
    fn wrap(self, cause: &Value) -> Result<T, StatefulError>;
}
pub trait Wrapped<T> {
    #[track_caller]
    fn wrap(self) -> Result<T, StatefulError>;
}

impl<T> WrappedWith<T> for Result<T, StatefulError> {
    fn wrap(self, cause: &Value) -> Result<T, StatefulError> {
        self.map_err(|e| StatefulError {
            error: e.error,
            data: {
                let mut cause = cause.clone();
                Some(if let Some(e_data) = e.data {
                    cause
                        .as_object_mut()
                        .map(|x| x.insert(String::from("caused_by"), e_data));
                    cause
                } else {
                    cause
                })
            },
        })
    }
}

impl<T> Wrapped<T> for Result<T, WrappedError> {
    fn wrap(self) -> Result<T, StatefulError> {
        self.map_err(|e| StatefulError {
            error: e,
            data: None,
        })
    }
}
