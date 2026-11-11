//! One MyTools per installation: a second start asks the running one to show its window
//! and quits, so it neither loads plugins nor fights over the shortcuts.
//!
//! On Windows the lock is a named mutex in the user's session, named after the executable's
//! directory, so a development build in `target/` still runs beside the installed copy.
//! The system releases it however the process ends, so a crash never locks MyTools out.
//! Elsewhere there is no lock yet and every start runs.
use std::time::Duration;

/// How long a start that replaces a running MyTools (a restart, a deploy) waits for it to
/// exit and release the lock.
pub const RESTART_WAIT: Duration = Duration::from_secs(10);

#[cfg(windows)]
pub use platform::Instance;

/// Claims this installation; `None` when another MyTools of it still runs after `wait`.
#[cfg(windows)]
pub fn claim(wait: Duration) -> Option<Instance> {
    Instance::claim(wait)
}

/// Asks the MyTools that holds the lock to show its window.
#[cfg(windows)]
pub fn show_running() {
    platform::signal_show();
}

#[cfg(not(windows))]
pub struct Instance;

#[cfg(not(windows))]
impl Instance {
    pub fn on_show(&self, _show: impl Fn() + Send + 'static) {}
}

#[cfg(not(windows))]
pub fn claim(_wait: Duration) -> Option<Instance> {
    Some(Instance)
}

#[cfg(not(windows))]
pub fn show_running() {}

/// A name shared by every start of the same installation: paths are case-insensitive on
/// Windows, so the directory is lowercased before hashing (FNV-1a).
#[cfg_attr(not(windows), allow(dead_code))]
fn name(kind: &str, directory: &str) -> String {
    let hash = directory
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("Local\\MyTools.{kind}.{hash:016x}")
}

#[cfg(windows)]
mod platform {
    use std::{ptr, time::Duration};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::{
            CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, ReleaseMutex,
            SetEvent, WaitForSingleObject,
        },
    };

    fn wide(kind: &str) -> Vec<u16> {
        let directory = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.display().to_string()))
            .unwrap_or_default();
        super::name(kind, &directory)
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    /// Holds the lock until dropped, which is when MyTools exits.
    pub struct Instance {
        mutex: HANDLE,
        /// Signalled by later starts that want the window shown.
        show: HANDLE,
    }

    impl Instance {
        pub fn claim(wait: Duration) -> Option<Self> {
            let name = wide("Instance");
            let mutex = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
            if mutex.is_null() {
                // Without a lock MyTools still runs, as it did before single instances.
                return Some(Self {
                    mutex,
                    show: ptr::null_mut(),
                });
            }
            let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX - 1);
            match unsafe { WaitForSingleObject(mutex, millis) } {
                // Abandoned: the previous holder ended without releasing it, e.g. it crashed.
                WAIT_OBJECT_0 | WAIT_ABANDONED => {
                    let name = wide("Show");
                    let show = unsafe { CreateEventW(ptr::null(), 0, 0, name.as_ptr()) };
                    Some(Self { mutex, show })
                }
                _ => {
                    unsafe { CloseHandle(mutex) };
                    None
                }
            }
        }

        /// Calls `show` on a background thread whenever a later start asks for the window.
        pub fn on_show(&self, show: impl Fn() + Send + 'static) {
            if self.show.is_null() {
                return;
            }
            // The handle stays open until MyTools exits, outliving this thread's waits.
            let event = self.show as usize;
            std::thread::spawn(move || {
                while unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } == WAIT_OBJECT_0 {
                    show();
                }
            });
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                if !self.mutex.is_null() {
                    ReleaseMutex(self.mutex);
                    CloseHandle(self.mutex);
                }
                if !self.show.is_null() {
                    CloseHandle(self.show);
                }
            }
        }
    }

    pub fn signal_show() {
        let name = wide("Show");
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if !event.is_null() {
            unsafe {
                SetEvent(event);
                CloseHandle(event);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_second_claim_waits_for_the_first_to_go() {
            let first = Instance::claim(Duration::ZERO).expect("nothing else holds the lock");
            // The mutex belongs to the claiming thread; another thread is another holder.
            let blocked = std::thread::spawn(|| Instance::claim(Duration::ZERO).is_none());
            assert!(blocked.join().unwrap());
            let (shown, seen) = std::sync::mpsc::channel();
            first.on_show(move || {
                let _ = shown.send(());
            });
            signal_show();
            assert!(seen.recv_timeout(Duration::from_secs(5)).is_ok());
            drop(first);
            let later = std::thread::spawn(|| Instance::claim(Duration::ZERO).is_some());
            assert!(later.join().unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_follow_the_directory_but_not_its_case() {
        let installed = super::name("Instance", r"C:\soft\MyTools");
        assert!(installed.starts_with("Local\\MyTools.Instance."));
        assert_eq!(installed, super::name("Instance", r"c:\SOFT\mytools"));
        assert_ne!(
            installed,
            super::name("Instance", r"C:\git\MyTools.Rust\target\debug")
        );
        assert_ne!(installed, super::name("Show", r"C:\soft\MyTools"));
    }
}
