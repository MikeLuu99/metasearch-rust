use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("engine '{engine}' timed out")]
    Timeout { engine: &'static str },

    #[error("engine '{engine}' HTTP error: {source}")]
    Http {
        engine: &'static str,
        #[source]
        source: reqwest::Error,
    },

    #[error("engine '{engine}' returned status {status}")]
    BadStatus { engine: &'static str, status: u16 },

    #[error("engine '{engine}' parse failed: {reason}")]
    ParseFailed {
        engine: &'static str,
        reason: String,
    },

    #[error("engine '{engine}' blocked: {reason}")]
    Blocked {
        engine: &'static str,
        reason: String,
    },
}

/// Axum handler error — wraps anyhow for flexibility at the HTTP boundary.
/// Implements IntoResponse so handlers can use `?` and return typed HTTP errors.
#[derive(Debug)]
pub struct AppError {
    status: axum::http::StatusCode,
    message: String,
}

impl AppError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
            message: msg.into(),
        }
    }
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, axum::Json(body)).into_response()
    }
}
