/// What can go wrong signing in or renewing a sign-in, in words a person can act on.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum OAuthError {
    /// The network or the other side failed.
    #[error("Talking to the server failed: {0}")]
    Transport(String),
    /// Nobody answered in time.
    #[error("The server did not answer in time: {0}")]
    Timeout(String),
    /// It was cancelled.
    #[error("The call was cancelled.")]
    Cancelled,
    /// The answer broke the rules.
    #[error("The server broke the protocol: {0}")]
    Protocol(String),
    /// A person has to sign in.
    #[error("The server needs you to sign in: {0}")]
    NeedsLogin(String),
    /// A client could not be made.
    #[error("The server could not be started: {0}")]
    Spawn(String),
    /// A response was bigger than the limit.
    #[error("The server sent a message larger than {limit} bytes, so the connection was closed.")]
    TooLarge {
        /// The limit.
        limit: usize,
    },
}
