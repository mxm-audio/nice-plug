//! MXM PATCH (presets): where the plugin was loaded from. See `PATCHES.md`, *Presets*.
//!
//! A plugin's own files (its factory presets) are installed with its bundle, so it finds them
//! from where its code was loaded: the module holding this function, read from the system, then
//! the outermost `.clap` or `.vst3` around it. The same for both formats, on every platform, and
//! for whichever plugin of a library asks.

use std::path::{Path, PathBuf};

/// The plugin format a bundle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    Clap,
    Vst3,
}

/// Where the running plugin was loaded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginLocation {
    pub format: PluginFormat,
    /// The `.clap` (a file on Windows and Linux, a bundle on macOS) or the `.vst3` bundle.
    pub bundle: PathBuf,
}

/// Where the running plugin was loaded from, or `None` when it wasn't loaded from a bundle, as in
/// a unit test.
pub fn plugin_location() -> Option<PluginLocation> {
    bundle_of(&module_path()?)
}

/// The outermost `.clap` or `.vst3` holding `module`, the module itself included: a VST3 on
/// Windows is `X.vst3/Contents/x86_64-win/X.vst3`, whose bundle is the outer one.
pub fn bundle_of(module: &Path) -> Option<PluginLocation> {
    module
        .ancestors()
        .filter_map(|path| {
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            let format = match extension.as_str() {
                "clap" => PluginFormat::Clap,
                "vst3" => PluginFormat::Vst3,
                _ => return None,
            };
            Some(PluginLocation {
                format,
                bundle: path.to_path_buf(),
            })
        })
        .last()
}

/// The file of the module (the DLL, shared library or dylib) this code was loaded from.
#[cfg(target_os = "windows")]
fn module_path() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };
    use windows::core::PCWSTR;

    let mut module = HMODULE::default();
    let address = module_path as *const () as *const u16;
    // SAFETY: with FROM_ADDRESS the name is an address inside the module, this function's; with
    // UNCHANGED_REFCOUNT the handle is borrowed, so nothing is freed.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address),
            &mut module,
        )
    }
    .ok()?;
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: the buffer is ours and its length is passed with it.
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

/// The file of the module (the DLL, shared library or dylib) this code was loaded from.
#[cfg(target_family = "unix")]
fn module_path() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: `dladdr` fills `info` for an address inside a loaded object, this function's.
    if unsafe { libc::dladdr(module_path as *const libc::c_void, &mut info) } == 0
        || info.dli_fname.is_null()
    {
        return None;
    }
    // SAFETY: `dli_fname` is the object's path, a C string the loader keeps.
    let name = unsafe { CStr::from_ptr(info.dli_fname) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
}

#[cfg(not(any(target_os = "windows", target_family = "unix")))]
fn module_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str) -> Option<(PluginFormat, PathBuf)> {
        bundle_of(Path::new(path)).map(|found| (found.format, found.bundle))
    }

    #[test]
    fn a_clap_file_is_its_own_bundle() {
        assert_eq!(
            at("C:/Program Files/Common Files/CLAP/mxm-mono-01.clap"),
            Some((
                PluginFormat::Clap,
                PathBuf::from("C:/Program Files/Common Files/CLAP/mxm-mono-01.clap")
            ))
        );
    }

    #[test]
    fn a_module_inside_a_bundle_is_found_in_the_outermost() {
        assert_eq!(
            at("C:/VST3/mxm-mono-01.vst3/Contents/x86_64-win/mxm-mono-01.vst3"),
            Some((
                PluginFormat::Vst3,
                PathBuf::from("C:/VST3/mxm-mono-01.vst3")
            ))
        );
        assert_eq!(
            at("/Library/Audio/Plug-Ins/CLAP/mxm-mono-01.clap/Contents/MacOS/mxm-mono-01"),
            Some((
                PluginFormat::Clap,
                PathBuf::from("/Library/Audio/Plug-Ins/CLAP/mxm-mono-01.clap")
            ))
        );
    }

    #[test]
    fn a_module_in_no_bundle_has_no_location() {
        assert_eq!(at("E:/newDAWn/target/debug/deps/tests.exe"), None);
        // A unit test is no plugin.
        assert_eq!(plugin_location(), None);
    }
}
