//! The sandbox of a local (stdio) MCP server (ADR-0050): what it may read, write and reach, enforced by the operating system.
//!
//! - **macOS:** a Seatbelt profile run through `sandbox-exec`.
//! - **Linux:** Landlock for files and a seccomp filter for sockets, applied in the child before the program starts.
//! - **Windows:** only the Job Object's limits ([`crate::job`]). Files and the network are **not** confined, and the interface says so.
//!
//! The server gets a folder of its own as its home, so a package manager keeps its cache there. It can read the system and the
//! language toolchains it runs on, the folders a person gave it, and nothing else of the person's; it can write only in its own folder
//! and the folders given for writing. Credential folders and ZORIX's data are refused even when a wider folder would include them.
//! Local services reached over Unix sockets (the keyring, the Docker socket, the SSH agent) are refused too.

use std::path::{Path, PathBuf};

use tokio::process::Command;
use zorix_core::{SandboxMode, ServerSandbox};

use crate::error::McpError;

/// Folders under a home that hold credentials. Never readable by a confined server.
const CREDENTIALS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".config/gcloud",
    ".config/gh",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    "Library/Keychains",
    "Library/Cookies",
    ".local/share/keyrings",
];

/// Folders under a home where language toolchains live. A server written for Node or Python cannot start without reading them.
const TOOLCHAINS: &[&str] = &[
    ".nvm",
    ".volta",
    ".fnm",
    ".asdf",
    ".pyenv",
    ".rbenv",
    ".bun",
    ".deno",
    ".npm-global",
    ".local/bin",
    ".local/share/uv",
    ".local/share/mise",
    ".local/share/fnm",
    ".cargo/bin",
    "Library/pnpm",
    "n",
];

/// What a confined server may touch.
#[derive(Clone, Debug)]
pub struct Confinement {
    /// Its own folder: its home, temporary files and package caches.
    pub server_dir: PathBuf,
    /// Folders given by a person, and whether it may write in them.
    pub folders: Vec<(PathBuf, bool)>,
    /// Whether it may open network connections.
    pub network: bool,
    /// Refused whatever else allows them: credential folders and ZORIX's data.
    pub private: Vec<PathBuf>,
    /// Where a person's files live. Not readable except for what is allowed above (macOS; Linux refuses everything not listed).
    pub user_data: Vec<PathBuf>,
    /// The person's home, where toolchains are looked for.
    pub home: Option<PathBuf>,
}

/// What this operating system enforces of a [`Confinement`], so nothing is promised that is not done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enforced {
    /// Files outside what is allowed cannot be read or written.
    pub files: bool,
    /// With the network off, connections cannot be opened.
    pub network: bool,
    /// Credential folders and ZORIX's data cannot be read, and local services over Unix sockets cannot be reached.
    pub credentials: bool,
    /// What the server starts ends with it.
    pub processes: bool,
}

/// What native confinement enforces on this computer.
pub fn enforced() -> Enforced {
    let files = cfg!(any(target_os = "macos", target_os = "linux"));
    Enforced { files, network: files, credentials: files, processes: true }
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|p| p.is_absolute())
}

impl Confinement {
    /// The confinement `sandbox` asks for, or `None` when the server is not confined. `data_home` is ZORIX's data folder.
    pub fn of(sandbox: &ServerSandbox, server_dir: PathBuf, data_home: Option<&Path>) -> Option<Confinement> {
        if sandbox.mode != SandboxMode::Native {
            return None;
        }
        let home = user_home().map(|h| real(&h));
        let mut private: Vec<PathBuf> = home.iter().flat_map(|h| CREDENTIALS.iter().map(move |c| h.join(c))).collect();
        private.extend(data_home.map(real));
        let mut user_data: Vec<PathBuf> = home.clone().into_iter().collect();
        if cfg!(target_os = "macos") {
            user_data.extend(["/Users", "/Volumes"].map(PathBuf::from));
        }
        let folders = sandbox.folders.iter().map(|f| (real(Path::new(&f.path)), f.write)).collect();
        Some(Confinement { server_dir, folders, network: sandbox.network, private, user_data, home })
    }

    /// The variables that put the server's home, temporary files and caches in its own folder. Variables a person gave the server are set
    /// after these and win.
    pub fn environment(&self) -> Vec<(&'static str, PathBuf)> {
        if !enforced().files {
            return Vec::new();
        }
        let own = &self.server_dir;
        vec![
            ("HOME", own.clone()),
            ("TMPDIR", own.join("tmp")),
            ("XDG_CACHE_HOME", own.join("cache")),
            ("XDG_CONFIG_HOME", own.join("config")),
            ("XDG_DATA_HOME", own.join("data")),
            ("npm_config_cache", own.join("cache/npm")),
            ("UV_CACHE_DIR", own.join("cache/uv")),
        ]
    }

    /// Make the folders [`environment`](Self::environment) names.
    pub fn prepare(&mut self) -> Result<(), McpError> {
        let cannot = |e: std::io::Error| McpError::Spawn(format!("the server's own folder could not be made: {e}"));
        for part in ["tmp", "cache", "config", "data"] {
            std::fs::create_dir_all(self.server_dir.join(part)).map_err(cannot)?;
        }
        self.server_dir = std::fs::canonicalize(&self.server_dir).map_err(cannot)?;
        Ok(())
    }

    /// The toolchain folders of the person that exist, and the folders the program itself sits in.
    fn toolchains(&self, program: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> =
            self.home.iter().flat_map(|h| TOOLCHAINS.iter().map(move |t| h.join(t))).filter(|p| p.exists()).collect();
        for path in [program.to_path_buf(), real(program)] {
            match path.parent() {
                // A program that sits right in a home or at the top of the disk gets only itself, not everything beside it.
                Some(parent) if parent.parent().is_some() && !self.user_data.iter().any(|u| u == parent) => {
                    found.push(parent.to_path_buf());
                }
                _ => found.push(path.clone()),
            }
        }
        found
    }

    /// The command that runs `program` confined. `program` is the file that was found for the name a person wrote.
    pub fn command(&self, program: &Path, args: &[String]) -> Result<Command, McpError> {
        platform::command(self, program, args)
    }
}

/// The file a command name stands for: itself when it has a folder in it, else the first match on `PATH`.
pub fn find_program(name: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 {
        let full = match cwd {
            Some(cwd) if path.is_relative() => cwd.join(path),
            _ => path.to_path_buf(),
        };
        return full.is_file().then_some(full);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// A path as a Seatbelt string. Paths with a quote or a newline were refused when they were chosen; they are escaped all the same.
    fn quoted(path: &Path) -> String {
        format!("\"{}\"", path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('\n', ""))
    }

    fn subpaths(paths: &[PathBuf]) -> String {
        paths.iter().map(|p| format!(" (subpath {})", quoted(p))).collect()
    }

    /// The profile. Later rules win over earlier ones, which is what lets a refusal inside an allowed folder, and the server's own
    /// folder inside ZORIX's refused data folder, both hold.
    pub(super) fn profile(c: &Confinement, program: &Path) -> String {
        let writable: Vec<PathBuf> = std::iter::once(c.server_dir.clone())
            .chain(c.folders.iter().filter(|(_, write)| *write).map(|(p, _)| p.clone()))
            .collect();
        let readable: Vec<PathBuf> = c
            .toolchains(program)
            .into_iter()
            .chain(c.folders.iter().map(|(p, _)| p.clone()))
            .chain(std::iter::once(c.server_dir.clone()))
            .collect();
        let mut p = String::from("(version 1)\n(allow default)\n");
        // Asking another program to act is a way out: LaunchServices, AppleScript, launchd, the keychain and the pasteboard are all
        // services reached by name, so none is reachable except the few a program needs to resolve names, users and certificates.
        p.push_str("(deny appleevent-send)\n(deny lsopen)\n(deny mach-lookup)\n(allow mach-lookup");
        for service in [
            "com.apple.system.opendirectoryd.libinfo",
            "com.apple.system.notification_center",
            "com.apple.system.logger",
            "com.apple.logd",
            "com.apple.trustd.agent",
            "com.apple.SystemConfiguration.configd",
            "com.apple.dnssd.service",
        ] {
            p.push_str(&format!(" (global-name \"{service}\")"));
        }
        p.push_str(")\n");
        p.push_str("(deny file-write*)\n");
        p.push_str(&format!(
            "(allow file-write*{} (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n",
            subpaths(&writable)
        ));
        if !c.user_data.is_empty() {
            p.push_str(&format!("(deny file-read-data{})\n", subpaths(&c.user_data)));
        }
        p.push_str(&format!("(allow file-read-data{})\n", subpaths(&readable)));
        // A rule for one operation (`file-read-data`) wins over a later rule for the family (`file-read*`), so a refusal has to name the
        // very operation that was allowed above, or a folder a person gave would open the credentials inside it.
        if !c.private.is_empty() {
            p.push_str(&format!("(deny file-read-data file-read* file-write*{})\n", subpaths(&c.private)));
        }
        p.push_str(&format!(
            "(allow file-read-data file-read* file-write*{})\n",
            subpaths(std::slice::from_ref(&c.server_dir))
        ));
        // Resolving a path asks about every folder on the way to it, and some of those are refused above (the server's own folder sits in
        // ZORIX's data folder). Only that those folders exist can be asked, never what is in them.
        let mut way: Vec<PathBuf> = Vec::new();
        for path in std::iter::once(&c.server_dir).chain(c.folders.iter().map(|(p, _)| p)) {
            for ancestor in path.ancestors().skip(1) {
                if !way.iter().any(|p| p == ancestor) {
                    way.push(ancestor.to_path_buf());
                }
            }
        }
        p.push_str("(allow file-read-metadata");
        for folder in &way {
            p.push_str(&format!(" (literal {})", quoted(folder)));
        }
        p.push_str(")\n");
        if c.network {
            // The internet, but no local service behind a Unix socket (the SSH agent, the Docker socket), except the name resolver.
            p.push_str("(deny network-outbound (remote unix-socket))\n");
            p.push_str(
                "(allow network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")))\n",
            );
        } else {
            p.push_str("(deny network*)\n");
        }
        p
    }

    pub(super) fn command(c: &Confinement, program: &Path, args: &[String]) -> Result<Command, McpError> {
        let launcher = Path::new("/usr/bin/sandbox-exec");
        if !launcher.is_file() {
            return Err(McpError::Spawn(
                "this macOS has no sandbox-exec, so ZORIX cannot confine the server. Turn its sandbox off if you trust it.".into(),
            ));
        }
        let mut command = Command::new(launcher);
        command.arg("-p").arg(profile(c, program)).arg(program).args(args);
        Ok(command)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::BTreeMap;

    use landlock::{
        ABI, Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetCreated, RulesetCreatedAttr,
        RulesetStatus, path_beneath_rules,
    };
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
        TargetArch,
    };

    use super::*;

    /// What every program needs to run, none of it a person's files.
    const SYSTEM: &[&str] = &[
        "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/proc", "/sys", "/dev",
        "/run", "/nix", "/snap",
    ];

    fn unsupported(what: &str) -> McpError {
        McpError::Spawn(format!("this Linux cannot confine the server ({what}). Turn its sandbox off if you trust it."))
    }

    /// Landlock refuses everything not listed. Credential folders are safe because nothing above them is listed: only the system,
    /// the toolchains, the server's folder and the folders a person chose (which may not be a credential folder).
    fn ruleset(c: &Confinement, program: &Path) -> Result<RulesetCreated, McpError> {
        let abi = ABI::V3;
        let all = AccessFs::from_all(abi);
        let read = AccessFs::from_read(abi);
        // Without the first version of Landlock there is no confinement at all, and ZORIX does not pretend otherwise.
        Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(ABI::V1))
            .map_err(|_| unsupported("the kernel has no Landlock"))?;
        let readable: Vec<PathBuf> = SYSTEM
            .iter()
            .map(PathBuf::from)
            .chain(c.toolchains(program))
            .chain(c.folders.iter().filter(|(_, write)| !*write).map(|(p, _)| p.clone()))
            .collect();
        let writable: Vec<PathBuf> = std::iter::once(c.server_dir.clone())
            .chain(c.folders.iter().filter(|(_, write)| *write).map(|(p, _)| p.clone()))
            .chain(["/dev/null", "/dev/zero", "/dev/shm", "/dev/tty"].map(PathBuf::from))
            .collect();
        let fail = |e: landlock::RulesetError| unsupported(&e.to_string());
        Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(all)
            .map_err(fail)?
            .create()
            .map_err(fail)?
            .add_rules(path_beneath_rules(&readable, read))
            .map_err(fail)?
            .add_rules(path_beneath_rules(&writable, all))
            .map_err(fail)
    }

    /// Refuse the sockets that are not allowed: Unix sockets always (the keyring over D-Bus, the Docker socket, the SSH agent), internet
    /// sockets when the network is off, and io_uring, which can open sockets without the `socket` call.
    fn filter(network: bool) -> Result<BpfProgram, McpError> {
        let fail = |e: seccompiler::BackendError| unsupported(&e.to_string());
        let family = |domain: i32| -> Result<SeccompRule, McpError> {
            let condition =
                SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, domain as u64).map_err(fail)?;
            SeccompRule::new(vec![condition]).map_err(fail)
        };
        let mut sockets = vec![family(libc::AF_UNIX)?];
        if !network {
            for domain in [libc::AF_INET, libc::AF_INET6, libc::AF_PACKET] {
                sockets.push(family(domain)?);
            }
        }
        let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        rules.insert(libc::SYS_socket, sockets);
        rules.insert(libc::SYS_io_uring_setup, Vec::new());
        let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(|_| unsupported("this processor"))?;
        let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), arch)
            .map_err(fail)?;
        filter.try_into().map_err(fail)
    }

    pub(super) fn command(c: &Confinement, program: &Path, args: &[String]) -> Result<Command, McpError> {
        let mut rules = Some(ruleset(c, program)?);
        let sockets = filter(c.network)?;
        let mut command = Command::new(program);
        command.args(args);
        // SAFETY: the closure runs in the child between fork and exec. It only makes the system calls that confine the process, with
        // everything they need built before the fork.
        unsafe {
            command.pre_exec(move || {
                if let Some(rules) = rules.take() {
                    let status = rules.restrict_self().map_err(|_| std::io::Error::from_raw_os_error(libc::EPERM))?;
                    if status.ruleset == RulesetStatus::NotEnforced {
                        return Err(std::io::Error::from_raw_os_error(libc::ENOSYS));
                    }
                }
                seccompiler::apply_filter(&sockets).map_err(|_| std::io::Error::from_raw_os_error(libc::EPERM))
            });
        }
        Ok(command)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    /// Nothing to wrap: the Job Object of the process tree carries the limits ([`crate::job`]).
    pub(super) fn command(_: &Confinement, program: &Path, args: &[String]) -> Result<Command, McpError> {
        let mut command = Command::new(program);
        command.args(args);
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::SandboxFolder;

    use super::*;

    fn sandbox(mode: SandboxMode) -> ServerSandbox {
        ServerSandbox {
            mode,
            network: false,
            folders: vec![SandboxFolder { path: std::env::temp_dir().display().to_string(), write: true }],
        }
    }

    #[test]
    fn only_a_native_sandbox_confines_and_credentials_and_the_data_folder_are_always_refused() {
        let own = std::env::temp_dir().join("zorix-sandbox-test");
        assert!(Confinement::of(&sandbox(SandboxMode::Off), own.clone(), None).is_none());
        let data = std::env::temp_dir().join("zorix-data");
        let confined = Confinement::of(&sandbox(SandboxMode::Native), own, Some(&data)).unwrap();
        assert!(confined.private.iter().any(|p| p.ends_with("zorix-data")), "{:?}", confined.private);
        if user_home().is_some() {
            assert!(confined.private.iter().any(|p| p.ends_with(".ssh")), "{:?}", confined.private);
        }
        assert!(confined.folders.len() == 1 && confined.folders[0].1 && !confined.network);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_profile_refuses_what_it_must_after_what_it_allows() {
        let c = Confinement {
            server_dir: "/Users/ana/Library/zorix/mcp/1".into(),
            folders: vec![("/Users/ana/notes".into(), false), ("/Users/ana/out \"x\"".into(), true)],
            network: false,
            private: vec!["/Users/ana/.ssh".into(), "/Users/ana/Library/zorix".into()],
            user_data: vec!["/Users".into()],
            home: Some("/Users/ana".into()),
        };
        let p = platform::profile(&c, Path::new("/usr/local/bin/node"));
        let at = |needle: &str| p.find(needle).unwrap_or_else(|| panic!("{needle} is not in:\n{p}"));
        assert!(at("(deny mach-lookup)") < at("(allow mach-lookup"));
        assert!(at("(deny file-write*)") < at("(allow file-write*"));
        assert!(at("(deny file-read-data") < at("(allow file-read-data"));
        assert!(
            at("(deny file-read-data file-read* file-write*") < at("(allow file-read-data file-read* file-write*"),
            "the server's own folder wins last"
        );
        assert!(p.contains("(deny network*)") && p.contains("/Users/ana/out \\\"x\\\""), "{p}");
        let with_network = platform::profile(&Confinement { network: true, ..c }, Path::new("/usr/local/bin/node"));
        assert!(
            !with_network.contains("(deny network*)")
                && with_network.contains("(deny network-outbound (remote unix-socket))")
        );
    }
}
