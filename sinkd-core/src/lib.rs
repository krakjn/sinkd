//! sinkd-core — shared sync daemon library (Zenoh + rsync).
//!
//! Binaries: `sinkd` (client) and `sinkd-srv` (server).

#[macro_use]
pub mod fancy;
#[macro_use]
pub mod outcome;
pub mod config;
pub mod conflict;
pub mod init;
pub mod ipc;
pub mod parameters;
pub mod rsync;
pub mod shiplog;
pub mod test_hooks;
pub mod time;

pub use outcome::Outcome;
pub use std::path::{Path, PathBuf};
pub use std::fs;

pub fn have_permissions() -> bool {
    #[cfg(unix)]
    {
        // get effective user ID
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::GetTokenInformation;
        use windows::Win32::Security::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let process_handle = GetCurrentProcess();
            let mut token_handle: HANDLE = HANDLE::default();
            if OpenProcessToken(process_handle, TOKEN_QUERY, &raw mut token_handle).is_err() {
                return false;
            }
            // Check if the token has admin rights
            let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
            let mut return_length = 0;
            let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(u32::MAX);
            if GetTokenInformation(
                token_handle,
                TokenElevation,
                // look away, interfacing with Windows is hacky even with windows crate
                Some((&raw mut elevation).cast()),
                size,
                &raw mut return_length,
            )
            .is_err()
            {
                return false;
            }

            elevation.TokenIsElevated != 0
        }
    }
}

#[cfg(unix)]
pub fn get_hostname() -> Outcome<String> {
    use libc::{c_char, sysconf, _SC_HOST_NAME_MAX};
    use std::ffi::CStr;

    unsafe {
        // Get the maximum hostname length
        let max_len = sysconf(_SC_HOST_NAME_MAX);
        if max_len == -1 {
            return bad!("Failed to determine maximum hostname length");
        }

        let mut buffer =
            vec![0u8; usize::try_from(max_len).map_err(|_| "Invalid hostname buffer size")?];
        let ptr = buffer.as_mut_ptr().cast::<c_char>();

        if libc::gethostname(
            ptr,
            usize::try_from(max_len).map_err(|_| "Invalid hostname buffer size")?,
        ) != 0
        {
            return bad!("Failed to retrieve hostname");
        }

        // Convert the hostname from C string to Rust string
        let cstr = CStr::from_ptr(ptr);
        match cstr.to_str() {
            Ok(s) => Ok(s.to_owned()),
            Err(e) => bad!("Failed to convert hostname to UTF-8: {}", e),
        }
    }
}

#[cfg(windows)]
pub fn get_hostname() -> Outcome<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::WindowsProgramming::{GetComputerNameW, MAX_COMPUTERNAME_LENGTH};

    let mut buffer = [0u16; MAX_COMPUTERNAME_LENGTH as usize + 1];
    let mut size = u32::try_from(buffer.len()).unwrap_or(u32::MAX);

    unsafe {
        let pwstr = windows::core::PWSTR(buffer.as_mut_ptr());
        if let Err(e) = GetComputerNameW(Some(pwstr), &raw mut size) {
            bad!("Failed to retrieve the hostname: {}", e)
        } else {
            let hostname = OsString::from_wide(&buffer[..size as usize]);
            Ok(hostname.to_string_lossy().into_owned())
        }
    }
}


// this will resolve all known paths, converts relative to absolute
pub fn resolve(path: &str) -> Outcome<PathBuf> {
    // NOTE: `~` is a shell expansion not handled by system calls
    if path.starts_with("~/") {
        let mut p = match std::env::var("HOME") {
            Ok(home_dir) => PathBuf::from(home_dir),
            Err(e) => {
                return bad!("HOME env var not defined: {}", e);
            }
        };
        let after_tilde = path
            .strip_prefix("~/")
            .ok_or_else(|| format!("internal: path was expected to start with '~/': {path}"))?;
        p.push(after_tilde);
        match p.canonicalize() {
            Ok(resolved) => Ok(resolved),
            Err(e) => bad!("{} '{}'", e, p.display()),
        }
    } else {
        match PathBuf::from(path).canonicalize() {
            Ok(resolved) => Ok(resolved),
            Err(e) => bad!("{} '{}'", e, path),
        }
    }
}

pub fn ensure_path(path: &Path) -> Outcome<()> {
    if path.exists() {
        return Ok(());
    }
    log::debug!("Creating directory for path: {}", path.display());
    match fs::create_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) => bad!("Unable to create '{}'  {}", path.display(), e),
    }
}

/// Ensure parent directories exist when `path` is a file (config, log, etc.).
pub fn ensure_parent_dir(path: &Path) -> Outcome<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    if parent.exists() {
        return Ok(());
    }
    log::debug!("Creating parent directory: {}", parent.display());
    match fs::create_dir_all(parent) {
        Ok(()) => Ok(()),
        Err(e) => bad!("Unable to create '{}' {}", parent.display(), e),
    }
}