//! MXM PATCH (presets): a plugin's preset files, offered to hosts through CLAP's preset discovery
//! and loaded through `preset-load`. A capability, not a fix. See `PATCHES.md`, *Presets*.
//!
//! The discovery factory needs no plugin instance: each plugin with [`ClapPresets`] gets one
//! provider, which declares its preset file type and the factory folder of its id and version, and
//! reads a file's name, category and plugin for the host's index. Loading goes through the plugin, by
//! [`ClapPresetLoader`], so a preset loads in a host exactly as in the plugin's own browser.

use clap_sys::factory::preset_discovery::{
    CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT, CLAP_PRESET_DISCOVERY_IS_USER_CONTENT,
    CLAP_PRESET_DISCOVERY_LOCATION_FILE, clap_preset_discovery_factory,
    clap_preset_discovery_filetype, clap_preset_discovery_indexer, clap_preset_discovery_location,
    clap_preset_discovery_location_kind, clap_preset_discovery_metadata_receiver,
    clap_preset_discovery_provider, clap_preset_discovery_provider_descriptor,
};
use clap_sys::universal_plugin_id::clap_universal_plugin_id;
use clap_sys::version::CLAP_VERSION;
#[cfg(feature = "editor")]
use nice_plug_core::context::gui::ParamSetter;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// How a plugin's preset files are found and read without an instance of it: what a host's
/// indexer asks of the preset-discovery factory.
#[derive(Clone, Copy)]
pub struct ClapPresets {
    /// A preset file's extension, without the dot, such as `json`.
    pub file_extension: &'static str,
    /// The folder of factory presets installed for the plugin `clap_id` at `version` (its
    /// `Plugin::VERSION`), if there is one.
    pub factory_folder: fn(clap_id: &str, version: &str) -> Option<PathBuf>,
    /// A file's metadata, for the plugin `clap_id`: `Ok(None)` for a file that is another
    /// plugin's or no preset at all, `Err` for one of its own that can't be read.
    pub metadata: fn(path: &Path, clap_id: &str) -> Result<Option<PresetMetadata>, String>,
}

/// What a host's index holds of one preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetMetadata {
    pub name: String,
    /// Given to the host as the extra information `category`.
    pub category: Option<String>,
}

/// Loads a preset file into the plugin, for a host's `preset-load`. Its writes go through the
/// editor's context, so it needs the `editor` feature.
#[cfg(feature = "editor")]
pub trait ClapPresetLoader: Send + Sync {
    /// Loads `path` through `setter`, on the main thread, as the plugin's own browser would. The
    /// error says why it wasn't loaded.
    fn load(&self, path: &Path, setter: &ParamSetter) -> Result<(), String>;
}

/// Whether `path` is a file directly inside `folder`, both resolved, links included: a factory
/// preset by where it is, never by what it says. A path that doesn't resolve is in no folder.
pub fn in_folder(path: &Path, folder: &Path) -> bool {
    match (path.canonicalize(), folder.canonicalize()) {
        (Ok(path), Ok(folder)) => path.is_file() && path.parent() == Some(folder.as_path()),
        _ => false,
    }
}

/// One plugin's preset provider, as the export macro lists them.
#[doc(hidden)]
pub struct PresetProvider {
    clap_id: CString,
    version: String,
    id: CString,
    name: CString,
    /// Kept for `descriptor.vendor`, which points into it.
    _vendor: CString,
    descriptor: clap_preset_discovery_provider_descriptor,
    presets: ClapPresets,
}

// SAFETY: the descriptor's pointers point into the provider's own strings, which never change.
unsafe impl Send for PresetProvider {}
unsafe impl Sync for PresetProvider {}

impl PresetProvider {
    /// The provider for a plugin with `presets`, or `None` for one without.
    pub fn for_plugin<P: super::ClapPlugin>() -> Option<Self> {
        Self::new(P::CLAP_ID, P::VERSION, P::NAME, P::VENDOR, P::CLAP_PRESETS?)
    }

    fn new(
        clap_id: &str,
        version: &str,
        name: &str,
        vendor: &str,
        presets: ClapPresets,
    ) -> Option<Self> {
        let clap_id = CString::new(clap_id).ok()?;
        let id = CString::new(format!("{}.presets", clap_id.to_str().ok()?)).ok()?;
        let name = CString::new(format!("{name} presets")).ok()?;
        let vendor = CString::new(vendor).ok()?;
        // A CString's bytes stay where they are when it moves, so these outlive the move into
        // the provider.
        let descriptor = clap_preset_discovery_provider_descriptor {
            clap_version: CLAP_VERSION,
            id: id.as_ptr(),
            name: name.as_ptr(),
            vendor: vendor.as_ptr(),
        };
        Some(Self {
            clap_id,
            version: version.to_owned(),
            id,
            name,
            _vendor: vendor,
            descriptor,
            presets,
        })
    }
}

impl PresetProvider {
    /// The plugin's factory folder, asked for by its id and version.
    fn factory_folder(&self) -> Option<PathBuf> {
        let clap_id = self.clap_id.to_str().ok()?;
        (self.presets.factory_folder)(clap_id, &self.version)
    }
}

static PROVIDERS: OnceLock<Vec<PresetProvider>> = OnceLock::new();

const FACTORY: clap_preset_discovery_factory = clap_preset_discovery_factory {
    count: Some(count),
    get_descriptor: Some(get_descriptor),
    create: Some(create),
};

/// The preset-discovery factory for this library's plugins, or null when none has presets. The
/// export macro hands it every plugin's provider, once.
#[doc(hidden)]
pub fn factory(providers: impl FnOnce() -> Vec<Option<PresetProvider>>) -> *const c_void {
    let providers = PROVIDERS.get_or_init(|| providers().into_iter().flatten().collect());
    if providers.is_empty() {
        std::ptr::null()
    } else {
        &FACTORY as *const _ as *const c_void
    }
}

fn providers() -> &'static [PresetProvider] {
    PROVIDERS.get().map_or(&[], Vec::as_slice)
}

unsafe extern "C" fn count(_factory: *const clap_preset_discovery_factory) -> u32 {
    providers().len() as u32
}

unsafe extern "C" fn get_descriptor(
    _factory: *const clap_preset_discovery_factory,
    index: u32,
) -> *const clap_preset_discovery_provider_descriptor {
    providers()
        .get(index as usize)
        .map_or(std::ptr::null(), |provider| &provider.descriptor)
}

unsafe extern "C" fn create(
    _factory: *const clap_preset_discovery_factory,
    indexer: *const clap_preset_discovery_indexer,
    provider_id: *const c_char,
) -> *const clap_preset_discovery_provider {
    if indexer.is_null() || provider_id.is_null() {
        return std::ptr::null();
    }
    let wanted = unsafe { CStr::from_ptr(provider_id) };
    match providers()
        .iter()
        .find(|provider| provider.id.as_c_str() == wanted)
    {
        Some(entry) => Provider::create(entry, indexer),
        None => std::ptr::null(),
    }
}

/// A provider the host created: its CLAP side, and what it serves.
struct Provider {
    clap: clap_preset_discovery_provider,
    indexer: *const clap_preset_discovery_indexer,
    entry: &'static PresetProvider,
}

impl Provider {
    /// Boxed, its `provider_data` pointing at itself; `destroy` frees it.
    fn create(
        entry: &'static PresetProvider,
        indexer: *const clap_preset_discovery_indexer,
    ) -> *const clap_preset_discovery_provider {
        let provider = Box::into_raw(Box::new(Self {
            clap: clap_preset_discovery_provider {
                desc: &entry.descriptor,
                provider_data: std::ptr::null_mut(),
                init: Some(init),
                destroy: Some(destroy),
                get_metadata: Some(get_metadata),
                get_extension: Some(get_extension),
            },
            indexer,
            entry,
        }));
        // SAFETY: just allocated, and nothing else holds it yet.
        unsafe {
            (*provider).clap.provider_data = provider as *mut c_void;
            &(*provider).clap
        }
    }

    /// # Safety
    /// `provider` is one [`Provider::create`] made and `destroy` hasn't freed.
    unsafe fn of<'a>(provider: *const clap_preset_discovery_provider) -> Option<&'a Self> {
        if provider.is_null() {
            return None;
        }
        let data = unsafe { (*provider).provider_data } as *const Self;
        (!data.is_null()).then(|| unsafe { &*data })
    }
}

unsafe extern "C" fn init(provider: *const clap_preset_discovery_provider) -> bool {
    let Some(this) = (unsafe { Provider::of(provider) }) else {
        return false;
    };
    let indexer = unsafe { &*this.indexer };
    let presets = &this.entry.presets;
    let (Ok(name), Ok(extension)) = (
        CString::new(format!("{} preset", this.entry.name.to_string_lossy())),
        CString::new(presets.file_extension),
    ) else {
        return false;
    };
    let filetype = clap_preset_discovery_filetype {
        name: name.as_ptr(),
        description: std::ptr::null(),
        file_extension: extension.as_ptr(),
    };
    if let Some(declare) = indexer.declare_filetype
        && !unsafe { declare(indexer, &filetype) }
    {
        return false;
    }
    // A plugin installed without its folder has no factory presets: nothing to declare.
    let Some(folder) = this.entry.factory_folder().filter(|folder| folder.is_dir()) else {
        return true;
    };
    let Some(path) = folder.to_str().and_then(|path| CString::new(path).ok()) else {
        return true;
    };
    let location = clap_preset_discovery_location {
        flags: CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
        name: c"Factory presets".as_ptr(),
        kind: CLAP_PRESET_DISCOVERY_LOCATION_FILE,
        location: path.as_ptr(),
    };
    match indexer.declare_location {
        Some(declare) => unsafe { declare(indexer, &location) },
        None => true,
    }
}

unsafe extern "C" fn destroy(provider: *const clap_preset_discovery_provider) {
    if let Some(this) = unsafe { Provider::of(provider) } {
        // SAFETY: made by `Box::into_raw` in `Provider::create`, freed once, here.
        drop(unsafe { Box::from_raw(this as *const Provider as *mut Provider) });
    }
}

unsafe extern "C" fn get_metadata(
    provider: *const clap_preset_discovery_provider,
    location_kind: clap_preset_discovery_location_kind,
    location: *const c_char,
    receiver: *const clap_preset_discovery_metadata_receiver,
) -> bool {
    let Some(this) = (unsafe { Provider::of(provider) }) else {
        return false;
    };
    if receiver.is_null() {
        return false;
    }
    let receiver_ref = unsafe { &*receiver };
    let fail = |why: &str| {
        if let (Some(on_error), Ok(why)) = (receiver_ref.on_error, CString::new(why)) {
            unsafe { on_error(receiver, 0, why.as_ptr()) };
        }
        false
    };
    if location_kind != CLAP_PRESET_DISCOVERY_LOCATION_FILE || location.is_null() {
        return fail("only preset files are indexed");
    }
    let Ok(path) = (unsafe { CStr::from_ptr(location) }).to_str() else {
        return fail("the preset's path isn't UTF-8");
    };
    let path = PathBuf::from(path);
    let presets = &this.entry.presets;
    let clap_id = this.entry.clap_id.to_str().unwrap_or_default();
    let metadata = match (presets.metadata)(&path, clap_id) {
        Ok(Some(metadata)) => metadata,
        // Another plugin's file, or no preset: nothing to index, and no error.
        Ok(None) => return true,
        Err(why) => return fail(&format!("{}: {why}", path.display())),
    };
    let Ok(name) = CString::new(metadata.name) else {
        return fail("the preset's name holds a NUL");
    };
    let Some(begin) = receiver_ref.begin_preset else {
        return false;
    };
    if !unsafe { begin(receiver, name.as_ptr(), std::ptr::null()) } {
        return true;
    }
    if let Some(add_plugin_id) = receiver_ref.add_plugin_id {
        let plugin = clap_universal_plugin_id {
            abi: c"clap".as_ptr(),
            id: this.entry.clap_id.as_ptr(),
        };
        unsafe { add_plugin_id(receiver, &plugin) };
    }
    if let Some(set_flags) = receiver_ref.set_flags {
        let factory = this
            .entry
            .factory_folder()
            .is_some_and(|folder| in_folder(&path, &folder));
        let flags = if factory {
            CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT
        } else {
            CLAP_PRESET_DISCOVERY_IS_USER_CONTENT
        };
        unsafe { set_flags(receiver, flags) };
    }
    if let (Some(add_extra_info), Some(category)) = (
        receiver_ref.add_extra_info,
        metadata
            .category
            .and_then(|category| CString::new(category).ok()),
    ) {
        unsafe { add_extra_info(receiver, c"category".as_ptr(), category.as_ptr()) };
    }
    true
}

unsafe extern "C" fn get_extension(
    _provider: *const clap_preset_discovery_provider,
    _extension_id: *const c_char,
) -> *const c_void {
    std::ptr::null()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The test plugin's factory folder; a `fn` can't capture one.
    static FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);

    /// The folder, for the test plugin's id and version only.
    fn folder(clap_id: &str, version: &str) -> Option<PathBuf> {
        (clap_id == "dk.test.synth" && version == "1.2.3")
            .then(|| FOLDER.lock().unwrap().clone())
            .flatten()
    }

    /// `name|category|plugin` in a `.txt`.
    fn metadata(path: &Path, clap_id: &str) -> Result<Option<PresetMetadata>, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let parts: Vec<&str> = text.split('|').collect();
        match parts.as_slice() {
            [_, _, plugin] if *plugin != clap_id => Ok(None),
            [name, category, _] => Ok(Some(PresetMetadata {
                name: (*name).to_owned(),
                category: (!category.is_empty()).then(|| (*category).to_owned()),
            })),
            _ => Err("not a preset".to_owned()),
        }
    }

    const PRESETS: ClapPresets = ClapPresets {
        file_extension: "txt",
        factory_folder: folder,
        metadata,
    };

    /// What the indexer and the receiver were told.
    #[derive(Default, Debug)]
    struct Heard {
        filetypes: Vec<String>,
        locations: Vec<(u32, String)>,
        presets: Vec<String>,
        plugin_ids: Vec<(String, String)>,
        flags: Vec<u32>,
        extra: Vec<(String, String)>,
        errors: Vec<String>,
    }

    fn text(pointer: *const c_char) -> String {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }

    fn heard<'a>(data: *mut c_void) -> &'a mut Heard {
        unsafe { &mut *(data as *mut Heard) }
    }

    unsafe extern "C" fn declare_filetype(
        indexer: *const clap_preset_discovery_indexer,
        filetype: *const clap_preset_discovery_filetype,
    ) -> bool {
        let heard = heard(unsafe { (*indexer).indexer_data });
        heard
            .filetypes
            .push(text(unsafe { (*filetype).file_extension }));
        true
    }

    unsafe extern "C" fn declare_location(
        indexer: *const clap_preset_discovery_indexer,
        location: *const clap_preset_discovery_location,
    ) -> bool {
        let heard = heard(unsafe { (*indexer).indexer_data });
        let location = unsafe { &*location };
        heard
            .locations
            .push((location.flags, text(location.location)));
        true
    }

    unsafe extern "C" fn on_error(
        receiver: *const clap_preset_discovery_metadata_receiver,
        _os_error: i32,
        message: *const c_char,
    ) {
        heard(unsafe { (*receiver).receiver_data })
            .errors
            .push(text(message));
    }

    unsafe extern "C" fn begin_preset(
        receiver: *const clap_preset_discovery_metadata_receiver,
        name: *const c_char,
        _load_key: *const c_char,
    ) -> bool {
        heard(unsafe { (*receiver).receiver_data })
            .presets
            .push(text(name));
        true
    }

    unsafe extern "C" fn add_plugin_id(
        receiver: *const clap_preset_discovery_metadata_receiver,
        plugin_id: *const clap_universal_plugin_id,
    ) {
        let id = unsafe { &*plugin_id };
        heard(unsafe { (*receiver).receiver_data })
            .plugin_ids
            .push((text(id.abi), text(id.id)));
    }

    unsafe extern "C" fn set_flags(
        receiver: *const clap_preset_discovery_metadata_receiver,
        flags: u32,
    ) {
        heard(unsafe { (*receiver).receiver_data })
            .flags
            .push(flags);
    }

    unsafe extern "C" fn add_extra_info(
        receiver: *const clap_preset_discovery_metadata_receiver,
        key: *const c_char,
        value: *const c_char,
    ) {
        heard(unsafe { (*receiver).receiver_data })
            .extra
            .push((text(key), text(value)));
    }

    fn indexer_for(heard: &mut Heard) -> clap_preset_discovery_indexer {
        clap_preset_discovery_indexer {
            clap_version: CLAP_VERSION,
            name: c"test".as_ptr(),
            vendor: std::ptr::null(),
            url: std::ptr::null(),
            version: std::ptr::null(),
            indexer_data: heard as *mut Heard as *mut c_void,
            declare_filetype: Some(declare_filetype),
            declare_location: Some(declare_location),
            declare_soundpack: None,
            get_extension: None,
        }
    }

    fn receiver_for(heard: &mut Heard) -> clap_preset_discovery_metadata_receiver {
        clap_preset_discovery_metadata_receiver {
            receiver_data: heard as *mut Heard as *mut c_void,
            on_error: Some(on_error),
            begin_preset: Some(begin_preset),
            add_plugin_id: Some(add_plugin_id),
            set_soundpack_id: None,
            set_flags: Some(set_flags),
            add_creator: None,
            set_description: None,
            set_timestamps: None,
            add_feature: None,
            add_extra_info: Some(add_extra_info),
        }
    }

    fn metadata_of(provider: *const clap_preset_discovery_provider, path: &Path) -> (bool, Heard) {
        let mut heard = Heard::default();
        let receiver = receiver_for(&mut heard);
        let location = CString::new(path.to_str().unwrap()).unwrap();
        let read = unsafe {
            get_metadata(
                provider,
                CLAP_PRESET_DISCOVERY_LOCATION_FILE,
                location.as_ptr(),
                &receiver,
            )
        };
        (read, heard)
    }

    /// One test, as the folder is one static: the provider declares the type and the folder of
    /// its plugin's id and version, reads a preset's metadata, passes over another plugin's file, says why it can't read a
    /// broken one, tells a file outside the folder from a factory one, and declares no folder
    /// when there is none.
    #[test]
    fn the_provider_indexes_the_factory_folder() {
        let root = std::env::temp_dir().join(format!("nice-plug-presets-{}", std::process::id()));
        let factory = root.join("factory");
        std::fs::create_dir_all(&factory).unwrap();
        std::fs::write(factory.join("bass.txt"), "Deep bass|Bass|dk.test.synth").unwrap();
        std::fs::write(factory.join("other.txt"), "Other|Pad|dk.test.other").unwrap();
        std::fs::write(factory.join("broken.txt"), "broken").unwrap();
        std::fs::write(root.join("mine.txt"), "Mine||dk.test.synth").unwrap();
        *FOLDER.lock().unwrap() = Some(factory.clone());

        let entry: &'static PresetProvider = Box::leak(Box::new(
            PresetProvider::new("dk.test.synth", "1.2.3", "Synth", "Test", PRESETS).unwrap(),
        ));
        assert_eq!(text(entry.descriptor.id), "dk.test.synth.presets");

        let mut heard = Heard::default();
        let indexer = indexer_for(&mut heard);
        let provider = Provider::create(entry, &indexer);
        assert!(unsafe { init(provider) });
        assert_eq!(heard.filetypes, ["txt"]);
        assert_eq!(
            heard.locations,
            [(
                CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT,
                factory.to_str().unwrap().to_owned()
            )]
        );

        let (read, bass) = metadata_of(provider, &factory.join("bass.txt"));
        assert!(read);
        assert_eq!(bass.presets, ["Deep bass"]);
        assert_eq!(
            bass.plugin_ids,
            [("clap".to_owned(), "dk.test.synth".to_owned())]
        );
        assert_eq!(bass.flags, [CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT]);
        assert_eq!(bass.extra, [("category".to_owned(), "Bass".to_owned())]);

        let (read, other) = metadata_of(provider, &factory.join("other.txt"));
        assert!(read, "another plugin's file is no error");
        assert!(other.presets.is_empty());

        let (read, broken) = metadata_of(provider, &factory.join("broken.txt"));
        assert!(!read);
        assert!(broken.presets.is_empty());
        assert!(broken.errors[0].contains("not a preset"), "{broken:?}");

        let (read, mine) = metadata_of(provider, &root.join("mine.txt"));
        assert!(read);
        assert_eq!(mine.flags, [CLAP_PRESET_DISCOVERY_IS_USER_CONTENT]);
        assert!(mine.extra.is_empty(), "no category, none told");
        unsafe { destroy(provider) };

        // Installed without its folder: the type, and no location.
        *FOLDER.lock().unwrap() = Some(root.join("missing"));
        let mut heard = Heard::default();
        let indexer = indexer_for(&mut heard);
        let provider = Provider::create(entry, &indexer);
        assert!(unsafe { init(provider) });
        assert_eq!(heard.filetypes, ["txt"]);
        assert!(heard.locations.is_empty());
        unsafe { destroy(provider) };

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_a_file_directly_inside_the_folder_is_in_it() {
        let root = std::env::temp_dir().join(format!("nice-plug-in-folder-{}", std::process::id()));
        let factory = root.join("factory");
        std::fs::create_dir_all(factory.join("deeper")).unwrap();
        std::fs::write(factory.join("a.txt"), "").unwrap();
        std::fs::write(factory.join("deeper").join("b.txt"), "").unwrap();
        std::fs::write(root.join("c.txt"), "").unwrap();
        assert!(in_folder(&factory.join("a.txt"), &factory));
        assert!(in_folder(
            &factory.join("deeper").join("..").join("a.txt"),
            &factory
        ));
        assert!(!in_folder(&factory.join("deeper").join("b.txt"), &factory));
        assert!(!in_folder(&factory.join("..").join("c.txt"), &factory));
        assert!(!in_folder(&factory.join("missing.txt"), &factory));
        let _ = std::fs::remove_dir_all(&root);
    }
}
