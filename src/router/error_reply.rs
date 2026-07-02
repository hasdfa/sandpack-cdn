use serde::{self, Deserialize, Serialize};
use warp::hyper::StatusCode;

use crate::app_error::ServerError;

use super::custom_reply::CustomReply;

#[derive(Clone, Serialize, Deserialize)]
pub struct ErrorReply {
    status: u16,
    message: String,
    details: String,
}

impl ErrorReply {
    pub fn new(status: u16, message: String, details: String) -> Self {
        ErrorReply {
            status,
            message,
            details,
        }
    }

    pub fn as_reply(&self) -> CustomReply {
        // Serializing this plain struct cannot fail today, but never panic on
        // the request path: fall back to a static body just in case.
        let mut reply = CustomReply::json(self).unwrap_or_else(|_| {
            CustomReply::raw_json(
                br#"{"status":500,"message":"Internal error","details":"failed to serialize error reply"}"#
                    .to_vec(),
            )
        });
        reply.set_status(
            StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        );
        if self.status < 500 {
            // Client errors (including 404s) are stable answers: let CDNs
            // cache them briefly.
            reply.add_header("Cache-Control", "public, max-age=300");
            reply.add_header("CDN-Cache-Control", "max-age=300");
        } else {
            // Server/upstream errors are transient: never cache them.
            reply.add_header("Cache-Control", "no-store");
            reply.add_header("CDN-Cache-Control", "no-store");
        }
        reply
    }
}

impl From<ServerError> for ErrorReply {
    fn from(err: ServerError) -> Self {
        ErrorReply::new(err.status_code(), format!("{}", err), format!("{:?}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use warp::Reply;

    fn header<'r>(response: &'r warp::reply::Response, name: &str) -> &'r str {
        response.headers().get(name).unwrap().to_str().unwrap()
    }

    #[test]
    fn client_errors_are_cacheable() {
        let reply =
            ErrorReply::from(ServerError::PackageNotFound("left-pad".to_string())).as_reply();
        let response = reply.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(header(&response, "Cache-Control"), "public, max-age=300");
        assert_eq!(header(&response, "CDN-Cache-Control"), "max-age=300");
    }

    #[test]
    fn server_errors_are_not_cached() {
        let reply = ErrorReply::from(ServerError::RequestErrorStatus { status_code: 503 })
            .as_reply();
        let response = reply.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(header(&response, "Cache-Control"), "no-store");
        assert_eq!(header(&response, "CDN-Cache-Control"), "no-store");

        let reply = ErrorReply::from(ServerError::SerializeError()).as_reply();
        let response = reply.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(header(&response, "Cache-Control"), "no-store");
    }

    #[test]
    fn invalid_status_falls_back_to_500() {
        let reply = ErrorReply::new(0, "bad".to_string(), "bad".to_string()).as_reply();
        let response = reply.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
