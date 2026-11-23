//! MXM PATCH (ARA): `PATCHES.md`, *ARA*. An ARA plugin and a plain one exported together through
//! both formats, read as a host reads them: CLAP's ARA factory lists only the ARA plugin; VST3's
//! factory lists its ARA main factory class after the plugins' classes, whose `IMainFactory`
//! hands over its `ARAFactory`; its component answers `IPlugInEntryPoint` and `IPlugInEntryPoint2`
//! and binds through the plugin's hook; the plain plugin's answers neither.

use std::ffi::{CStr, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nice_plug::prelude::*;
use nice_plug::wrapper::clap::ara::{
    CLAP_EXT_ARA_FACTORY, CLAP_EXT_ARA_FACTORY_COMPAT, clap_ara_factory,
};
use nice_plug::wrapper::vst3::ara::{
    ARA_MAIN_FACTORY_CLASS, IMainFactory, IPlugInEntryPoint, IPlugInEntryPoint2,
};
use nice_plug::wrapper::vst3::vst3::Steinberg::Vst::IComponent;
use nice_plug::wrapper::vst3::vst3::Steinberg::{
    FIDString, IPluginFactory, IPluginFactoryTrait, PClassInfo, kResultOk,
};
use nice_plug::wrapper::vst3::vst3::{ComPtr, Interface};

/// Stands for an `ARAFactory`: only its address is handed on.
static FACTORY: u8 = 0;
/// Stands for an `ARAPlugInExtensionInstance`.
static INSTANCE: u8 = 0;
/// The last binding the hook saw: the document controller's address and the roles, packed.
static BOUND: AtomicUsize = AtomicUsize::new(0);

fn factory() -> *const c_void {
    (&raw const FACTORY).cast()
}

#[derive(Default)]
struct NoParams;

unsafe impl Params for NoParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        Vec::new()
    }
}

macro_rules! test_plugin {
    ($name:ident, $plugin_name:literal, $id:literal, $cid:expr, $ara:expr) => {
        #[derive(Default)]
        struct $name {
            params: Arc<NoParams>,
        }

        impl Plugin for $name {
            const NAME: &'static str = $plugin_name;
            const VENDOR: &'static str = "MXM";
            const URL: &'static str = "";
            const EMAIL: &'static str = "";
            const VERSION: &'static str = "1";
            const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
                main_input_channels: NonZeroU32::new(2),
                main_output_channels: NonZeroU32::new(2),
                ..AudioIOLayout::const_default()
            }];
            type Editor = ();
            type SysExMessage = ();
            type BackgroundTask = ();

            fn params(&self) -> Arc<dyn Params> {
                self.params.clone()
            }

            fn process(
                &mut self,
                _buffer: &mut Buffer,
                _aux: &mut AuxiliaryBuffers,
                _context: &mut impl ProcessContext<Self>,
            ) -> ProcessStatus {
                ProcessStatus::Normal
            }
        }

        impl ClapPlugin for $name {
            const CLAP_ID: &'static str = $id;
            const CLAP_DESCRIPTION: Option<&'static str> = None;
            const CLAP_MANUAL_URL: Option<&'static str> = None;
            const CLAP_SUPPORT_URL: Option<&'static str> = None;
            const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect];
            const CLAP_ARA: Option<AraHooks<Self>> = $ara;
        }

        impl Vst3Plugin for $name {
            const VST3_CLASS_ID: [u8; 16] = $cid;
            const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
            const VST3_ARA: Option<AraHooks<Self>> = $ara;
        }
    };
}

test_plugin!(
    Offering,
    "Offering",
    "dk.mxm.test.ara",
    [2; 16],
    Some(AraHooks {
        factory,
        bind: |_plugin: &mut Offering, controller, known, assigned| {
            BOUND.store(
                (controller as usize) << 8 | (known as usize) << 4 | assigned as usize,
                Ordering::SeqCst,
            );
            (&raw const INSTANCE).cast()
        },
        vst3_main_factory_class_id: [3; 16],
    })
);
test_plugin!(Plain, "Plain", "dk.mxm.test.plain", [4; 16], None);

nice_export_clap!(Offering, Plain);
nice_export_vst3!(Offering, Plain);

#[test]
fn clap_lists_only_the_plugin_that_offers_ara() {
    let get_factory = clap_entry.get_factory.unwrap();
    let ara = unsafe { get_factory(CLAP_EXT_ARA_FACTORY.as_ptr()) }.cast::<clap_ara_factory>();
    assert!(!ara.is_null());
    assert_eq!(
        unsafe { get_factory(CLAP_EXT_ARA_FACTORY_COMPAT.as_ptr()) },
        ara.cast(),
        "the draft id finds the same factory"
    );
    let ara = unsafe { &*ara };
    let this = ara as *const clap_ara_factory;
    assert_eq!(unsafe { ara.get_factory_count.unwrap()(this) }, 1);
    assert_eq!(
        unsafe { CStr::from_ptr(ara.get_plugin_id.unwrap()(this, 0)) },
        c"dk.mxm.test.ara"
    );
    assert_eq!(unsafe { ara.get_ara_factory.unwrap()(this, 0) }, factory());
    assert!(unsafe { ara.get_plugin_id.unwrap()(this, 1) }.is_null());
}

fn vst3_factory() -> ComPtr<IPluginFactory> {
    unsafe { ComPtr::from_raw(GetPluginFactory().cast::<IPluginFactory>()) }.unwrap()
}

fn create<I: Interface>(factory: &ComPtr<IPluginFactory>, cid: &[u8; 16]) -> Option<ComPtr<I>> {
    let mut object: *mut c_void = std::ptr::null_mut();
    let result = unsafe {
        factory.createInstance(
            cid.as_ptr() as FIDString,
            I::IID.as_ptr() as FIDString,
            &mut object,
        )
    };
    (result == kResultOk).then(|| unsafe { ComPtr::from_raw(object.cast::<I>()) }.unwrap())
}

#[test]
fn vst3_lists_the_ara_main_factory_class_after_the_plugins() {
    let plugins = vst3_factory();
    assert_eq!(unsafe { plugins.countClasses() }, 3);
    let class = |index| {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { plugins.getClassInfo(index, &mut info) }, kResultOk);
        let text = |chars: &[std::ffi::c_char]| {
            unsafe { CStr::from_ptr(chars.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        };
        (
            info.cid.map(|b| b as u8),
            text(&info.category),
            text(&info.name),
        )
    };
    assert_eq!(class(0).1, "Audio Module Class");
    assert_eq!(class(1).1, "Audio Module Class");
    let (cid, category, name) = class(2);
    assert_eq!(cid, Offering::PLATFORM_VST3_ARA_CLASS_ID);
    assert_eq!(category, ARA_MAIN_FACTORY_CLASS);
    assert_eq!(name, "Offering", "the plugin's own name, as ARA asks");

    let main = create::<IMainFactory>(&plugins, &Offering::PLATFORM_VST3_ARA_CLASS_ID).unwrap();
    let ptr = main.as_ptr();
    assert_eq!(unsafe { ((*(*ptr).vtbl).getFactory)(ptr) }, factory());
}

#[test]
fn vst3_binds_the_plugin_that_offers_ara_and_not_the_other() {
    let plugins = vst3_factory();
    let component = create::<IComponent>(&plugins, &Offering::PLATFORM_VST3_CLASS_ID).unwrap();
    let entry = component
        .cast::<IPlugInEntryPoint>()
        .expect("IPlugInEntryPoint");
    let ptr = entry.as_ptr();
    assert_eq!(unsafe { ((*(*ptr).vtbl).getFactory)(ptr) }, factory());
    let entry2 = component
        .cast::<IPlugInEntryPoint2>()
        .expect("IPlugInEntryPoint2");
    let ptr = entry2.as_ptr();
    let controller = 0x1234usize as *const c_void;
    let instance =
        unsafe { ((*(*ptr).vtbl).bindToDocumentControllerWithRoles)(ptr, controller, 7, 1) };
    assert_eq!(instance, (&raw const INSTANCE).cast());
    assert_eq!(BOUND.load(Ordering::SeqCst), 0x1234 << 8 | 7 << 4 | 1);

    let plain = create::<IComponent>(&plugins, &Plain::PLATFORM_VST3_CLASS_ID).unwrap();
    assert!(plain.cast::<IPlugInEntryPoint>().is_none(), "not offered");
    assert!(plain.cast::<IPlugInEntryPoint2>().is_none(), "not offered");
}
