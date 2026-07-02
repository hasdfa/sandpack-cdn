use thiserror::Error;
use tokio::sync::broadcast;
use warp::{hyper::http, reject};

pub type AppResult<T> = Result<T, ServerError>;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("invalid semver")]
    InvalidSemver(#[from] node_semver::SemverError),
    #[error("Failed request")]
    FailedRequest(#[from] reqwest_middleware::Error),
    #[error(transparent)]
    RequestFailed(#[from] reqwest::Error),
    #[error("Response has a non-200 status code")]
    RequestErrorStatus { status_code: u16 },
    #[error("IO Operation failed")]
    IoError(#[from] std::io::Error),
    #[error("Could not parse url")]
    UrlParseError(#[from] url::ParseError),
    #[error("Could not parse json string")]
    JSONParseError(#[from] serde_json::Error),
    #[error("Package version not found {0}@{1}")]
    PackageVersionNotFound(String, String),
    #[error("Package {0} not found")]
    PackageNotFound(String),
    #[error("Infallible error")]
    Infallible(#[from] std::convert::Infallible),
    #[error("Could not parse module")]
    SWCParseError { message: String },
    #[error("Could not download tarball package")]
    TarballDownloadError { status_code: u16, url: String },
    #[error("Could not download package metadata")]
    PackageMetadataDownloadError { status_code: u16, url: String },
    #[error("Could not download npm package manifest")]
    NpmManifestDownloadError {
        status_code: u16,
        package_name: String,
    },
    #[error("Invalid package specifier")]
    InvalidPackageSpecifier,
    #[error("Invalid byte buffer")]
    InvalidString(#[from] std::str::Utf8Error),
    #[error("Join error")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("Invalid CDN version")]
    InvalidCDNVersion,
    #[error("Could not parse integer")]
    IntegerParse(#[from] std::num::ParseIntError),
    #[error("Invalid status code")]
    InvalidStatusCode(#[from] http::status::InvalidStatusCode),
    #[error("Failed to serialize to msgpack")]
    SerializeError(),
    #[error("Failed to deserialize from msgpack")]
    DeserializeError(),
    #[error("Failed to decode base64 string")]
    Base64DecodingError(),
    #[error("Sendable error")]
    SendableError(#[from] SendableError),
    #[error("Resource hasn't changed")]
    NotChanged,
    #[error("Invalid query")]
    InvalidQuery,
    #[error("Unexpected error")]
    UnexpectedError { message: String },
    #[error("MessagePack Decode Error")]
    MessagePackDecodeError(#[from] rmp_serde::decode::Error),
}

impl ServerError {
    /// HTTP status this error should surface as. Client mistakes are 4xx,
    /// upstream registry failures are 502, everything else is a plain 500.
    /// Note: tarball errors coalesced through `Cached` are stringified into
    /// `SendableError` and land in the default 500 arm.
    pub fn status_code(&self) -> u16 {
        match self {
            ServerError::PackageNotFound(_) | ServerError::PackageVersionNotFound(_, _) => 404,
            // The npm registry says the package doesn't exist: a not-found,
            // not an upstream failure.
            ServerError::PackageMetadataDownloadError {
                status_code: 404, ..
            } => 404,
            ServerError::InvalidPackageSpecifier
            | ServerError::InvalidSemver(_)
            | ServerError::Base64DecodingError()
            | ServerError::InvalidQuery
            | ServerError::InvalidCDNVersion
            | ServerError::IntegerParse(_) => 400,
            ServerError::TarballDownloadError { .. }
            | ServerError::PackageMetadataDownloadError { .. }
            | ServerError::NpmManifestDownloadError { .. }
            | ServerError::FailedRequest(_)
            | ServerError::RequestFailed(_)
            | ServerError::RequestErrorStatus { .. } => 502,
            _ => 500,
        }
    }
}

impl From<ServerError> for std::io::Error {
    fn from(err: ServerError) -> Self {
        std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", err))
    }
}

impl reject::Reject for ServerError {}

#[derive(Debug, Clone, thiserror::Error)]
#[error("stringified error: {inner}")]
pub struct SendableError {
    pub inner: String,
}

impl SendableError {
    pub fn new<E: std::fmt::Display>(e: E) -> Self {
        Self {
            inner: e.to_string(),
        }
    }
}

impl From<broadcast::error::RecvError> for SendableError {
    fn from(e: broadcast::error::RecvError) -> Self {
        SendableError::new(e)
    }
}

impl From<ServerError> for SendableError {
    fn from(e: ServerError) -> Self {
        SendableError::new(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_errors_are_404() {
        assert_eq!(
            ServerError::PackageNotFound("left-pad".to_string()).status_code(),
            404
        );
        assert_eq!(
            ServerError::PackageVersionNotFound("a".to_string(), "1.2.3".to_string())
                .status_code(),
            404
        );
        // npm itself says 404: the package doesn't exist upstream.
        assert_eq!(
            ServerError::PackageMetadataDownloadError {
                status_code: 404,
                url: "https://registry.npmjs.org/nope".to_string(),
            }
            .status_code(),
            404
        );
    }

    #[test]
    fn client_errors_are_400() {
        assert_eq!(ServerError::InvalidPackageSpecifier.status_code(), 400);
        assert_eq!(ServerError::InvalidQuery.status_code(), 400);
        assert_eq!(ServerError::InvalidCDNVersion.status_code(), 400);
        assert_eq!(ServerError::Base64DecodingError().status_code(), 400);
        assert_eq!(
            ServerError::from(node_semver::Version::parse("x").unwrap_err()).status_code(),
            400
        );
        assert_eq!(
            ServerError::from("x".parse::<u64>().unwrap_err()).status_code(),
            400
        );
    }

    #[test]
    fn upstream_errors_are_502() {
        assert_eq!(
            ServerError::TarballDownloadError {
                status_code: 503,
                url: "https://registry.npmjs.org/react".to_string(),
            }
            .status_code(),
            502
        );
        assert_eq!(
            ServerError::PackageMetadataDownloadError {
                status_code: 500,
                url: "https://registry.npmjs.org/react".to_string(),
            }
            .status_code(),
            502
        );
        assert_eq!(
            ServerError::NpmManifestDownloadError {
                status_code: 500,
                package_name: "react".to_string(),
            }
            .status_code(),
            502
        );
        assert_eq!(
            ServerError::RequestErrorStatus { status_code: 503 }.status_code(),
            502
        );
    }

    #[test]
    fn internal_errors_are_500() {
        assert_eq!(ServerError::SerializeError().status_code(), 500);
        assert_eq!(
            ServerError::UnexpectedError {
                message: "boom".to_string(),
            }
            .status_code(),
            500
        );
    }
}
