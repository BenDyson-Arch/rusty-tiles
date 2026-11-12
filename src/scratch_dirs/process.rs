//! Process identity that tells a running owner apart from a later process reusing its PID.
//!
//! The start token is the process start time in the platform's own clock: on Linux the
//! boot ID with `/proc/<pid>/stat` start ticks, on macOS the BSD start time, on Windows the
//! process creation time. Other platforms have no token; their owners are never proved
//! gone while their PID exists.
/// What a PID names right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// No such process, or only its zombie remains.
    Absent,
    /// A process exists; `start` is its start token when it can be read.
    Present { start: Option<String> },
}

/// The current process's PID and start token.
pub(crate) fn current() -> (u32, Option<String>) {
    let pid = std::process::id();
    let start = match query(pid) {
        Presence::Present { start } => start,
        Presence::Absent => None,
    };
    (pid, start)
}

/// This machine's host name, when the platform gives one.
pub(crate) fn host() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is writable for its whole length.
        let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
        if status != 0 {
            return None;
        }
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
        Some(name).filter(|name| !name.is_empty())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.is_empty())
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Whether `path` belongs to the user running this process. Platforms without file
/// ownership in the standard library report `true`; removal permissions still apply.
pub(crate) fn owned_by_current_user(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

/// The state of `pid` now.
pub(crate) fn query(pid: u32) -> Presence {
    platform::query(pid)
}

/// The process state and start ticks from the text of `/proc/<pid>/stat`. The command
/// name in parentheses may contain spaces and parentheses, so fields are counted from
/// the last `)`.
pub(crate) fn parse_linux_stat(text: &str) -> Option<(char, u64)> {
    let rest = &text[text.rfind(')')? + 1..];
    let mut fields = rest.split_ascii_whitespace();
    let state = fields.next()?.chars().next()?;
    // Field 3 is the state; field 22 is the start time in clock ticks since boot.
    let start = fields.nth(22 - 4)?.parse().ok()?;
    Some((state, start))
}

/// `kill(pid, 0)`: whether a process with this PID exists (permission errors mean it does).
#[cfg(unix)]
fn kill_probe(pid: u32) -> Presence {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return Presence::Absent;
    };
    if pid <= 0 {
        return Presence::Absent;
    }
    // SAFETY: signal 0 performs only the existence and permission check.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Presence::Present { start: None };
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Presence::Absent,
        _ => Presence::Present { start: None },
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub(super) fn query(pid: u32) -> Presence {
        if pid == 0 {
            return Presence::Absent;
        }
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Presence::Absent,
            // A restricted /proc (hidepid) still answers kill(2).
            Err(_) => return kill_probe(pid),
        };
        let Some((state, ticks)) = parse_linux_stat(&stat) else {
            return kill_probe(pid);
        };
        if matches!(state, 'Z' | 'X' | 'x') {
            return Presence::Absent;
        }
        let start = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|boot| boot.trim().to_owned())
            .filter(|boot| !boot.is_empty())
            .map(|boot| format!("linux:{boot}:{ticks}"));
        Presence::Present { start }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod platform {
    use super::*;

    pub(super) fn query(pid: u32) -> Presence {
        let Ok(pid) = libc::c_int::try_from(pid) else {
            return Presence::Absent;
        };
        if pid <= 0 {
            return Presence::Absent;
        }
        // SAFETY: zeroed is a valid proc_bsdinfo; the size passed is the buffer's own.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if written != size {
            return kill_probe(pid as u32);
        }
        Presence::Present {
            start: Some(format!(
                "macos:{}.{:06}",
                info.pbi_start_tvsec, info.pbi_start_tvusec
            )),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub(super) fn query(pid: u32) -> Presence {
        if pid == 0 {
            return Presence::Absent;
        }
        // SAFETY: OpenProcess has no memory preconditions; a null handle is checked.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // SAFETY: reads this thread's last error.
            return if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                Presence::Absent
            } else {
                Presence::Present { start: None }
            };
        }
        let mut code = 0u32;
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the handle is open and every out-pointer is a live local.
        let (exited, timed) = unsafe {
            let exited = GetExitCodeProcess(handle, &mut code) != 0 && code != STILL_ACTIVE as u32;
            let timed =
                GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
            CloseHandle(handle);
            (exited, timed)
        };
        if exited {
            return Presence::Absent;
        }
        let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        Presence::Present {
            start: timed.then(|| format!("windows:{ticks}")),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios", windows)))]
mod platform {
    use super::*;

    pub(super) fn query(pid: u32) -> Presence {
        #[cfg(unix)]
        {
            kill_probe(pid)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Presence::Present { start: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_stat_fields_count_from_the_last_parenthesis() {
        let fields: Vec<String> = (4..=52).map(|n| format!("{n}")).collect();
        let text = format!("4242 (a) b (c)) S {}\n", fields.join(" "));
        // Each numeric field holds its own field number; field 22 is the start time.
        assert_eq!(parse_linux_stat(&text), Some(('S', 22)));
        assert_eq!(parse_linux_stat("12 (x) R 1 2"), None);
        assert_eq!(parse_linux_stat("no parenthesis"), None);
    }

    #[test]
    fn the_current_process_is_present_with_a_stable_start() {
        let (pid, start) = current();
        assert_eq!(pid, std::process::id());
        assert_eq!(
            query(pid),
            Presence::Present {
                start: start.clone()
            }
        );
        if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            assert!(start.is_some(), "no start token on a supported platform");
        }
    }

    #[test]
    fn an_exited_child_is_absent() {
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "exit"])
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("true").spawn().unwrap()
        };
        let pid = child.id();
        child.wait().unwrap();
        // The PID could in principle be reused at once; then its start differs.
        match query(pid) {
            Presence::Absent => {}
            Presence::Present { start } => {
                assert_ne!(start, current().1);
            }
        }
        assert_eq!(query(0), Presence::Absent);
    }
}
