//! MXM PATCH (ARA): a capability, not a fix. See `PATCHES.md`, *ARA*.
//!
//! A plugin that offers ARA 2 (Celemony's Audio Random Access, `ARAInterface.h`), through its
//! companion APIs: CLAP's (`ARACLAP.h`: the factory `org.ara-audio.ara.factory/2` and the plugin
//! extension `org.ara-audio.ara.pluginextension/2`) and VST3's (`ARAVST3.h`: the class
//! `kARAMainFactoryClass` answering `IMainFactory`, and `IPlugInEntryPoint` and
//! `IPlugInEntryPoint2` on the component). nice-plug knows ARA's objects only as pointers: the
//! plugin brings its own ARA implementation (mxm-fx-pitch's `mxm-ara`), and the wrappers only
//! hand the host its factory and bind an instance to the host's document controller.
//!
//! A plugin opts in through `ClapPlugin::CLAP_ARA` and `Vst3Plugin::VST3_ARA`, usually the same
//! [`AraHooks`]; one that doesn't is unchanged in both formats.

use std::ffi::c_void;

/// What a plugin that offers ARA gives its wrappers.
pub struct AraHooks<P> {
    /// Its ARA factory (`const ARAFactory*`), alive as long as the library is loaded and the same
    /// for every instance.
    pub factory: fn() -> *const c_void,
    /// Binds an instance to a host's document controller (an `ARADocumentControllerRef`) in the
    /// roles given (`ARAPlugInInstanceRoleFlags`: those the host knows, then those it assigns),
    /// on the main thread, once, before the instance's first activation, state load or editor (both
    /// companion APIs require it): the instance's `const ARAPlugInExtensionInstance*`, alive until
    /// the instance is destroyed, or null where it can't be bound.
    pub bind: fn(&mut P, *const c_void, u32, u32) -> *const c_void,
    /// The VST3 class ID of its ARA main factory class: permanent, as `VST3_CLASS_ID` is, and
    /// distinct from it.
    pub vst3_main_factory_class_id: [u8; 16],
}

impl<P> Clone for AraHooks<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for AraHooks<P> {}
