//! MXM PATCH (outputs in use): a capability, not a fix. See `PATCHES.md`, *Outputs in use*.
//!
//! The CLAP side of [`OutputsInUse`][crate::wrapper::outputs_in_use::OutputsInUse]: an MXM CLAP
//! extension, `dk.mxm.outputs-in-use/1`, through which a host learns the output ports after the
//! main one that something in the plugin sends to now, each with a name for what plays there. CLAP
//! has nothing like it: a host can tell a plugin which ports it listens to
//! (`audio-ports-activation`), never the other way round. The convention is mxm-kit's
//! `docs/plugin-conventions.md`, *Outputs in use*; the state behind it is
//! `wrapper/outputs_in_use.rs`'s, shared with the VST3 wrapper.

use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use nice_plug_core::audio_setup::AudioIOLayout;
use std::ffi::{CStr, c_char};

/// The extension's id, for the plugin's `get_extension` and the host's.
pub const DK_MXM_EXT_OUTPUTS_IN_USE: &CStr = c"dk.mxm.outputs-in-use/1";

/// The plugin's side. Both calls are main-thread calls.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct dk_mxm_plugin_outputs_in_use {
    /// How many output ports after the main one something in the plugin sends to now, whatever
    /// configuration is active.
    pub count: Option<unsafe extern "C" fn(plugin: *const clap_plugin) -> u32>,
    /// One of them: its port id, as the configuration that has the port gives it, and a name for
    /// what plays there, NUL-terminated UTF-8 in `name`, which holds `CLAP_NAME_SIZE` bytes. False
    /// for an index past the count.
    pub get: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            index: u32,
            port_id: *mut clap_id,
            name: *mut c_char,
        ) -> bool,
    >,
    /// What sends whatever plays on port `port_id` back to the main output: one parameter value,
    /// its id and value as `params` gives them, for the host to set as it sets any. `index`
    /// counts from 0; false past the last, or for a port with nothing routed to it.
    pub to_main: Option<
        unsafe extern "C" fn(
            plugin: *const clap_plugin,
            port_id: clap_id,
            index: u32,
            param_id: *mut clap_id,
            value: *mut f64,
        ) -> bool,
    >,
}

/// The host's side.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct dk_mxm_host_outputs_in_use {
    /// [main-thread] The outputs in use, or their names, may have changed: the host reads them
    /// again.
    pub changed: Option<unsafe extern "C" fn(host: *const clap_host)>,
}

/// The CLAP port id of output `output` (its place among the output ports, the main one first):
/// as the wrapper numbers the ports of the first layout that has it, the inputs first and the
/// outputs after them (`ext_audio_ports_get`). `None` when no layout has it.
pub(crate) fn port_id(layouts: &[AudioIOLayout], output: u32) -> Option<u32> {
    layouts
        .iter()
        .find(|layout| {
            u32::from(layout.main_output_channels.is_some()) + layout.aux_output_ports.len() as u32
                > output
        })
        .map(|layout| {
            u32::from(layout.main_input_channels.is_some())
                + layout.aux_input_ports.len() as u32
                + output
        })
}

/// The output (its place among the output ports, the main one first) that CLAP port `port` is:
/// [`port_id`]'s inverse.
pub(crate) fn output_of_port(layouts: &[AudioIOLayout], port: u32) -> Option<u32> {
    let most = layouts
        .iter()
        .map(|layout| {
            u32::from(layout.main_output_channels.is_some()) + layout.aux_output_ports.len() as u32
        })
        .max()?;
    (0..most).find(|&output| port_id(layouts, output) == Some(port))
}

/// Copies `name` into `dest` as NUL-terminated UTF-8, cut at a character boundary when it
/// doesn't fit.
pub(crate) fn copy_name(dest: &mut [c_char], name: &str) {
    let Some(room) = dest.len().checked_sub(1) else {
        return;
    };
    let mut end = name.len().min(room);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    crate::wrapper::util::strlcpy(dest, &name[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug_core::audio_setup::new_nonzero_u32;

    const STEREO: std::num::NonZeroU32 = new_nonzero_u32(2);

    #[test]
    fn a_port_id_counts_the_inputs_of_the_first_layout_that_has_the_output() {
        let drums = [
            AudioIOLayout {
                main_output_channels: Some(STEREO),
                aux_output_ports: &[STEREO, STEREO],
                ..AudioIOLayout::const_default()
            },
            AudioIOLayout {
                main_output_channels: Some(STEREO),
                ..AudioIOLayout::const_default()
            },
        ];
        assert_eq!(port_id(&drums, 0), Some(0));
        assert_eq!(port_id(&drums, 2), Some(2));
        assert_eq!(port_id(&drums, 3), None, "no layout has it");
        let with_inputs = [AudioIOLayout {
            main_input_channels: Some(STEREO),
            aux_input_ports: &[STEREO],
            main_output_channels: Some(STEREO),
            aux_output_ports: &[STEREO],
            ..AudioIOLayout::const_default()
        }];
        assert_eq!(port_id(&with_inputs, 1), Some(3), "after two inputs");
        assert_eq!(port_id(&[], 1), None);
        for output in 0..3 {
            assert_eq!(output_of_port(&drums, output), Some(output));
        }
        assert_eq!(output_of_port(&drums, 3), None);
        assert_eq!(output_of_port(&with_inputs, 3), Some(1));
        assert_eq!(output_of_port(&with_inputs, 1), None, "an input");
    }

    #[test]
    fn a_name_is_cut_at_a_character_boundary() {
        // "Kick" and a two-byte "é" fill six bytes, the NUL a seventh.
        let mut dest = [0 as c_char; 7];
        copy_name(&mut dest, "Kickéé");
        let text = unsafe { CStr::from_ptr(dest.as_ptr()) };
        assert_eq!(text.to_str().unwrap(), "Kické");
        let mut dest = [0 as c_char; 6];
        copy_name(&mut dest, "Kické");
        let text = unsafe { CStr::from_ptr(dest.as_ptr()) };
        assert_eq!(text.to_str().unwrap(), "Kick", "half an é is never written");
        copy_name(&mut [], "Kick");
    }
}
