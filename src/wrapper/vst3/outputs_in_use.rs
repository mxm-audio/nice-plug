//! MXM PATCH (outputs in use): a capability, not a fix. See `PATCHES.md`, *Outputs in use*.
//!
//! The VST3 side of [`OutputsInUse`][crate::wrapper::outputs_in_use::OutputsInUse]: two MXM
//! interfaces, through which a host learns the output buses after the main one that something in
//! the plugin sends to now, each with a name for what plays there. VST3 has nothing like it: a
//! host can tell a plugin which buses it listens to (`IComponent::activateBus`), never the other
//! way round. The convention is mxm-kit's `docs/plugin-conventions.md`, *Outputs in use*, which
//! declares them in C++:
//!
//! ```cpp
//! namespace MXM {
//! // On the component: the output buses something in the plugin sends to.
//! class IOutputsInUse : public Steinberg::FUnknown {
//! public:
//!     // [UI-thread] How many output buses after the main one something in the plugin sends to
//!     // now, whatever arrangement is active. kNotImplemented from a plugin that doesn't say.
//!     virtual tresult PLUGIN_API getOutputCount (int32* count) = 0;
//!     // [UI-thread] One of them: its output bus index, and a name for what plays there.
//!     virtual tresult PLUGIN_API getOutputInUse (int32 index, int32* busIndex, String128 name) = 0;
//!     // [UI-thread] What sends whatever plays on output bus busIndex back to the main output: one
//!     // parameter value for the host to set as it sets any. kResultFalse past the last.
//!     virtual tresult PLUGIN_API getToMain (int32 busIndex, int32 index, ParamID* id,
//!                                           ParamValue* valueNormalized) = 0;
//!     static const FUID iid;
//! };
//! DECLARE_CLASS_IID (IOutputsInUse, 0x19786A5F, 0xE141689D, 0x5D908823, 0xCE588BF6)
//!
//! // On the host's IComponentHandler.
//! class IOutputsInUseHandler : public Steinberg::FUnknown {
//! public:
//!     // [UI-thread] The outputs in use, or their names, may have changed: read them again.
//!     virtual tresult PLUGIN_API outputsInUseChanged () = 0;
//!     static const FUID iid;
//! };
//! DECLARE_CLASS_IID (IOutputsInUseHandler, 0x30EA7F50, 0x4E3D0C7E, 0x498B4086, 0xC95A6C81)
//! }
//! ```
//!
//! The `vst3` crate generates only Steinberg's own interfaces, so these two are written here as it
//! writes them (its `IInfoListener`): a vtable after `FUnknown`'s, an [`Interface`] with the ID
//! `vst3::uid` lays out as the SDK's `DECLARE_CLASS_IID` does on each platform, and a
//! [`Construct`] for a Rust class that implements one.

#![allow(non_snake_case)]

use std::ffi::c_void;
use vst3::Steinberg::Vst::{ParamID, ParamValue, String128, TChar};
use vst3::Steinberg::{FUnknown, FUnknownVtbl, TUID, int32, tresult};
use vst3::com_scrape_types::{Class, Construct, Guid, Header, Inherits, Unknown, Wrapper};
use vst3::{ComPtr, Interface};

/// A [`TUID`] as the [`Guid`] an [`Interface`] declares.
const fn guid(tuid: TUID) -> Guid {
    let mut guid = [0; 16];
    let mut i = 0;
    while i < 16 {
        guid[i] = tuid[i] as u8;
        i += 1;
    }
    guid
}

/// The plugin's interface, on its component.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IOutputsInUse {
    pub vtbl: *const IOutputsInUseVtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IOutputsInUseVtbl {
    pub base: FUnknownVtbl,
    pub getOutputCount:
        unsafe extern "system" fn(this: *mut IOutputsInUse, count: *mut int32) -> tresult,
    pub getOutputInUse: unsafe extern "system" fn(
        this: *mut IOutputsInUse,
        index: int32,
        bus_index: *mut int32,
        name: *mut String128,
    ) -> tresult,
    pub getToMain: unsafe extern "system" fn(
        this: *mut IOutputsInUse,
        bus_index: int32,
        index: int32,
        id: *mut ParamID,
        value: *mut ParamValue,
    ) -> tresult,
}

/// What a class implementing [`IOutputsInUse`] does.
///
/// Each method is called through the interface's vtable, its pointers as valid as the C++
/// declaration says, as the `vst3` crate's generated traits are.
#[allow(clippy::missing_safety_doc)]
pub trait IOutputsInUseTrait {
    unsafe fn getOutputCount(&self, count: *mut int32) -> tresult;
    unsafe fn getOutputInUse(
        &self,
        index: int32,
        bus_index: *mut int32,
        name: *mut String128,
    ) -> tresult;
    unsafe fn getToMain(
        &self,
        bus_index: int32,
        index: int32,
        id: *mut ParamID,
        value: *mut ParamValue,
    ) -> tresult;
}

/// The host's interface, on its `IComponentHandler`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IOutputsInUseHandler {
    pub vtbl: *const IOutputsInUseHandlerVtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IOutputsInUseHandlerVtbl {
    pub base: FUnknownVtbl,
    pub outputsInUseChanged: unsafe extern "system" fn(this: *mut IOutputsInUseHandler) -> tresult,
}

/// What a class implementing [`IOutputsInUseHandler`] does: a host's, or a test's.
///
/// Each method is called through the interface's vtable, its pointers as valid as the C++
/// declaration says, as the `vst3` crate's generated traits are.
#[allow(clippy::missing_safety_doc)]
pub trait IOutputsInUseHandlerTrait {
    unsafe fn outputsInUseChanged(&self) -> tresult;
}

/// Tells the host the outputs in use may have changed, on the main thread.
pub(crate) fn tell_changed(handler: &ComPtr<IOutputsInUseHandler>) {
    let ptr = handler.as_ptr();
    // SAFETY: the host's live object, called through its own vtable on the main thread.
    unsafe { ((*(*ptr).vtbl).outputsInUseChanged)(ptr) };
}

/// Writes `name` into `dest` as NUL-terminated UTF-16, cut where it no longer fits, never between
/// the two halves of a surrogate pair.
pub(crate) fn copy_name(dest: &mut String128, name: &str) {
    let room = dest.len() - 1;
    let mut at = 0;
    for c in name.chars() {
        let mut units = [0; 2];
        let units = c.encode_utf16(&mut units);
        if at + units.len() > room {
            break;
        }
        for unit in units.iter() {
            dest[at] = *unit as TChar;
            at += 1;
        }
    }
    dest[at] = 0;
}

/// `Unknown`, `Interface` and `Inherits<FUnknown>` for an interface whose vtable begins with
/// `FUnknown`'s, as the generated bindings implement them.
macro_rules! interface {
    ($interface:ident, $vtbl:ident, $a:literal, $b:literal, $c:literal, $d:literal) => {
        // SAFETY: a pointer to the vtable, as COM lays out every interface.
        unsafe impl Send for $interface {}
        // SAFETY: as above.
        unsafe impl Sync for $interface {}

        impl Unknown for $interface {
            unsafe fn query_interface(this: *mut Self, iid: &Guid) -> Option<*mut c_void> {
                // SAFETY: its vtable begins with `FUnknown`'s.
                unsafe { FUnknown::query_interface(this.cast(), iid) }
            }

            unsafe fn add_ref(this: *mut Self) -> usize {
                // SAFETY: as above.
                unsafe { FUnknown::add_ref(this.cast()) }
            }

            unsafe fn release(this: *mut Self) -> usize {
                // SAFETY: as above.
                unsafe { FUnknown::release(this.cast()) }
            }
        }

        // SAFETY: the layout is a pointer to `$vtbl`, which begins with `FUnknownVtbl`.
        unsafe impl Interface for $interface {
            type Vtbl = $vtbl;
            const IID: Guid = guid(vst3::uid($a, $b, $c, $d));

            fn inherits(iid: &Guid) -> bool {
                iid == &Self::IID || FUnknown::inherits(iid)
            }
        }

        // SAFETY: `$vtbl` begins with `FUnknownVtbl`.
        unsafe impl Inherits<FUnknown> for $interface {}
    };
}

interface!(
    IOutputsInUse,
    IOutputsInUseVtbl,
    0x19786A5F,
    0xE141689D,
    0x5D908823,
    0xCE588BF6
);
interface!(
    IOutputsInUseHandler,
    IOutputsInUseHandlerVtbl,
    0x30EA7F50,
    0x4E3D0C7E,
    0x498B4086,
    0xC95A6C81
);

/// The object a method of an interface at `OFFSET` in `C`'s header was called on.
///
/// # Safety
///
/// `this` must be the interface pointer `W` made for a live object of `C`, at `OFFSET`.
unsafe fn object<'a, C: Class, W: Wrapper<C>, const OFFSET: isize, I>(this: *mut I) -> &'a C {
    // SAFETY: the caller's.
    unsafe {
        let header = (this as *mut u8).offset(-OFFSET) as *mut Header<C>;
        &*W::data_from_header(header)
    }
}

impl IOutputsInUse {
    const fn make_vtbl<C, W, const OFFSET: isize>() -> IOutputsInUseVtbl
    where
        C: IOutputsInUseTrait + Class,
        W: Wrapper<C>,
    {
        unsafe extern "system" fn getOutputCount<C, W, const OFFSET: isize>(
            this: *mut IOutputsInUse,
            count: *mut int32,
        ) -> tresult
        where
            C: IOutputsInUseTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: called through the vtable `W` made for this object.
            unsafe { object::<C, W, OFFSET, _>(this).getOutputCount(count) }
        }

        unsafe extern "system" fn getOutputInUse<C, W, const OFFSET: isize>(
            this: *mut IOutputsInUse,
            index: int32,
            bus_index: *mut int32,
            name: *mut String128,
        ) -> tresult
        where
            C: IOutputsInUseTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: as above.
            unsafe { object::<C, W, OFFSET, _>(this).getOutputInUse(index, bus_index, name) }
        }

        unsafe extern "system" fn getToMain<C, W, const OFFSET: isize>(
            this: *mut IOutputsInUse,
            bus_index: int32,
            index: int32,
            id: *mut ParamID,
            value: *mut ParamValue,
        ) -> tresult
        where
            C: IOutputsInUseTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: as above.
            unsafe { object::<C, W, OFFSET, _>(this).getToMain(bus_index, index, id, value) }
        }

        IOutputsInUseVtbl {
            base: FUnknown::make_vtbl::<C, W, OFFSET>(),
            getOutputCount: getOutputCount::<C, W, OFFSET>,
            getOutputInUse: getOutputInUse::<C, W, OFFSET>,
            getToMain: getToMain::<C, W, OFFSET>,
        }
    }
}

// SAFETY: `OBJ` points to a vtable of this interface's layout, made for `C`.
unsafe impl<C, W, const OFFSET: isize> Construct<C, W, OFFSET> for IOutputsInUse
where
    C: IOutputsInUseTrait + Class,
    W: Wrapper<C>,
{
    const OBJ: Self = IOutputsInUse {
        vtbl: &Self::make_vtbl::<C, W, OFFSET>(),
    };
}

impl IOutputsInUseHandler {
    const fn make_vtbl<C, W, const OFFSET: isize>() -> IOutputsInUseHandlerVtbl
    where
        C: IOutputsInUseHandlerTrait + Class,
        W: Wrapper<C>,
    {
        unsafe extern "system" fn outputsInUseChanged<C, W, const OFFSET: isize>(
            this: *mut IOutputsInUseHandler,
        ) -> tresult
        where
            C: IOutputsInUseHandlerTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: called through the vtable `W` made for this object.
            unsafe { object::<C, W, OFFSET, _>(this).outputsInUseChanged() }
        }

        IOutputsInUseHandlerVtbl {
            base: FUnknown::make_vtbl::<C, W, OFFSET>(),
            outputsInUseChanged: outputsInUseChanged::<C, W, OFFSET>,
        }
    }
}

// SAFETY: `OBJ` points to a vtable of this interface's layout, made for `C`.
unsafe impl<C, W, const OFFSET: isize> Construct<C, W, OFFSET> for IOutputsInUseHandler
where
    C: IOutputsInUseHandlerTrait + Class,
    W: Wrapper<C>,
{
    const OBJ: Self = IOutputsInUseHandler {
        vtbl: &Self::make_vtbl::<C, W, OFFSET>(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(name: &String128) -> String {
        let end = name.iter().position(|&unit| unit == 0).unwrap();
        String::from_utf16(&name[..end]).unwrap()
    }

    #[test]
    fn a_name_is_cut_where_it_no_longer_fits_never_inside_a_surrogate_pair() {
        let mut name = [0; 128];
        copy_name(&mut name, "Kick");
        assert_eq!(text(&name), "Kick");
        // 126 units of "a", then "𝄞" (two units) has no room: 127 units hold text, the last the NUL.
        let long = format!("{}𝄞", "a".repeat(126));
        copy_name(&mut name, &long);
        assert_eq!(text(&name), "a".repeat(126));
        let fits = format!("{}𝄞", "a".repeat(125));
        copy_name(&mut name, &fits);
        assert_eq!(text(&name), fits);
    }

    #[test]
    fn the_ids_are_the_sdks_for_the_same_four_numbers() {
        // `DECLARE_CLASS_IID`'s bytes on this platform: COM's order on Windows, big-endian
        // elsewhere, as `vst3::uid` lays them out.
        let expected: [u8; 16] = if cfg!(windows) {
            [
                0x5F, 0x6A, 0x78, 0x19, 0x41, 0xE1, 0x9D, 0x68, 0x5D, 0x90, 0x88, 0x23, 0xCE, 0x58,
                0x8B, 0xF6,
            ]
        } else {
            [
                0x19, 0x78, 0x6A, 0x5F, 0xE1, 0x41, 0x68, 0x9D, 0x5D, 0x90, 0x88, 0x23, 0xCE, 0x58,
                0x8B, 0xF6,
            ]
        };
        assert_eq!(IOutputsInUse::IID, expected);
        assert_ne!(IOutputsInUse::IID, IOutputsInUseHandler::IID);
    }
}
