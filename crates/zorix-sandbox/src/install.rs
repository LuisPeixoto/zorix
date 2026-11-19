//! Fetching the Python module, once (ADR-0022). It is never bundled: it is 26 MB, and it is accepted only from one pinned
//! address and only if its SHA-256 is the one recorded here.

use std::time::Duration;

use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::runner::{PYTHON_BYTES, PYTHON_FILE, PYTHON_SHA256, PYTHON_URL, Sandbox, SandboxError};

fn fail(message: impl std::fmt::Display) -> SandboxError {
    SandboxError::Failed(message.to_string())
}

/// The only hosts a download may be redirected to (GitHub serves release files from its own storage).
fn trusted(host: &str) -> bool {
    host == "github.com" || host.ends_with(".githubusercontent.com")
}

impl Sandbox {
    /// Download Python into the data folder. `progress` is told how many bytes have arrived and how many are expected.
    pub async fn install_python(&self, progress: impl Fn(u64, u64)) -> Result<(), SandboxError> {
        self.install_python_from(PYTHON_URL, progress).await
    }

    /// The same from another address (for tests); the checksum is still the pinned one.
    pub async fn install_python_from(&self, url: &str, progress: impl Fn(u64, u64)) -> Result<(), SandboxError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(600))
            .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::custom(|attempt| {
                let ok = attempt.url().scheme() == "https"
                    && attempt.url().host_str().is_some_and(trusted)
                    && attempt.previous().len() < 5;
                if ok {
                    attempt.follow()
                } else {
                    attempt.error("the download was sent somewhere ZORIX does not trust")
                }
            }))
            .build()
            .map_err(fail)?;
        let mut response =
            client.get(url).send().await.map_err(|e| fail(format!("The download failed: {}", e.without_url())))?;
        if !response.status().is_success() {
            return Err(fail(format!("The download failed (HTTP {}).", response.status().as_u16())));
        }
        let folder = self.folder().to_path_buf();
        tokio::fs::create_dir_all(&folder).await.map_err(fail)?;
        let partial = folder.join(format!("{PYTHON_FILE}.part"));
        let mut file = tokio::fs::File::create(&partial).await.map_err(fail)?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        while let Some(chunk) =
            response.chunk().await.map_err(|e| fail(format!("The download was interrupted: {}", e.without_url())))?
        {
            received += chunk.len() as u64;
            if received > PYTHON_BYTES + 1024 {
                drop(file);
                let _ = tokio::fs::remove_file(&partial).await;
                return Err(fail("The download is larger than the Python module should be, so it was stopped."));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(fail)?;
            progress(received, PYTHON_BYTES);
        }
        file.flush().await.map_err(fail)?;
        drop(file);
        let found: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if found != PYTHON_SHA256 {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(fail(
                "What was downloaded is not the Python module ZORIX expects (its checksum differs), so it was thrown away.",
            ));
        }
        tokio::fs::rename(&partial, self.python_file()).await.map_err(fail)?;
        Ok(())
    }
}
