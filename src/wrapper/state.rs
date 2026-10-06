use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use anyhow::{Context, Result};
use nice_plug_core::{
    audio_setup::BufferConfig,
    params::{InternalParamMut, Param, Params, internals::ParamPtr},
    plugin::{ParamValue, Plugin, PluginState},
};

// MXM PATCH (defect 11): defect 9 must distinguish a field that rejected its input from one that
// accepted it and immediately installed a canonical representation. `PersistentField::set()` has no
// return value and nice-plug-core is a separate published crate, so the wrapper opens this scoped,
// same-thread acknowledgement channel only around `Params::deserialize_fields()`. A field may mark
// its stable persisted key only after successful canonical publication; marks outside a restore are
// discarded and cannot weaken a later transaction.
thread_local! {
    static CANONICALIZED_FIELDS: RefCell<Vec<BTreeSet<&'static str>>> =
        const { RefCell::new(Vec::new()) };
}

/// Acknowledge that the current persistent-field restore accepted `field_key` and installed a
/// canonical value. Calling this outside `Params::deserialize_fields()` has no effect.
pub fn accept_canonicalized_persistent_field(field_key: &'static str) {
    CANONICALIZED_FIELDS.with(|scopes| {
        if let Some(fields) = scopes.borrow_mut().last_mut() {
            fields.insert(field_key);
        }
    });
}

struct CanonicalizedFieldScope {
    active: bool,
}

impl CanonicalizedFieldScope {
    fn begin() -> Self {
        CANONICALIZED_FIELDS.with(|scopes| scopes.borrow_mut().push(BTreeSet::new()));
        Self { active: true }
    }

    fn finish(mut self) -> BTreeSet<&'static str> {
        self.active = false;
        CANONICALIZED_FIELDS.with(|scopes| {
            scopes
                .borrow_mut()
                .pop()
                .expect("canonicalized-field restore scope")
        })
    }
}

impl Drop for CanonicalizedFieldScope {
    fn drop(&mut self) {
        if self.active {
            CANONICALIZED_FIELDS.with(|scopes| {
                scopes.borrow_mut().pop();
            });
        }
    }
}

/// Create a parameters iterator from the hashtables stored in the plugin wrappers. This avoids
/// having to call `.param_map()` again, which may include expensive user written code.
pub(crate) fn make_params_iter<'a>(
    param_by_hash: &'a HashMap<u32, ParamPtr>,
    param_id_to_hash: &'a HashMap<String, u32>,
) -> impl IntoIterator<Item = (&'a String, ParamPtr)> {
    param_id_to_hash.iter().filter_map(|(param_id_str, hash)| {
        let param_ptr = param_by_hash.get(hash)?;
        Some((param_id_str, *param_ptr))
    })
}

/// Create a getter function that gets a parameter from the hashtables stored in the plugin by
/// string ID.
pub(crate) fn make_params_getter<'a>(
    param_by_hash: &'a HashMap<u32, ParamPtr>,
    param_id_to_hash: &'a HashMap<String, u32>,
) -> impl Fn(&str) -> Option<ParamPtr> + 'a {
    |param_id_str| {
        param_id_to_hash
            .get(param_id_str)
            .and_then(|hash| param_by_hash.get(hash))
            .copied()
    }
}

/// Serialize a plugin's state to a state object. This is separate from [`serialize_json()`] to
/// allow passing the raw object directly to the plugin. The parameters are not pulled directly from
/// `plugin_params` by default to avoid unnecessary allocations in the `.param_map()` method, as the
/// plugin wrappers will already have a list of parameters handy. See [`make_params_iter()`].
pub(crate) unsafe fn serialize_object<'a, P: Plugin>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> PluginState {
    // We'll serialize parameter values as a simple `string_param_id: display_value` map.
    // NOTE: If the plugin is being modulated (and the plugin is a CLAP plugin in Bitwig Studio),
    //       then this should save the values without any modulation applied to it
    let params: BTreeMap<_, _> = unsafe {
        params_iter
            .into_iter()
            .map(|(param_id_str, param_ptr)| match param_ptr {
                ParamPtr::FloatParam(p) => (
                    param_id_str.clone(),
                    ParamValue::F32((*p).unmodulated_plain_value()),
                ),
                ParamPtr::IntParam(p) => (
                    param_id_str.clone(),
                    ParamValue::I32((*p).unmodulated_plain_value()),
                ),
                ParamPtr::BoolParam(p) => (
                    param_id_str.clone(),
                    ParamValue::Bool((*p).unmodulated_plain_value()),
                ),
                ParamPtr::EnumParam(p) => (
                    // Enums are either serialized based on the active variant's index (which may not be
                    // the same as the discriminator), or a custom set stable string ID. The latter
                    // allows the variants to be reordered.
                    param_id_str.clone(),
                    match (*p).unmodulated_plain_id() {
                        Some(id) => ParamValue::String(id.to_owned()),
                        None => ParamValue::I32((*p).unmodulated_plain_value()),
                    },
                ),
            })
            .collect()
    };

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    let fields = plugin_params.serialize_fields();

    PluginState {
        version: String::from(P::VERSION),
        params,
        fields,
    }
}

/// Serialize a plugin's state to a vector containing JSON data. This can (and should) be shared
/// across plugin formats. If the `zstd` feature is enabled, then the state will be compressed using
/// Zstandard.
pub(crate) unsafe fn serialize_json<'a, P: Plugin>(
    plugin_params: Arc<dyn Params>,
    params_iter: impl IntoIterator<Item = (&'a String, ParamPtr)>,
) -> Result<Vec<u8>> {
    let plugin_state = unsafe { serialize_object::<P>(plugin_params, params_iter) };
    let json = serde_json::to_vec(&plugin_state).context("Could not format as JSON")?;

    #[cfg(feature = "zstd")]
    {
        let compressed = zstd::encode_all(json.as_slice(), zstd::DEFAULT_COMPRESSION_LEVEL)
            .context("Could not compress state")?;

        let state_bytes = json.len();
        let compressed_state_bytes = compressed.len();
        let compression_ratio = compressed_state_bytes as f32 / state_bytes as f32 * 100.0;
        nice_plug_core::nice_trace!(
            "Compressed {state_bytes} bytes of state to {compressed_state_bytes} bytes \
             ({compression_ratio:.1}% compression ratio)"
        );

        Ok(compressed)
    }
    #[cfg(not(feature = "zstd"))]
    {
        Ok(json)
    }
}

/// Deserialize a plugin's state from a [`PluginState`] object. This is used to allow the plugin to
/// do its own internal preset management. Returns `false` and logs an error if the state could not
/// be deserialized.
///
/// This uses a parameter getter function to avoid having to rebuild the parameter map, which may
/// include expensive user written code. See [`make_params_getter()`].
///
/// Make sure to reactivate plugin after deserializing the state so it can react to the new
/// parameter values. The smoothers have already been reset by this function.
///
/// The [`Plugin`] argument is used to call [`Plugin::filter_state()`] just before loading the
/// state.
pub(crate) unsafe fn deserialize_object<P: Plugin>(
    state: &mut PluginState,
    plugin_params: Arc<dyn Params>,
    params_getter: impl Fn(&str) -> Option<ParamPtr>,
    current_buffer_config: Option<&BufferConfig>,
) -> bool {
    // This lets the plugin perform migrations on old state if needed
    P::filter_state(state);

    let sample_rate = current_buffer_config.map(|c| c.sample_rate);
    for (param_id_str, param_value) in &state.params {
        let param_ptr = match params_getter(param_id_str.as_str()) {
            Some(ptr) => ptr,
            None => {
                #[cfg(debug_assertions)]
                crate::nice_warn!("Unknown parameter: {}", param_id_str);
                continue;
            }
        };

        unsafe {
            match (param_ptr, param_value) {
                (ParamPtr::FloatParam(p), ParamValue::F32(v)) => {
                    (*p)._internal_set_plain_value(*v);
                }
                (ParamPtr::IntParam(p), ParamValue::I32(v)) => {
                    (*p)._internal_set_plain_value(*v);
                }
                (ParamPtr::BoolParam(p), ParamValue::Bool(v)) => {
                    (*p)._internal_set_plain_value(*v);
                }
                // Enums are either serialized based on the active variant's index (which may not be the
                // same as the discriminator), or a custom set stable string ID. The latter allows the
                // variants to be reordered.
                (ParamPtr::EnumParam(p), ParamValue::I32(variant_idx)) => {
                    (*p)._internal_set_plain_value(*variant_idx);
                }
                (ParamPtr::EnumParam(p), ParamValue::String(id)) => {
                    let deserialized_enum = (*p).set_from_id(id);
                    if !deserialized_enum {
                        #[cfg(debug_assertions)]
                        crate::nice_warn!(
                            "Unknown ID {:?} for enum parameter \"{}\"",
                            id,
                            param_id_str,
                        );
                    }
                }
                (param_ptr, param_value) => {
                    #[cfg(debug_assertions)]
                    crate::nice_warn!(
                        "Invalid serialized value {:?} for parameter \"{}\" ({:?})",
                        param_value,
                        param_id_str,
                        param_ptr,
                    );
                }
            }
        }

        // Make sure everything starts out in sync
        if let Some(sample_rate) = sample_rate {
            unsafe { param_ptr._internal_update_smoother(sample_rate, true) };
        }
    }

    // The plugin can also persist arbitrary fields alongside its parameters. This is useful for
    // storing things like sample data.
    let canonicalized_scope = CanonicalizedFieldScope::begin();
    plugin_params.deserialize_fields(&state.fields);
    let canonicalized_fields = canonicalized_scope.finish();

    // MXM PATCH (defects 9 and 11): `PersistentField::set()` is intentionally infallible and may
    // reject malformed or unpreparable durable content by preserving its previous value. Detect
    // that rejection generically by requiring every supplied field to serialize back identically.
    // A field that deliberately installed a different canonical value is accepted only when that
    // exact stable key explicitly acknowledged successful canonicalization in the scoped call above.
    // This keeps a genuine mismatch on defect 8's complete rollback path without treating every
    // difference as acceptance.
    let restored_fields = plugin_params.serialize_fields();
    state.fields.iter().all(|(key, requested)| {
        restored_fields.get(key).is_some_and(|restored| {
            persisted_field_accepted(key, requested, restored, &canonicalized_fields)
        })
    })
}

fn persisted_field_accepted(
    key: &str,
    requested: &str,
    restored: &str,
    canonicalized_fields: &BTreeSet<&'static str>,
) -> bool {
    persisted_field_matches(requested, restored) || canonicalized_fields.contains(key)
}

fn persisted_field_matches(requested: &str, restored: &str) -> bool {
    requested == restored
        || serde_json::from_str::<serde_json::Value>(requested)
            .ok()
            .zip(serde_json::from_str::<serde_json::Value>(restored).ok())
            .is_some_and(|(requested, restored)| requested == restored)
}

/// Deserialize a plugin's state from a vector containing (compressed) JSON data. Doesn't load the
/// plugin state since doing so should be accompanied by calls to `Plugin::init()` and
/// `Plugin::reset()`, and this way all of that behavior can be encapsulated so it can be reused in
/// multiple places. The returned state object can be passed to [`deserialize_object()`].
pub(crate) unsafe fn deserialize_json(state: &[u8]) -> Option<PluginState> {
    #[cfg(feature = "zstd")]
    let result: Option<PluginState> = match zstd::decode_all(state) {
        Ok(decompressed) => match serde_json::from_slice(decompressed.as_slice()) {
            Ok(s) => {
                let state_bytes = decompressed.len();
                let compressed_state_bytes = state.len();
                let compression_ratio = compressed_state_bytes as f32 / state_bytes as f32 * 100.0;
                crate::nice_trace!(
                    "Inflated {compressed_state_bytes} bytes of state to {state_bytes} bytes \
                     ({compression_ratio:.1}% compression ratio)"
                );

                Some(s)
            }
            Err(err) => {
                crate::nice_debug_assert_failure!("Error while deserializing state: {}", err);
                None
            }
        },
        // Uncompressed state files can still be loaded after enabling this feature to prevent
        // breaking existing plugin instances
        Err(zstd_err) => match serde_json::from_slice(state) {
            Ok(s) => {
                crate::nice_trace!("Older uncompressed state found");
                Some(s)
            }
            Err(json_err) => {
                crate::nice_debug_assert_failure!(
                    "Error while deserializing state as either compressed or uncompressed state: \
                     {}, {}",
                    zstd_err,
                    json_err
                );
                None
            }
        },
    };

    #[cfg(not(feature = "zstd"))]
    let result: Option<PluginState> = match serde_json::from_slice(state) {
        Ok(s) => Some(s),
        Err(err) => {
            crate::nice_debug_assert_failure!("Error while deserializing state: {}", err);
            None
        }
    };

    result
}

// MXM PATCH (defect 11): kept after every item, as `clippy::items_after_test_module` requires.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalized_field_acceptance_is_scoped_and_key_specific() {
        accept_canonicalized_persistent_field("response");

        let scope = CanonicalizedFieldScope::begin();
        accept_canonicalized_persistent_field("response");
        let accepted = scope.finish();
        assert!(accepted.contains("response"));
        assert!(!accepted.contains("preset"));
        assert!(persisted_field_accepted(
            "response",
            r#"{"value": 200}"#,
            r#"{"value": 100}"#,
            &accepted,
        ));
        assert!(
            !persisted_field_accepted(
                "preset",
                r#"{"name": "requested"}"#,
                r#"{"name": "previous"}"#,
                &accepted,
            ),
            "acknowledging response must not admit a rejected preset"
        );

        let next_scope = CanonicalizedFieldScope::begin();
        assert!(next_scope.finish().is_empty(), "an old mark leaked");
    }

    #[test]
    fn canonicalized_field_scope_clears_on_unwind() {
        let _ = std::panic::catch_unwind(|| {
            let _scope = CanonicalizedFieldScope::begin();
            accept_canonicalized_persistent_field("response");
            panic!("exercise scope drop");
        });

        let next_scope = CanonicalizedFieldScope::begin();
        assert!(next_scope.finish().is_empty(), "a panic leaked acceptance");
    }
}
