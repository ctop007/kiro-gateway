//! The few operating system calls the gateway manager needs: liveness, the executable
//! path of a process, and ending a process.
//!
//! Behind the [`ProcessOps`] trait so the stale-process logic can be tested with scripted
//! answers. Unix uses `libc`; Windows uses `windows-sys` (`cfg(windows)` only).

use std::path::PathBuf;

/// Inspecting and ending processes by pid.
pub trait ProcessOps: Send + Sync {
    /// Whether the process exists and is running. A zombie (exited, not yet reaped) is not alive.
    fn is_alive(&self, pid: u32) -> bool;
    /// Absolute path of the executable the process runs, if it can be determined.
    fn image_path(&self, pid: u32) -> Option<PathBuf>;
    /// Ask the process to exit (SIGTERM). On Windows there is no polite way, so this ends it.
    fn terminate(&self, pid: u32);
    /// End the process and its process group at once (SIGKILL / TerminateProcess).
    fn kill(&self, pid: u32);
}

/// Real implementation for this platform.
pub struct SystemProcessOps;

/// Whether it is acceptable to signal `pid`.
///
/// Refuses pid 0 (would address our own process group), pid 1 (init / launchd), the App
/// itself, and values that do not fit the OS pid type. A forged or corrupt `gateway.pid`
/// must never turn into a signal for any of those.
pub fn is_signalable(pid: u32) -> bool {
    pid > 1 && pid <= i32::MAX as u32 && pid != std::process::id()
}

#[cfg(unix)]
mod imp {
    use super::*;

    impl ProcessOps for SystemProcessOps {
        fn is_alive(&self, pid: u32) -> bool {
            if pid == 0 || pid > i32::MAX as u32 {
                return false;
            }
            // Signal 0 only checks for existence; EPERM means it exists but is not ours.
            let exists = unsafe { libc::kill(pid as i32, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
            exists && !is_zombie(pid)
        }

        fn image_path(&self, pid: u32) -> Option<PathBuf> {
            if pid == 0 || pid > i32::MAX as u32 {
                return None;
            }
            image_path_of(pid)
        }

        fn terminate(&self, pid: u32) {
            if is_signalable(pid) {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }

        fn kill(&self, pid: u32) {
            if is_signalable(pid) {
                unsafe {
                    // The sidecar is started as the leader of its own process group, so this
                    // also takes down anything it spawned. It fails harmlessly (ESRCH) when
                    // `pid` is not a group leader; the direct kill covers that case.
                    libc::killpg(pid as i32, libc::SIGKILL);
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn image_path_of(pid: u32) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let written = unsafe { libc::proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if written <= 0 {
            return None;
        }
        buffer.truncate(written as usize);
        Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
    }

    #[cfg(not(target_os = "macos"))]
    fn image_path_of(pid: u32) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }

    /// macOS: `kill(pid, 0)` succeeds for a zombie (exited, not yet reaped), but
    /// `proc_pidinfo` already answers ESRCH for it.
    #[cfg(target_os = "macos")]
    fn is_zombie(pid: u32) -> bool {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        if got == size {
            return info.pbi_status == libc::SZOMB;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    #[cfg(not(target_os = "macos"))]
    fn is_zombie(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|text| text.rsplit_once(')').map(|(_, rest)| rest.trim_start().starts_with('Z')))
            .unwrap_or(false)
    }
}

/// Give a child process the default signal state instead of the App's.
///
/// `signals::install` blocks SIGTERM / SIGINT / SIGHUP in the main thread, and both a signal
/// mask and "ignored" dispositions survive `fork` and `exec`. Without this a sidecar never
/// sees SIGTERM and every stop waits for the SIGKILL fallback. The hook runs between `fork`
/// and `exec` and only calls async-signal-safe functions (`sigaction`, `sigemptyset`,
/// `pthread_sigmask`).
#[cfg(unix)]
pub fn restore_default_signals(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure only calls async-signal-safe libc functions and allocates nothing.
    unsafe {
        command.pre_exec(|| {
            let mut default_action: libc::sigaction = std::mem::zeroed();
            default_action.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut default_action.sa_mask);
            for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT, libc::SIGPIPE] {
                libc::sigaction(signal, &default_action, std::ptr::null_mut());
            }
            let mut empty: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut empty);
            if libc::pthread_sigmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };

    /// `STILL_ACTIVE` exit code of a running process.
    const STILL_ACTIVE: u32 = 259;

    fn open(pid: u32, access: u32) -> Option<HANDLE> {
        let handle = unsafe { OpenProcess(access, FALSE, pid) };
        if handle.is_null() {
            None
        } else {
            Some(handle)
        }
    }

    impl ProcessOps for SystemProcessOps {
        fn is_alive(&self, pid: u32) -> bool {
            let Some(handle) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return false };
            let mut code = 0u32;
            let ok = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
            unsafe { CloseHandle(handle) };
            ok && code == STILL_ACTIVE
        }

        fn image_path(&self, pid: u32) -> Option<PathBuf> {
            let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
            let mut buffer = vec![0u16; 32 * 1024];
            let mut len = buffer.len() as u32;
            let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) } != 0;
            unsafe { CloseHandle(handle) };
            if !ok {
                return None;
            }
            Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])))
        }

        fn terminate(&self, pid: u32) {
            self.kill(pid);
        }

        fn kill(&self, pid: u32) {
            if !is_signalable(pid) {
                return;
            }
            if let Some(handle) = open(pid, PROCESS_TERMINATE) {
                unsafe {
                    TerminateProcess(handle, 1);
                    CloseHandle(handle);
                }
            }
        }
    }

    /// Process-wide Job Object that kills its processes when the App exits or crashes.
    struct Job(HANDLE);
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0;
            if ok {
                Some(Job(handle))
            } else {
                CloseHandle(handle);
                None
            }
        })
        .as_ref()
    }

    /// Bind `child` to the kill-on-close Job Object, so it dies with the App even if the App crashes.
    ///
    /// Returns whether the assignment worked (it can fail when the App itself already runs in a
    /// job that forbids nesting; the gateway is then ended by the normal stop path only).
    pub fn bind_to_job(child: &std::process::Child) -> bool {
        match job() {
            Some(job) => unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) != 0 },
            None => false,
        }
    }
}

#[cfg(windows)]
pub use imp::bind_to_job;

#[cfg(test)]
mod tests;
