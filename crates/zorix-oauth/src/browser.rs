/// Opens the sign-in page for a person. The command line prints the address and tries the browser; the desktop app uses
/// its own opener.
pub trait BrowserOpener: Send + Sync {
    /// Show `url` to the person.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// The system's default browser.
pub struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = std::process::Command::new("rundll32");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = std::process::Command::new("xdg-open");
        #[cfg(target_os = "windows")]
        command.arg("url.dll,FileProtocolHandler");
        command
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}
