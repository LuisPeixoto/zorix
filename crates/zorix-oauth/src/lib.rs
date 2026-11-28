//! The OAuth pieces ZORIX's sign-ins share: PKCE, the loopback answer, token calls that a server cannot aim at a private
//! address, and tokens kept in the keychain and renewed when they run out (ADR-0019, ADR-0053).

mod browser;
mod callback;
mod error;
mod reach;
mod session;
mod token;
mod vault;

pub use browser::{BrowserOpener, SystemBrowser};
pub use callback::{LOGIN_TIMEOUT, wait_for_callback};
pub use error::OAuthError;
pub use reach::{Reach, endpoint, get_json, read_json};
pub use session::{Renewable, Session};
pub use token::{TokenResponse, form_body, pkce_pair, random_token, token_request, token_request_json};
pub use vault::{Access, Vault};
