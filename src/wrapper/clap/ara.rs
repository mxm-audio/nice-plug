//! MXM PATCH (ARA): a capability, not a fix. See `PATCHES.md`, *ARA*.
//!
//! ARA's CLAP companion API (`ARACLAP.h`, ARA SDK 2.3): the factory a host finds through
//! `clap_plugin_entry.get_factory`, listing each plugin of the library that offers ARA with its
//! `ARAFactory`, and the plugin extension a host binds an instance through. ARA's own types are
//! pointers here ([`crate::wrapper::ara`]).

#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;

use clap_sys::plugin::clap_plugin;

use super::ClapPlugin;

/// The ARA factory's id, and the draft id hosts before ARA 2.3 ask for.
pub const CLAP_EXT_ARA_FACTORY: &CStr = c"org.ara-audio.ara.factory/2";
pub const CLAP_EXT_ARA_FACTORY_COMPAT: &CStr = c"org.ara-audio.ara.factory.draft/2";
/// The plugin extension's id, and its draft id.
pub const CLAP_EXT_ARA_PLUGINEXTENSION: &CStr = c"org.ara-audio.ara.pluginextension/2";
pub const CLAP_EXT_ARA_PLUGINEXTENSION_COMPAT: &CStr = c"org.ara-audio.ara.pluginextension.draft/2";

/// `clap_ara_factory_t`.
#[repr(C)]
pub struct clap_ara_factory {
    pub get_factory_count: Option<unsafe extern "C" fn(factory: *const clap_ara_factory) -> u32>,
    pub get_ara_factory:
        Option<unsafe extern "C" fn(factory: *const clap_ara_factory, index: u32) -> *const c_void>,
    pub get_plugin_id:
        Option<unsafe extern "C" fn(factory: *const clap_ara_factory, index: u32) -> *const c_char>,
}

/// `clap_ara_plugin_extension_t`.
#[repr(C)]
pub struct clap_ara_plugin_extension {
    pub get_factory: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> *const c_void>,
    pub bind_to_document_controller: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            document_controller: *const c_void,
            known_roles: u32,
            assigned_roles: u32,
        ) -> *const c_void,
    >,
}

/// One plugin of the library that offers ARA, as the factory lists it.
#[doc(hidden)]
pub struct AraEntry {
    id: CString,
    factory: fn() -> *const c_void,
}

impl AraEntry {
    /// `P`'s entry, if it offers ARA.
    pub fn for_plugin<P: ClapPlugin>() -> Option<AraEntry> {
        P::CLAP_ARA.map(|hooks| AraEntry {
            id: CString::new(P::CLAP_ID).expect("a CLAP id without NUL"),
            factory: hooks.factory,
        })
    }
}

static ENTRIES: OnceLock<Vec<AraEntry>> = OnceLock::new();

static FACTORY: clap_ara_factory = clap_ara_factory {
    get_factory_count: Some(count),
    get_ara_factory: Some(get_ara_factory),
    get_plugin_id: Some(get_plugin_id),
};

/// The ARA factory for this library's plugins, or null when none offers ARA. The export macro
/// hands it every plugin's entry, once.
#[doc(hidden)]
pub fn factory(entries: impl FnOnce() -> Vec<Option<AraEntry>>) -> *const c_void {
    let entries = ENTRIES.get_or_init(|| entries().into_iter().flatten().collect());
    if entries.is_empty() {
        std::ptr::null()
    } else {
        &FACTORY as *const _ as *const c_void
    }
}

fn entries() -> &'static [AraEntry] {
    ENTRIES.get().map_or(&[], Vec::as_slice)
}

unsafe extern "C" fn count(_factory: *const clap_ara_factory) -> u32 {
    entries().len() as u32
}

unsafe extern "C" fn get_ara_factory(
    _factory: *const clap_ara_factory,
    index: u32,
) -> *const c_void {
    entries()
        .get(index as usize)
        .map_or(std::ptr::null(), |entry| (entry.factory)())
}

unsafe extern "C" fn get_plugin_id(_factory: *const clap_ara_factory, index: u32) -> *const c_char {
    entries()
        .get(index as usize)
        .map_or(std::ptr::null(), |entry| entry.id.as_ptr())
}
