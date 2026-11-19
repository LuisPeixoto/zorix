//! How a request to a remote server proves who it is. The HTTP transport asks an [`AuthProvider`] for the header and tells
//! it when the server answers 401 or 403 (ADR-0019).

use async_trait::async_trait;
use zorix_core::Secret;

use crate::error::McpError;

/// Supplies the `Authorization` header of requests to one server.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// The value of the `Authorization` header for the next request, if there is one.
    async fn authorization(&self) -> Result<Option<String>, McpError>;

    /// The server answered 401 or 403. `Ok` means trying once more may work (a token was refreshed); an error says what a
    /// person has to do.
    async fn challenged(&self, status: u16, www_authenticate: Option<&str>) -> Result<(), McpError>;
}

/// No sign-in.
pub struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn authorization(&self) -> Result<Option<String>, McpError> {
        Ok(None)
    }

    async fn challenged(&self, status: u16, _www_authenticate: Option<&str>) -> Result<(), McpError> {
        Err(McpError::NeedsLogin(format!("the server answered {status}, and no sign-in is set up for it")))
    }
}

/// A fixed bearer token the person pasted.
pub struct StaticBearer(pub Secret);

#[async_trait]
impl AuthProvider for StaticBearer {
    async fn authorization(&self) -> Result<Option<String>, McpError> {
        Ok(Some(format!("Bearer {}", self.0.expose())))
    }

    async fn challenged(&self, status: u16, _www_authenticate: Option<&str>) -> Result<(), McpError> {
        Err(McpError::NeedsLogin(format!(
            "the server did not accept the token it was given ({status}); paste a new one"
        )))
    }
}
