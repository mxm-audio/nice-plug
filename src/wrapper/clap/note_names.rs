//! MXM PATCH (note names): a capability, not a fix. See `PATCHES.md`, *Note names*.
//!
//! The wrapper's side of [`ClapPlugin::note_names()`][super::ClapPlugin::note_names()]: the names
//! the host is served, whether they may have changed, and whether the host must be told.
//!
//! A change that may rename a note can come on any thread: a parameter on the audio thread, a state
//! load or a GUI transaction on the main thread. It only [raises][NoteNames::raise()] a flag; the
//! check that reads the names and compares them runs on the main thread. A raised flag stays
//! raised until a check takes it, so a change whose task found the queue full is still told: the
//! wrapper checks again whenever the host calls it on the main thread.

use atomic_refcell::AtomicRefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{ClapNoteNames, NoteName};

pub(crate) struct NoteNames {
    source: Arc<dyn ClapNoteNames>,
    /// The hashes of the parameters the names depend on.
    depends_on: HashSet<u32>,
    /// Raised by a change that may rename a note, taken by the check that reads the names.
    pending: AtomicBool,
    /// The names `count()` last took, which `get()` serves. Main thread only.
    served: AtomicRefCell<Vec<NoteName>>,
    /// The names as the host last knew them: as `count()` took them, or as a check found them and
    /// told it. Main thread only.
    known: AtomicRefCell<Vec<NoteName>>,
}

impl NoteNames {
    /// Reads the names once, as the host would find them before anything changes. Called on the
    /// main thread while the wrapper is built.
    pub fn new(source: Arc<dyn ClapNoteNames>, param_id_to_hash: &HashMap<String, u32>) -> Self {
        let depends_on = source
            .depends_on()
            .into_iter()
            .filter_map(|id| {
                let hash = param_id_to_hash.get(id).copied();
                crate::nice_debug_assert!(
                    hash.is_some(),
                    "The note names depend on {:?}, which is no parameter",
                    id
                );
                hash
            })
            .collect();
        let names = source.names();
        Self {
            source,
            depends_on,
            pending: AtomicBool::new(false),
            served: AtomicRefCell::new(names.clone()),
            known: AtomicRefCell::new(names),
        }
    }

    /// Whether a change to this parameter may rename a note.
    pub fn depends_on(&self, hash: u32) -> bool {
        self.depends_on.contains(&hash)
    }

    /// Marks the names as possibly changed, on any thread: no allocation, no lock. True when the
    /// caller must arrange a check; false when one is already pending.
    pub fn raise(&self) -> bool {
        !self.pending.swap(true, Ordering::AcqRel)
    }

    /// On the main thread: takes a raised flag, reads the names, and says whether they differ from
    /// what the host last knew, which they then become. False, reading nothing, when nothing was
    /// raised. The flag is taken before the names are read, so a change while they're read raises
    /// it again and is checked in turn.
    pub fn check(&self) -> bool {
        if !self.pending.swap(false, Ordering::AcqRel) {
            return false;
        }
        let names = self.source.names();
        let mut known = self.known.borrow_mut();
        if *known == names {
            false
        } else {
            *known = names;
            true
        }
    }

    /// On the main thread, as the host asks how many there are: reads the names, which `get()`
    /// then serves.
    pub fn count(&self) -> u32 {
        let names = self.source.names();
        let count = names.len();
        *self.known.borrow_mut() = names.clone();
        *self.served.borrow_mut() = names;
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// On the main thread: one of the names `count()` took.
    pub fn get(&self, index: u32) -> Option<NoteName> {
        self.served.borrow().get(index as usize).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Source(Mutex<Vec<NoteName>>);

    impl ClapNoteNames for Source {
        fn names(&self) -> Vec<NoteName> {
            self.0.lock().unwrap().clone()
        }

        fn depends_on(&self) -> Vec<&'static str> {
            vec!["model"]
        }
    }

    fn name(key: i16, name: &str) -> NoteName {
        NoteName {
            name: name.to_owned(),
            port: 0,
            key,
            channel: -1,
        }
    }

    fn names_with(names: Vec<NoteName>) -> (Arc<Source>, NoteNames) {
        let source = Arc::new(Source(Mutex::new(names)));
        let hashes = HashMap::from([("model".to_owned(), 7), ("level".to_owned(), 8)]);
        let note_names = NoteNames::new(source.clone(), &hashes);
        (source, note_names)
    }

    #[test]
    fn the_host_is_served_the_names_count_took() {
        let (source, names) = names_with(vec![name(36, "bd1"), name(38, "sn1")]);
        assert_eq!(names.count(), 2);
        *source.0.lock().unwrap() = vec![name(36, "subkick")];
        assert_eq!(names.get(0), Some(name(36, "bd1")), "served as counted");
        assert_eq!(names.get(1), Some(name(38, "sn1")));
        assert_eq!(names.get(2), None);
        assert_eq!(names.count(), 1);
        assert_eq!(names.get(0), Some(name(36, "subkick")));
    }

    #[test]
    fn only_the_parameters_the_names_depend_on_count() {
        let (_, names) = names_with(Vec::new());
        assert!(names.depends_on(7));
        assert!(!names.depends_on(8));
    }

    #[test]
    fn a_change_is_told_once_and_only_when_the_names_differ() {
        let (source, names) = names_with(vec![name(36, "bd1")]);
        assert!(!names.check(), "nothing raised, nothing read");
        assert!(names.raise(), "the first raise arranges a check");
        assert!(!names.raise(), "a second waits for the same check");
        assert!(!names.check(), "the names are as the host knows them");

        *source.0.lock().unwrap() = vec![name(36, "subkick")];
        assert!(names.raise());
        assert!(names.check(), "renamed: the host is told");
        assert!(!names.check(), "and only once");
        assert!(names.raise());
        assert!(!names.check(), "already told");
    }

    #[test]
    fn a_raise_whose_check_never_ran_is_still_checked_later() {
        let (source, names) = names_with(vec![name(36, "bd1")]);
        *source.0.lock().unwrap() = vec![name(36, "subkick")];
        // The sender's task found the queue full: the flag stays raised, and nothing else changes.
        assert!(names.raise());
        // The wrapper's next main-thread callback checks it, with no further change.
        assert!(names.check());
    }

    #[test]
    fn names_the_host_has_read_since_are_not_told_again() {
        let (source, names) = names_with(vec![name(36, "bd1")]);
        *source.0.lock().unwrap() = vec![name(36, "subkick")];
        assert_eq!(names.count(), 1, "the host read them on its own");
        assert!(names.raise());
        assert!(!names.check(), "it knows them already");
    }
}
