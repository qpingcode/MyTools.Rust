use crate::i18n;
use anyhow::{Result, ensure};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::{
            Shell::{
                IShellLinkW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
                ShellExecuteExW, ShellLink,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{Interface, PCWSTR},
};

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Target {
    path: PathBuf,
    arguments: OsString,
    directory: PathBuf,
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn from_wide(value: &[u16]) -> OsString {
    let end = value.iter().position(|&ch| ch == 0).unwrap_or(value.len());
    OsString::from_wide(&value[..end])
}

fn expand(path: &Path) -> Result<PathBuf> {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    let source = wide(path.as_os_str());
    let size = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    ensure!(size > 0, i18n::t("error.admin_target"));
    let mut output = vec![0; size as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut output)) };
    ensure!(
        written > 0 && written <= size,
        i18n::t("error.admin_target")
    );
    Ok(PathBuf::from(from_wide(&output)))
}

fn target(path: &Path) -> Result<Target> {
    let mut launch = Target {
        path: path.into(),
        arguments: OsString::new(),
        directory: path.parent().unwrap_or(Path::new(".")).into(),
    };
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
    {
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
        let file: IPersistFile = link.cast()?;
        let filename = wide(&super::shell_path(path));
        unsafe {
            file.Load(PCWSTR(filename.as_ptr()), STGM_READ)?;
        }
        let (mut executable, mut arguments, mut directory) =
            (vec![0; 32768], vec![0; 32768], vec![0; 32768]);
        unsafe {
            link.GetPath(&mut executable, std::ptr::null_mut(), 0)?;
            link.GetArguments(&mut arguments)?;
            link.GetWorkingDirectory(&mut directory)?;
        }
        launch.path = PathBuf::from(from_wide(&executable));
        launch.arguments = from_wide(&arguments);
        let directory = from_wide(&directory);
        launch.directory = if directory.is_empty() {
            launch.path.parent().unwrap_or(Path::new(".")).into()
        } else {
            directory.into()
        };
    }
    launch.path = expand(&launch.path)?;
    launch.directory = expand(&launch.directory)?;
    ensure!(
        launch.path.is_absolute()
            && launch.path.is_file()
            && launch.path.extension().is_some_and(|ext| ["exe", "com"]
                .iter()
                .any(|expected| ext.eq_ignore_ascii_case(expected))),
        i18n::t("error.admin_target")
    );
    Ok(launch)
}

pub fn run(path: &Path) -> Result<()> {
    let _apartment = Apartment::new()?;
    let launch = target(path)?;
    let executable = wide(&super::shell_path(&launch.path));
    let arguments = wide(&launch.arguments);
    let directory = wide(&super::shell_path(&launch.directory));
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(arguments.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        if error.code().0 as u32 == 0x800704C7 {
            anyhow::bail!(i18n::t("error.admin_cancelled"));
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_shortcut_target_arguments_and_working_directory_without_launching() {
        let _apartment = Apartment::new().unwrap();
        let root = std::env::temp_dir().join(format!("mytools-admin-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("测试 app.exe");
        std::fs::write(&executable, b"fixture").unwrap();
        let path = root.join("启动.lnk");
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap() };
        let file: IPersistFile = link.cast().unwrap();
        let args = OsString::from("--profile \"中文 folder\" & literal");
        unsafe {
            link.SetPath(PCWSTR(wide(executable.as_os_str()).as_ptr()))
                .unwrap();
            link.SetArguments(PCWSTR(wide(&args).as_ptr())).unwrap();
            link.SetWorkingDirectory(PCWSTR(wide(root.as_os_str()).as_ptr()))
                .unwrap();
            file.Save(PCWSTR(wide(path.as_os_str()).as_ptr()), true)
                .unwrap();
        }
        let actual = target(&path).unwrap();
        // ShellLink may expand an 8.3 path supplied by the runner temp directory.
        // Compare resolved filesystem paths rather than their spelling.
        assert_eq!(
            actual.path.canonicalize().unwrap(),
            executable.canonicalize().unwrap()
        );
        assert_eq!(actual.arguments, args);
        assert_eq!(
            actual.directory.canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
        assert_eq!(
            expand(Path::new("%WINDIR%")).unwrap(),
            PathBuf::from(std::env::var_os("WINDIR").unwrap())
        );
        assert!(target(&root.join("document.txt")).is_err());
        drop(file);
        drop(link);
        // Best effort: a virus scanner may still hold the new shortcut or executable open.
        let _ = std::fs::remove_dir_all(root);
    }
}
