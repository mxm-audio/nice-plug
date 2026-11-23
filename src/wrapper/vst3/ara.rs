//! MXM PATCH (ARA): a capability, not a fix. See `PATCHES.md`, *ARA*.
//!
//! ARA's VST3 companion API (`ARAVST3.h`, ARA SDK 2.3), written as the `vst3` crate writes its own
//! interfaces (as `outputs_in_use` does):
//!
//! ```cpp
//! namespace ARA {
//! // The class of category kARAMainFactoryClass ("ARA Main Factory Class").
//! class IMainFactory: public Steinberg::FUnknown {
//!     virtual const ARAFactory* PLUGIN_API getFactory () = 0;
//! };
//! DECLARE_CLASS_IID (IMainFactory, 0xDB2A1669, 0xFAFD42A5, 0xA82F864F, 0x7B6872EA)
//! // On the plugin's IAudioProcessor.
//! class IPlugInEntryPoint: public Steinberg::FUnknown {
//!     virtual const ARAFactory* PLUGIN_API getFactory () = 0;
//!     // Deprecated: every role.
//!     virtual const ARAPlugInExtensionInstance* PLUGIN_API bindToDocumentController (ARADocumentControllerRef) = 0;
//! };
//! DECLARE_CLASS_IID (IPlugInEntryPoint, 0x12814E54, 0xA1CE4076, 0x82B96813, 0x16950BD6)
//! class IPlugInEntryPoint2: public Steinberg::FUnknown {
//!     virtual const ARAPlugInExtensionInstance* PLUGIN_API bindToDocumentControllerWithRoles (
//!         ARADocumentControllerRef, ARAPlugInInstanceRoleFlags knownRoles,
//!         ARAPlugInInstanceRoleFlags assignedRoles) = 0;
//! };
//! DECLARE_CLASS_IID (IPlugInEntryPoint2, 0xCD9A5913, 0xC9EB46D7, 0x96CA53AD, 0xD1DB89F5)
//! }
//! ```
//!
//! **A plugin that doesn't offer ARA answers neither entry point** ([`AraFiltered`]) and its
//! factory lists no main factory class: it is as it was.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::marker::PhantomData;

use vst3::Interface;
use vst3::Steinberg::{FUnknown, FUnknownVtbl};
use vst3::com_scrape_types::{
    Class, Construct, Guid, Inherits, InterfaceList, MakeHeader, Unknown, Wrapper,
};

use super::Vst3Plugin;
use super::outputs_in_use::{guid, interface, object};

/// The category of an ARA main factory class (`kARAMainFactoryClass`).
pub const ARA_MAIN_FACTORY_CLASS: &str = "ARA Main Factory Class";

/// Every role (`kARAPlaybackRendererRole | kARAEditorRendererRole | kARAEditorViewRole`): what the
/// deprecated `bindToDocumentController` binds, as ARA's own library reads it.
pub const ALL_ROLES: u32 = 1 | 2 | 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IMainFactory {
    pub vtbl: *const IMainFactoryVtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IMainFactoryVtbl {
    pub base: FUnknownVtbl,
    pub getFactory: unsafe extern "system" fn(this: *mut IMainFactory) -> *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPlugInEntryPoint {
    pub vtbl: *const IPlugInEntryPointVtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPlugInEntryPointVtbl {
    pub base: FUnknownVtbl,
    pub getFactory: unsafe extern "system" fn(this: *mut IPlugInEntryPoint) -> *const c_void,
    pub bindToDocumentController: unsafe extern "system" fn(
        this: *mut IPlugInEntryPoint,
        document_controller: *const c_void,
    ) -> *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPlugInEntryPoint2 {
    pub vtbl: *const IPlugInEntryPoint2Vtbl,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPlugInEntryPoint2Vtbl {
    pub base: FUnknownVtbl,
    pub bindToDocumentControllerWithRoles: unsafe extern "system" fn(
        this: *mut IPlugInEntryPoint2,
        document_controller: *const c_void,
        known_roles: u32,
        assigned_roles: u32,
    ) -> *const c_void,
}

interface!(
    IMainFactory,
    IMainFactoryVtbl,
    0xDB2A1669,
    0xFAFD42A5,
    0xA82F864F,
    0x7B6872EA
);
interface!(
    IPlugInEntryPoint,
    IPlugInEntryPointVtbl,
    0x12814E54,
    0xA1CE4076,
    0x82B96813,
    0x16950BD6
);
interface!(
    IPlugInEntryPoint2,
    IPlugInEntryPoint2Vtbl,
    0xCD9A5913,
    0xC9EB46D7,
    0x96CA53AD,
    0xD1DB89F5
);

/// What a class implementing [`IMainFactory`] does.
#[allow(clippy::missing_safety_doc)]
pub trait IMainFactoryTrait {
    unsafe fn getFactory(&self) -> *const c_void;
}

/// What a class implementing [`IPlugInEntryPoint`] does.
#[allow(clippy::missing_safety_doc)]
pub trait IPlugInEntryPointTrait {
    unsafe fn getFactory(&self) -> *const c_void;
    unsafe fn bindToDocumentController(&self, document_controller: *const c_void) -> *const c_void;
}

/// What a class implementing [`IPlugInEntryPoint2`] does.
#[allow(clippy::missing_safety_doc)]
pub trait IPlugInEntryPoint2Trait {
    unsafe fn bindToDocumentControllerWithRoles(
        &self,
        document_controller: *const c_void,
        known_roles: u32,
        assigned_roles: u32,
    ) -> *const c_void;
}

impl IMainFactory {
    const fn make_vtbl<C, W, const OFFSET: isize>() -> IMainFactoryVtbl
    where
        C: IMainFactoryTrait + Class,
        W: Wrapper<C>,
    {
        unsafe extern "system" fn getFactory<C, W, const OFFSET: isize>(
            this: *mut IMainFactory,
        ) -> *const c_void
        where
            C: IMainFactoryTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: called through the vtable `W` made for this object.
            unsafe { object::<C, W, OFFSET, _>(this).getFactory() }
        }

        IMainFactoryVtbl {
            base: FUnknown::make_vtbl::<C, W, OFFSET>(),
            getFactory: getFactory::<C, W, OFFSET>,
        }
    }
}

// SAFETY: `OBJ` points to a vtable of this interface's layout, made for `C`.
unsafe impl<C, W, const OFFSET: isize> Construct<C, W, OFFSET> for IMainFactory
where
    C: IMainFactoryTrait + Class,
    W: Wrapper<C>,
{
    const OBJ: Self = IMainFactory {
        vtbl: &Self::make_vtbl::<C, W, OFFSET>(),
    };
}

impl IPlugInEntryPoint {
    const fn make_vtbl<C, W, const OFFSET: isize>() -> IPlugInEntryPointVtbl
    where
        C: IPlugInEntryPointTrait + Class,
        W: Wrapper<C>,
    {
        unsafe extern "system" fn getFactory<C, W, const OFFSET: isize>(
            this: *mut IPlugInEntryPoint,
        ) -> *const c_void
        where
            C: IPlugInEntryPointTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: called through the vtable `W` made for this object.
            unsafe { object::<C, W, OFFSET, _>(this).getFactory() }
        }

        unsafe extern "system" fn bindToDocumentController<C, W, const OFFSET: isize>(
            this: *mut IPlugInEntryPoint,
            document_controller: *const c_void,
        ) -> *const c_void
        where
            C: IPlugInEntryPointTrait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: as above.
            unsafe { object::<C, W, OFFSET, _>(this).bindToDocumentController(document_controller) }
        }

        IPlugInEntryPointVtbl {
            base: FUnknown::make_vtbl::<C, W, OFFSET>(),
            getFactory: getFactory::<C, W, OFFSET>,
            bindToDocumentController: bindToDocumentController::<C, W, OFFSET>,
        }
    }
}

// SAFETY: `OBJ` points to a vtable of this interface's layout, made for `C`.
unsafe impl<C, W, const OFFSET: isize> Construct<C, W, OFFSET> for IPlugInEntryPoint
where
    C: IPlugInEntryPointTrait + Class,
    W: Wrapper<C>,
{
    const OBJ: Self = IPlugInEntryPoint {
        vtbl: &Self::make_vtbl::<C, W, OFFSET>(),
    };
}

impl IPlugInEntryPoint2 {
    const fn make_vtbl<C, W, const OFFSET: isize>() -> IPlugInEntryPoint2Vtbl
    where
        C: IPlugInEntryPoint2Trait + Class,
        W: Wrapper<C>,
    {
        unsafe extern "system" fn bindToDocumentControllerWithRoles<C, W, const OFFSET: isize>(
            this: *mut IPlugInEntryPoint2,
            document_controller: *const c_void,
            known_roles: u32,
            assigned_roles: u32,
        ) -> *const c_void
        where
            C: IPlugInEntryPoint2Trait + Class,
            W: Wrapper<C>,
        {
            // SAFETY: called through the vtable `W` made for this object.
            unsafe {
                object::<C, W, OFFSET, _>(this).bindToDocumentControllerWithRoles(
                    document_controller,
                    known_roles,
                    assigned_roles,
                )
            }
        }

        IPlugInEntryPoint2Vtbl {
            base: FUnknown::make_vtbl::<C, W, OFFSET>(),
            bindToDocumentControllerWithRoles: bindToDocumentControllerWithRoles::<C, W, OFFSET>,
        }
    }
}

// SAFETY: `OBJ` points to a vtable of this interface's layout, made for `C`.
unsafe impl<C, W, const OFFSET: isize> Construct<C, W, OFFSET> for IPlugInEntryPoint2
where
    C: IPlugInEntryPoint2Trait + Class,
    W: Wrapper<C>,
{
    const OBJ: Self = IPlugInEntryPoint2 {
        vtbl: &Self::make_vtbl::<C, W, OFFSET>(),
    };
}

/// The object a host makes from an ARA main factory class: it hands over the plugin's
/// `ARAFactory`.
pub struct AraMainFactory {
    pub factory: fn() -> *const c_void,
}

impl Class for AraMainFactory {
    type Interfaces = (IMainFactory,);
}

impl IMainFactoryTrait for AraMainFactory {
    unsafe fn getFactory(&self) -> *const c_void {
        (self.factory)()
    }
}

/// The interfaces `L` a component answers, but ARA's two entry points only for a plugin `P` that
/// offers ARA: a host asks every VST3 for them to learn which offer ARA.
pub struct AraFiltered<P, L>(PhantomData<(P, L)>);

// SAFETY: `L`'s list, with two of its interfaces left out of `query`: every offset `query` gives
// is one `L` gives for the same header.
unsafe impl<P: Vst3Plugin, L: InterfaceList> InterfaceList for AraFiltered<P, L> {
    type Header = L::Header;

    fn query(iid: &Guid) -> Option<isize> {
        if P::VST3_ARA.is_none()
            && (iid == &IPlugInEntryPoint::IID || iid == &IPlugInEntryPoint2::IID)
        {
            return None;
        }
        L::query(iid)
    }
}

// SAFETY: `L`'s header, as `L` makes it.
unsafe impl<P, L, C, W> MakeHeader<C, W> for AraFiltered<P, L>
where
    P: Vst3Plugin,
    L: MakeHeader<C, W>,
    C: Class,
    W: Wrapper<C>,
{
    const HEADER: Self::Header = L::HEADER;
}
