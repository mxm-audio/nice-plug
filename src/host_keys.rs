//! MXM PATCH (*Host keys*): the keys a host hands an editor instead of letting its window have them.
//!
//! Some VST3 hosts keep keys from a plug-in's window and offer them through `IPlugView::onKeyDown`
//! and `onKeyUp` instead, using whatever the plug-in declines. Ableton Live 11 on Windows does so
//! for letters and for Shift with the arrows, as `VKEY_FIRST_ASCII + (character - '0')` for a
//! letter, and sends no key releases (measured 2026-10-16). `nice-plug-core`'s `EditorHandle` takes
//! only non-character keys (`on_virtual_key_from_host`), so an editor that wants every key the
//! host offers registers a [`Receiver`] here under the window it opened in, and the VST3 wrapper
//! hands each key to the one registered for the window it attached the editor to. The MXM fork of
//! nice-plug-egui registers one when its settings ask for it.

use crate::editor::{Modifiers, ParentWindowHandle, VirtualKeyCode};
use std::sync::{Arc, Mutex};

/// A key a host hands over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKey {
    /// A key with no character: an arrow, Escape, a function key.
    Virtual(VirtualKeyCode),
    /// A character key, named by its uppercase ASCII character: `S` for the S key, `1` for the 1
    /// key, `;` for the semicolon.
    Character(char),
}

/// What an editor does with a key a host offers it: `true` if it took the key, so the host doesn't
/// use it too. Called on the host's GUI thread with the key, whether it went down, and the
/// modifiers held.
pub type Receiver = Arc<dyn Fn(HostKey, bool, Modifiers) -> bool + Send + Sync>;

/// A [`Receiver`] registered for a window, until this is dropped.
pub struct Registration {
    window: usize,
    receiver: Receiver,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut registered = registry().lock().unwrap_or_else(|e| e.into_inner());
        registered.retain(|(window, receiver)| {
            *window != self.window || !Arc::ptr_eq(receiver, &self.receiver)
        });
    }
}

type Registered = Vec<(usize, Receiver)>;

fn registry() -> &'static Mutex<Registered> {
    static REGISTRY: Mutex<Registered> = Mutex::new(Vec::new());
    &REGISTRY
}

/// The number a window is registered under: its native handle.
pub fn window_key(window: &ParentWindowHandle) -> usize {
    match window {
        ParentWindowHandle::XlibWindow(window) => *window as usize,
        ParentWindowHandle::XcbWindow(window) => window.get() as usize,
        ParentWindowHandle::AppKitNsView(view) => view.as_ptr() as usize,
        ParentWindowHandle::Win32Hwnd(hwnd) => hwnd.get() as usize,
    }
}

/// Registers `receiver` for the editor opened in `window`, until the registration is dropped. A
/// later registration for the same window is the one keys go to.
pub fn register(window: &ParentWindowHandle, receiver: Receiver) -> Registration {
    let window = window_key(window);
    let mut registered = registry().lock().unwrap_or_else(|e| e.into_inner());
    registered.push((window, receiver.clone()));
    Registration { window, receiver }
}

/// Offers `key` to the receiver registered for `window` (a [`window_key`]): whether it took the
/// key, or `None` when nothing is registered there.
pub fn offer(window: usize, key: HostKey, down: bool, modifiers: Modifiers) -> Option<bool> {
    // Called outside the lock: a receiver may take its time, or register another.
    let receiver = {
        let registered = registry().lock().unwrap_or_else(|e| e.into_inner());
        registered
            .iter()
            .rev()
            .find(|(registered, _)| *registered == window)
            .map(|(_, receiver)| receiver.clone())
    }?;
    Some(receiver(key, down, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroIsize;

    fn window(n: isize) -> ParentWindowHandle {
        ParentWindowHandle::Win32Hwnd(NonZeroIsize::new(n).unwrap())
    }

    #[test]
    fn a_key_goes_to_the_receiver_of_its_window_while_it_is_registered() {
        let wants_s: Receiver = Arc::new(|key, _, _| key == HostKey::Character('S'));
        let registration = register(&window(0x7a01), wants_s);
        let key = window_key(&window(0x7a01));
        assert_eq!(
            offer(key, HostKey::Character('S'), true, Modifiers::empty()),
            Some(true)
        );
        assert_eq!(
            offer(key, HostKey::Character('T'), true, Modifiers::empty()),
            Some(false)
        );
        assert_eq!(
            offer(
                window_key(&window(0x7a02)),
                HostKey::Character('S'),
                true,
                Modifiers::empty()
            ),
            None
        );
        drop(registration);
        assert_eq!(
            offer(key, HostKey::Character('S'), true, Modifiers::empty()),
            None
        );
    }

    #[test]
    fn the_latest_registration_for_a_window_takes_its_keys_and_dropping_it_restores_the_first() {
        let first = register(&window(0x7b01), Arc::new(|_, _, _| false));
        let second = register(&window(0x7b01), Arc::new(|_, _, _| true));
        let key = window_key(&window(0x7b01));
        assert_eq!(
            offer(key, HostKey::Character('A'), true, Modifiers::empty()),
            Some(true)
        );
        drop(second);
        assert_eq!(
            offer(key, HostKey::Character('A'), true, Modifiers::empty()),
            Some(false)
        );
        drop(first);
    }
}
