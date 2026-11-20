//! MXM PATCH (outputs in use): a capability, not a fix. See `PATCHES.md`, *Outputs in use*.
//!
//! Which outputs after the main one something in a plugin sends to now, each with a name for what
//! plays there, so a host can give each its own track as the user routes to it. Neither CLAP nor
//! VST3 has anything like it: a host can tell a plugin which outputs it listens to, never the other
//! way round. The plugin answers once, through [`OutputsInUse`], and each wrapper serves it in its
//! own format: CLAP through the extension `dk.mxm.outputs-in-use/1` (`clap/outputs_in_use.rs`),
//! VST3 through the interface `IOutputsInUse` (`vst3/outputs_in_use.rs`). The convention is
//! mxm-kit's `docs/plugin-conventions.md`, *Outputs in use*.
//!
//! [`UsedOutputs`] is the state both wrappers keep, as the note names' (`clap/note_names.rs`): a
//! change that may route a part elsewhere, on any thread, only [raises][UsedOutputs::raise()] a
//! flag; the check that reads the outputs and compares them runs on the main thread, and the flag
//! stays raised until a check takes it.

use atomic_refcell::AtomicRefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One output after the main one that something in a plugin sends to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInUse {
    /// Which output: its place among the plugin's output ports, the main one 0 and the
    /// auxiliary ones after it, in the first audio IO layout that has it, whatever layout is
    /// active. Each wrapper numbers it its own way: CLAP's port id, VST3's output bus index.
    pub output: u32,
    /// What plays there, for the host to name a track after: "Kick", or "Kick / Rim" when two
    /// parts share the output.
    pub name: String,
}

/// Where a plugin's outputs in use come from: `ClapPlugin::outputs_in_use()` and
/// `Vst3Plugin::outputs_in_use()`.
pub trait OutputsInUse: Send + Sync {
    /// The outputs in use now, in output order. Called on the main thread only, never the audio
    /// thread, so it may allocate.
    fn outputs(&self) -> Vec<OutputInUse>;
    /// The IDs of the parameters they depend on, their names included. A change to one of them
    /// makes the wrapper read them again; any other parameter's change doesn't.
    fn depends_on(&self) -> Vec<&'static str>;
    /// The parameter values that send whatever plays on `output` back to the main output, each
    /// its parameter's ID and plain value, for the host to set as it sets any value: a host whose
    /// track for the output goes moves its parts back to main this way, in the same undo step.
    /// Empty, the default, when nothing is routed there or the plugin can't say. Main thread only.
    fn to_main(&self, output: u32) -> Vec<(&'static str, f32)> {
        let _ = output;
        Vec::new()
    }
}

/// How a wrapper numbers an output ([`OutputInUse::output`]) in its format; `None` for one it
/// can't, which the host is then not told of.
pub(crate) type Numbering = fn(u32) -> Option<u32>;

/// A wrapper's state of the outputs in use. The outputs it keeps are numbered as its format
/// numbers them.
pub(crate) struct UsedOutputs {
    source: Arc<dyn OutputsInUse>,
    numbering: Numbering,
    /// The hashes of the parameters the outputs depend on.
    depends_on: HashSet<u32>,
    /// Raised by a change that may route a part elsewhere, taken by the check that reads them.
    pending: AtomicBool,
    /// The outputs `count()` last took, which `get()` serves. Main thread only.
    served: AtomicRefCell<Vec<OutputInUse>>,
    /// The outputs as the host last knew them: as `count()` took them, or as a check found them
    /// and told it. Main thread only.
    known: AtomicRefCell<Vec<OutputInUse>>,
}

impl UsedOutputs {
    /// Reads the outputs once, as the host would find them before anything changes. Called on the
    /// main thread while the wrapper is built.
    pub fn new(
        source: Arc<dyn OutputsInUse>,
        param_id_to_hash: &HashMap<String, u32>,
        numbering: Numbering,
    ) -> Self {
        let depends_on = source
            .depends_on()
            .into_iter()
            .filter_map(|id| {
                let hash = param_id_to_hash.get(id).copied();
                crate::nice_debug_assert!(
                    hash.is_some(),
                    "The outputs in use depend on {:?}, which is no parameter",
                    id
                );
                hash
            })
            .collect();
        let outputs = read(&*source, numbering);
        Self {
            source,
            numbering,
            depends_on,
            pending: AtomicBool::new(false),
            served: AtomicRefCell::new(outputs.clone()),
            known: AtomicRefCell::new(outputs),
        }
    }

    fn read(&self) -> Vec<OutputInUse> {
        read(&*self.source, self.numbering)
    }

    /// Whether a change to this parameter may change the outputs in use or their names.
    pub fn depends_on(&self, hash: u32) -> bool {
        self.depends_on.contains(&hash)
    }

    /// Marks the outputs as possibly changed, on any thread: no allocation, no lock. True when the
    /// caller must arrange a check; false when one is already pending.
    pub fn raise(&self) -> bool {
        !self.pending.swap(true, Ordering::AcqRel)
    }

    /// On the main thread: takes a raised flag, reads the outputs, and says whether they differ
    /// from what the host last knew, which they then become. False, reading nothing, when nothing
    /// was raised.
    pub fn check(&self) -> bool {
        if !self.pending.swap(false, Ordering::AcqRel) {
            return false;
        }
        let outputs = self.read();
        let mut known = self.known.borrow_mut();
        if *known == outputs {
            false
        } else {
            *known = outputs;
            true
        }
    }

    /// On the main thread, as the host asks how many there are: reads the outputs, which `get()`
    /// then serves.
    pub fn count(&self) -> u32 {
        let outputs = self.read();
        let count = outputs.len();
        *self.known.borrow_mut() = outputs.clone();
        *self.served.borrow_mut() = outputs;
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// On the main thread: one of the outputs `count()` took.
    pub fn get(&self, index: u32) -> Option<OutputInUse> {
        self.served.borrow().get(index as usize).cloned()
    }

    /// On the main thread: what sends output `output` (its place among the outputs, as
    /// [`OutputInUse::output`] gives it, before the format numbers it) back to main.
    pub fn to_main(&self, output: u32) -> Vec<(&'static str, f32)> {
        self.source.to_main(output)
    }
}

/// The outputs `source` says are in use now, numbered as the format numbers them.
fn read(source: &dyn OutputsInUse, numbering: Numbering) -> Vec<OutputInUse> {
    source
        .outputs()
        .into_iter()
        .filter_map(|output| {
            Some(OutputInUse {
                output: numbering(output.output)?,
                name: output.name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Source(Mutex<Vec<OutputInUse>>);

    impl OutputsInUse for Source {
        fn outputs(&self) -> Vec<OutputInUse> {
            self.0.lock().unwrap().clone()
        }

        fn depends_on(&self) -> Vec<&'static str> {
            vec!["output"]
        }
    }

    fn output(output: u32, name: &str) -> OutputInUse {
        OutputInUse {
            output,
            name: name.to_owned(),
        }
    }

    fn outputs_with(outputs: Vec<OutputInUse>, numbering: Numbering) -> (Arc<Source>, UsedOutputs) {
        let source = Arc::new(Source(Mutex::new(outputs)));
        let hashes = HashMap::from([("output".to_owned(), 7), ("level".to_owned(), 8)]);
        let in_use = UsedOutputs::new(source.clone(), &hashes, numbering);
        (source, in_use)
    }

    #[test]
    fn the_host_is_served_the_outputs_count_took() {
        let (source, in_use) = outputs_with(vec![output(1, "Kick"), output(2, "Snare")], Some);
        assert_eq!(in_use.count(), 2);
        *source.0.lock().unwrap() = vec![output(3, "Hat")];
        assert_eq!(in_use.get(0), Some(output(1, "Kick")), "served as counted");
        assert_eq!(in_use.get(2), None);
        assert_eq!(in_use.count(), 1);
        assert_eq!(in_use.get(0), Some(output(3, "Hat")));
    }

    #[test]
    fn each_output_is_numbered_as_the_format_numbers_it_and_one_it_cannot_is_left_out() {
        let (_, in_use) = outputs_with(vec![output(1, "Kick"), output(5, "Snare")], |output| {
            (output < 3).then_some(output + 10)
        });
        assert_eq!(in_use.count(), 1);
        assert_eq!(in_use.get(0), Some(output(11, "Kick")));
    }

    #[test]
    fn only_the_parameters_the_outputs_depend_on_count() {
        let (_, in_use) = outputs_with(Vec::new(), Some);
        assert!(in_use.depends_on(7));
        assert!(!in_use.depends_on(8));
    }

    #[test]
    fn a_change_is_told_once_and_only_when_the_outputs_differ() {
        let (source, in_use) = outputs_with(Vec::new(), Some);
        assert!(!in_use.check(), "nothing raised, nothing read");
        assert!(in_use.raise(), "the first raise arranges a check");
        assert!(!in_use.raise(), "a second waits for the same check");
        assert!(!in_use.check(), "the outputs are as the host knows them");

        *source.0.lock().unwrap() = vec![output(1, "Kick")];
        assert!(in_use.raise());
        assert!(in_use.check(), "routed: the host is told");
        assert!(!in_use.check(), "and only once");
        assert!(in_use.raise());
        assert!(!in_use.check(), "already told");
    }
}
