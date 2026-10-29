use atomic_refcell::AtomicRefCell;
use crossbeam::atomic::AtomicCell;
use nice_plug_core::audio_setup::{AudioIOLayout, BufferConfig, ProcessMode};
#[cfg(feature = "editor")]
use nice_plug_core::context::gui::GuiContext;
use nice_plug_core::context::process::Transport;
#[cfg(feature = "editor")]
use nice_plug_core::editor::dpi::Size;
#[cfg(feature = "editor")]
use nice_plug_core::editor::{Editor, SpawnedEditor};
use nice_plug_core::midi::{MidiConfig, PluginNoteEvent};
use nice_plug_core::params::internals::ParamPtr;
use nice_plug_core::params::{ParamFlags, Params};
#[cfg(feature = "editor")]
use nice_plug_core::plugin::TrackInfo;
use nice_plug_core::plugin::{Plugin, PluginState, ProcessStatus, TaskExecutor};
use parking_lot::Mutex;
#[cfg(feature = "editor")]
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use try_lock::TryLock;
#[cfg(feature = "editor")]
use vst3::ComWrapper;
use vst3::Steinberg::Vst::{IComponentHandler, IComponentHandlerTrait, IEventList, RestartFlags_};
#[cfg(feature = "editor")]
use vst3::Steinberg::Vst::{IComponentHandler2, IComponentHandler2Trait};
use vst3::Steinberg::{kInvalidArgument, kResultOk, tresult};
use vst3::{ComPtr, ComRef};

use super::context::{WrapperActivateContext, WrapperProcessContext};
use super::note_expressions::NoteExpressionController;
use super::param_units::ParamUnits;
use super::util::{VST3_MIDI_PARAMS_END, VST3_MIDI_PARAMS_START};
use crate::event_loop::{EventLoop, MainThreadExecutor, OsEventLoop};
use crate::util::permit_alloc;
use crate::wrapper::state;
use crate::wrapper::util::buffer_management::BufferManager;
use crate::wrapper::util::hash_param_id;
use crate::wrapper::util::process_wrapper;
use crate::wrapper::vst3::Vst3Plugin;
#[cfg(feature = "editor")]
use crate::wrapper::vst3::context::WrapperGuiContext;
#[cfg(feature = "editor")]
use crate::wrapper::vst3::view::WrapperView;

/// The actual wrapper bits. We need this as an `Arc<T>` so we can safely use our event loop API.
/// Since we can't combine that with VST3's interior reference counting this just has to be moved to
/// its own struct.
pub(crate) struct WrapperInner<P: Vst3Plugin> {
    /// The wrapped plugin instance.
    pub plugin: TryLock<P>,
    /// The plugin's background task executor closure.
    pub task_executor: Mutex<TaskExecutor<P>>,
    /// The plugin's parameters. These are fetched once during initialization. That way the
    /// `ParamPtr`s are guaranteed to live at least as long as this object and we can interact with
    /// the `Params` object without having to acquire a lock on `plugin`.
    pub params: Arc<dyn Params>,
    /// The plugin's editor, if it has one. This object does not do anything on its own, but we need
    /// to instantiate this in advance so we don't need to lock the entire [`Plugin`] object when
    /// creating an editor. Wrapped in an `AtomicRefCell` because it needs to be initialized late.
    #[allow(clippy::type_complexity)]
    #[cfg(feature = "editor")]
    pub editor: AtomicRefCell<Option<Arc<Mutex<P::Editor>>>>,
    #[allow(clippy::type_complexity)]
    // My personal record for craziest Rust type!!!!
    #[cfg(feature = "editor")]
    pub editor_window:
        Arc<AtomicRefCell<Option<fragile::Fragile<SpawnedEditor<<P::Editor as Editor>::Handle>>>>>,

    /// The host's [`IComponentHandler`] instance, if passed through
    /// [`IEditController::set_component_handler`].
    pub component_handler: AtomicRefCell<Option<ComPtr<IComponentHandler>>>,

    /// Our own [`IPlugView`] instance.
    #[cfg(feature = "editor")]
    pub plug_view: RwLock<Option<ComWrapper<WrapperView<P>>>>,

    /// Whether the editor is currently open. This is changed by the attached and removed functions on IPlugViewTrait
    #[cfg(feature = "editor")]
    pub is_editor_open: AtomicBool,

    /// A realtime-safe task queue so the plugin can schedule tasks that need to be run later on the
    /// GUI thread. This field should not be used directly for posting tasks. This should be done
    /// through [`Self::schedule_gui()`] instead. That method posts the task to the host's
    /// `IRunLoop` instead of it's available.
    ///
    /// This AtomicRefCell+Option is only needed because it has to be initialized late. There is no
    /// reason to mutably borrow the event loop, so reads will never be contested.
    ///
    /// TODO: Is there a better type for Send+Sync late initialization?
    pub event_loop: AtomicRefCell<Option<OsEventLoop<Task<P>, Self>>>,

    /// Whether the plugin is currently processing audio. In other words, the last state
    /// `IAudioProcessor::setActive()` has been called with.
    pub is_processing: AtomicBool,
    /// The current audio IO layout. Modified through `IAudioProcessor::setBusArrangements()` after
    /// matching the proposed bus arrangement to one of the supported ones. The plugin's first audio
    /// IO layout is chosen as the default. Because of the way VST3 works it's not possible to
    /// change the number of busses from that default, only the channel counts can change.
    pub current_audio_io_layout: AtomicCell<AudioIOLayout>,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `IAudioProcessor::setupProcessing()`.
    pub current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// The current audio processing mode. Set in `IAudioProcessor::setup_processing()`.
    pub current_process_mode: AtomicCell<ProcessMode>,
    /// The most recently reported track information. Hosts may send partial updates (e.g.
    /// Ableton), so this is used to merge successive [`IInfoListener::setChannelContextInfos()`]
    /// calls.
    #[cfg(feature = "editor")]
    pub current_track_info: AtomicRefCell<TrackInfo>,

    /// The last process status returned by the plugin. This is used for tail handling.
    pub last_process_status: AtomicCell<ProcessStatus>,
    /// The current latency in samples, as set by the plugin through the [`ActivateContext`] and the
    /// [`ProcessContext`].
    pub current_latency: AtomicU32,
    /// A data structure that helps manage and create buffers for all of the plugin's inputs and
    /// outputs based on channel pointers provided by the host.
    pub buffer_manager: AtomicRefCell<BufferManager>,
    /// The incoming events for the plugin, if `P::ACCEPTS_MIDI` is set. If
    /// `P::SAMPLE_ACCURATE_AUTOMATION`, this is also read in lockstep with the parameter change
    /// block splitting.
    ///
    /// NOTE: Because with VST3 MIDI CC messages are sent as parameter changes and VST3 does not
    ///       interleave parameter changes and note events, this queue has to be sorted when
    ///       creating the process context
    pub input_note_events: AtomicRefCell<VecDeque<PluginNoteEvent<P>>>,
    /// VST3 has several useful predefined note expressions, but for some reason they are the only
    /// note event type that don't have MIDI note ID and channel fields. So we need to keep track of
    /// the most recent VST3 note IDs we've seen, and then map those back to MIDI note IDs and
    /// channels as needed.
    pub note_expression_controller: AtomicRefCell<NoteExpressionController>,
    /// Unprocessed parameter changes and note events sent by the host during a process call.
    /// Parameter changes are sent as separate queues for each parameter, and note events are in
    /// another queue on top of that. And if `P::MIDI_INPUT >= MidiConfig::MidiCCs`, then we can
    /// also receive MIDI CC messages through special parameter changes. On top of that, we also
    /// support sample accurate automation through block splitting if
    /// `P::SAMPLE_ACCURATE_AUTOMATION` is set. To account for all of this, we'll read all of the
    /// parameter changes and events into a vector at the start of the process call, sort it, and
    /// then do the block splitting based on that. Note events need to have their timing adjusted to
    /// match the block start, since they're all read upfront.
    pub process_events: AtomicRefCell<Vec<ProcessEvent<P>>>,
    /// MXM PATCH (defect 8, ported to VST3): counts each state transaction's start and end, so it is
    /// odd while one runs. `process()` reads it before and after a `try_lock()` that fails: odd, or
    /// moved, and the lock was a transaction's, so the block outputs silence and carries on rather
    /// than reporting an error the host may hold against the plugin.
    pub state_transactions: AtomicU64,
    /// Whether the host has the plugin active (`IComponent::setActive`): a state transaction
    /// reactivates it only then.
    pub is_active: AtomicBool,
    /// MXM PATCH (suspended processing): set while a `process()` call runs ([`InProcess`]).
    pub in_process: AtomicBool,
    /// MXM PATCH (suspended processing): how many threads are applying pending edits, which
    /// [`InProcess::begin`] and a state transaction wait out.
    pub appliers: AtomicU32,
    /// MXM PATCH (suspended processing): per parameter, in `param_hashes`' order, an edit made
    /// while the host had the plugin processing and not yet applied ([`pending`]), or 0.
    pub pending_edits: Vec<AtomicU64>,
    /// Whether a slot of `pending_edits` may be set.
    pub edits_pending: AtomicBool,
    /// Each parameter's place in `param_hashes`.
    pub param_index_by_hash: HashMap<u32, usize>,

    /// The keys from `param_map` in a stable order.
    pub param_hashes: Vec<u32>,
    /// A mapping from parameter ID hashes (obtained from the string parameter IDs) to pointers to
    /// parameters belonging to the plugin. These addresses will remain stable as long as the
    /// `params` object does not get deallocated.
    pub param_by_hash: HashMap<u32, ParamPtr>,
    /// Mappings from parameter hashes to string parameter IDs. Used for notifying the plugin's
    /// editor about parameter changes.
    pub param_id_by_hash: HashMap<u32, String>,
    pub param_units: ParamUnits,
    /// Mappings from string parameter identifiers to parameter hashes. Useful for debug logging
    /// and when storing and restoring plugin state.
    pub param_id_to_hash: HashMap<String, u32>,
    /// The inverse mapping from [`param_by_hash`][Self::param_by_hash]. This is needed to be able
    /// to have an ergonomic parameter setting API that uses references to the parameters instead of
    /// having to add a setter function to the parameter (or even worse, have it be completely
    /// untyped).
    pub param_ptr_to_hash: HashMap<ParamPtr, u32>,
}

/// Tasks that can be sent from the plugin to be executed on the main thread in a non-blocking
/// realtime-safe way (either a random thread or `IRunLoop` on Linux, the OS' message loop on
/// Windows and macOS).
#[allow(clippy::enum_variant_names)]
pub enum Task<P: Plugin> {
    /// Execute one of the plugin's background tasks.
    PluginTask(P::BackgroundTask),
    /// Inform the plugin that one or more parameter values have changed.
    #[cfg(feature = "editor")]
    StateChanged,
    /// Inform the plugin that one parameter's value has changed. This uses the parameter hashes
    /// since the task will be created from the audio thread.
    #[cfg(feature = "editor")]
    ParameterValueChanged(u32, f32),
    /// Trigger a restart with the given restart flags. This is a bit set of the flags from
    /// [`vst3::Steinberg::Vst::RestartFlags`].
    TriggerRestart(i32),
    // Request the editor to be resized according to its current size. Right now there is no way to
    // handle "denied resize" requests yet.
    #[cfg(feature = "editor")]
    RequestResize { size: Size, scale_factor: f64 },
    #[cfg(feature = "editor")]
    CallMainThread,
}

/// VST3 makes audio processing pretty complicated. In order to support both block splitting for
/// sample accurate automation and MIDI CC handling through parameters we need to put all parameter
/// changes and (translated) note events into a sorted array first.
#[derive(Debug, PartialEq)]
pub enum ProcessEvent<P: Plugin> {
    /// An incoming parameter change sent by the host. This will only be used when sample accurate
    /// automation has been enabled, and the parameters are only updated when we process this
    /// spooled event at the start of a block.
    ParameterChange {
        /// The event's sample offset within the buffer. Used for sorting.
        timing: u32,
        /// The parameter's hash, as used everywhere else.
        hash: u32,
        /// The normalized values, as provided by the host.
        normalized_value: f32,
    },
    /// An incoming parameter change sent by the host. This will only be used when sample accurate
    /// automation has been enabled, and the parameters are only updated when we process this
    /// spooled event at the start of a block.
    ///
    /// The timing stored within the note event needs to have the block start index subtraced from
    /// it. make sure to subtract the block start index with [`NoteEvent::subtract_timing()`] before
    /// putting this into the input event queue.
    NoteEvent(PluginNoteEvent<P>),
}

impl<P: Plugin> ProcessEvent<P> {
    #[inline]
    pub(crate) fn timing(&self) -> u32 {
        match self {
            ProcessEvent::ParameterChange { timing, .. } => *timing,
            ProcessEvent::NoteEvent(event) => event.timing(),
        }
    }
}

impl<P: Vst3Plugin> WrapperInner<P> {
    #[allow(unused_unsafe)]
    pub fn new() -> Arc<Self> {
        let mut plugin = P::default();
        let task_executor = Mutex::new(plugin.task_executor());

        // This is a mapping from the parameter IDs specified by the plugin to pointers to those
        // parameters. These pointers are assumed to be safe to dereference as long as
        // `wrapper.plugin` is alive. The plugin API identifiers these parameters by hashes, which
        // we'll calculate from the string ID specified by the plugin. These parameters should also
        // remain in the same order as the one returned by the plugin.
        let params = plugin.params();
        let param_id_hashes_ptrs_groups: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, ptr, group)| {
                let hash = hash_param_id(&id);
                (id, hash, ptr, group)
            })
            .collect();
        if cfg!(debug_assertions) {
            let param_map = params.param_map();
            let param_ids: HashSet<_> = param_id_hashes_ptrs_groups
                .iter()
                .map(|(id, _, _, _)| id.clone())
                .collect();
            crate::nice_debug_assert_eq!(
                param_map.len(),
                param_ids.len(),
                "The plugin has duplicate parameter IDs, weird things may happen. Consider using \
                 6 character parameter IDs to avoid collisions."
            );

            let mut bypass_param_exists = false;
            for (id, hash, ptr, _) in &param_id_hashes_ptrs_groups {
                let flags = unsafe { ptr.flags() };
                let is_bypass = flags.contains(ParamFlags::BYPASS);

                if is_bypass && bypass_param_exists {
                    crate::nice_debug_assert_failure!(
                        "Duplicate bypass parameters found, the host will only use the first one"
                    );
                }

                bypass_param_exists |= is_bypass;

                if P::MIDI_INPUT >= MidiConfig::MidiCCs
                    && (VST3_MIDI_PARAMS_START..VST3_MIDI_PARAMS_END).contains(hash)
                {
                    crate::nice_debug_assert_failure!(
                        "Parameter '{}' collides with an automatically generated MIDI CC \
                         parameter, consider giving it a different ID",
                        id
                    );
                }
            }
        }

        let param_hashes: Vec<u32> = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, _, _)| *hash)
            .collect();
        let param_index_by_hash = param_hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| (*hash, index))
            .collect();
        let pending_edits = param_hashes.iter().map(|_| AtomicU64::new(0)).collect();
        let param_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(_, hash, ptr, _)| (*hash, *ptr))
            .collect();
        let param_id_by_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(id, hash, _, _)| (*hash, id.clone()))
            .collect();
        let param_units = ParamUnits::from_param_groups(
            param_id_hashes_ptrs_groups
                .iter()
                .map(|(_, hash, _, group_name)| (*hash, group_name.as_str())),
        )
        .expect("Inconsistent parameter groups");
        let param_id_to_hash = param_id_hashes_ptrs_groups
            .iter()
            .map(|(id, hash, _, _)| (id.clone(), *hash))
            .collect();
        let param_ptr_to_hash = param_id_hashes_ptrs_groups
            .into_iter()
            .map(|(_, hash, ptr, _)| (ptr, hash))
            .collect();

        let input_note_events = if P::MIDI_INPUT == MidiConfig::None {
            AtomicRefCell::new(VecDeque::new())
        } else {
            AtomicRefCell::new(VecDeque::with_capacity(P::INPUT_EVENT_CAPACITY))
        };

        let wrapper = Arc::new(Self {
            plugin: TryLock::new(plugin),
            task_executor,
            params,
            // Initialized later as it needs a reference to the wrapper for the async executor
            #[cfg(feature = "editor")]
            editor: AtomicRefCell::new(None),
            #[cfg(feature = "editor")]
            editor_window: Arc::new(AtomicRefCell::new(None)),

            component_handler: AtomicRefCell::new(None),

            #[cfg(feature = "editor")]
            plug_view: RwLock::new(None),

            #[cfg(feature = "editor")]
            is_editor_open: AtomicBool::new(false),

            event_loop: AtomicRefCell::new(None),

            is_processing: AtomicBool::new(false),
            // Some hosts, like the current version of Bitwig and Ardour at the time of writing,
            // will try using the plugin's default not yet initialized bus arrangement. Because of
            // that, we'll always initialize this configuration even before the host requests a
            // channel layout.
            current_audio_io_layout: AtomicCell::new(
                P::AUDIO_IO_LAYOUTS.first().copied().unwrap_or_default(),
            ),
            current_buffer_config: AtomicCell::new(None),
            current_process_mode: AtomicCell::new(ProcessMode::Realtime),
            #[cfg(feature = "editor")]
            current_track_info: AtomicRefCell::new(TrackInfo::default()),
            last_process_status: AtomicCell::new(ProcessStatus::Normal),
            current_latency: AtomicU32::new(0),
            // This is initialized just before calling `Plugin::activate()` so that during the
            // process call buffers can be initialized without any allocations
            buffer_manager: AtomicRefCell::new(BufferManager::for_audio_io_layout(
                0,
                AudioIOLayout::default(),
            )),
            input_note_events,
            note_expression_controller: AtomicRefCell::new(NoteExpressionController::default()),
            process_events: AtomicRefCell::new(Vec::with_capacity(P::INPUT_EVENT_CAPACITY)),
            state_transactions: AtomicU64::new(0),
            is_active: AtomicBool::new(false),
            in_process: AtomicBool::new(false),
            appliers: AtomicU32::new(0),
            pending_edits,
            edits_pending: AtomicBool::new(false),
            param_index_by_hash,

            param_hashes,
            param_by_hash,
            param_id_by_hash,
            param_units,
            param_id_to_hash,
            param_ptr_to_hash,
        });

        // FIXME: Right now this is safe, but if we are going to have a singleton main thread queue
        //        serving multiple plugin instances, Arc can't be used because its reference count
        //        is separate from the internal COM-style reference count.
        *wrapper.event_loop.borrow_mut() =
            Some(OsEventLoop::new_and_spawn(Arc::downgrade(&wrapper)));

        // The editor also needs to be initialized later so the Async executor can work.

        #[cfg(feature = "editor")]
        {
            *wrapper.editor.borrow_mut() = wrapper
                .plugin
                .try_lock()
                .unwrap()
                .editor(nice_plug_core::context::gui::AsyncExecutor::new(
                    Arc::new({
                        let wrapper = Arc::downgrade(&wrapper);
                        move |task| {
                            let wrapper = match wrapper.upgrade() {
                                Some(wrapper) => wrapper,
                                None => return,
                            };

                            let task_posted = wrapper.schedule_background(Task::PluginTask(task));
                            crate::nice_debug_assert!(
                                task_posted,
                                "The task queue is full, dropping task..."
                            );
                        }
                    }),
                    Arc::new({
                        let wrapper = Arc::downgrade(&wrapper);
                        move |task| {
                            let wrapper = match wrapper.upgrade() {
                                Some(wrapper) => wrapper,
                                None => return,
                            };

                            let task_posted = wrapper.schedule_gui(Task::PluginTask(task));
                            crate::nice_debug_assert!(
                                task_posted,
                                "The task queue is full, dropping task..."
                            );
                        }
                    }),
                ))
                .map(|editor| Arc::new(Mutex::new(editor)));
        }

        wrapper
    }

    #[cfg(feature = "editor")]
    pub fn make_gui_context(self: Arc<Self>) -> GuiContext {
        GuiContext::new(Arc::new(WrapperGuiContext {
            inner: Arc::downgrade(&self),
            #[cfg(debug_assertions)]
            param_gesture_checker: Default::default(),
        }))
    }

    /// # Note
    ///
    /// The lock on the plugin must be dropped before this object is dropped to avoid deadlocks
    /// caused by reentrant function calls.
    pub fn make_activate_context(&self) -> WrapperActivateContext<'_, P> {
        WrapperActivateContext {
            inner: self,
            pending_requests: Default::default(),
        }
    }

    pub fn make_process_context<'a>(
        &'a self,
        transport: Transport,
        host_out_events: Option<ComRef<'a, IEventList>>,
        total_buffer_len: usize,
        current_sample_idx: usize,
    ) -> WrapperProcessContext<'a, P> {
        WrapperProcessContext {
            inner: self,
            input_events_guard: self.input_note_events.borrow_mut(),
            transport,
            host_out_events,
            total_buffer_len: total_buffer_len as u32,
            current_sample_idx: current_sample_idx as u32,
        }
    }

    /// Posts the task to the background task queue using [`EventLoop::schedule_background()`] so it
    /// can be run in the background without blocking either the GUI or the audio thread.
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    pub fn schedule_background(&self, task: Task<P>) -> bool {
        let event_loop = self.event_loop.borrow();
        let event_loop = event_loop.as_ref().unwrap();
        event_loop.schedule_background(task)
    }

    /// Either posts the task to the task queue using [`EventLoop::schedule_gui()`] so it can be
    /// delegated to the main thread, executes the task directly if this is the main thread, or runs
    /// the task on the host's `IRunLoop` if the GUI is open and it exposes one.
    ///
    /// If the task queue is full, then this will return false.
    #[must_use]
    pub fn schedule_gui(&self, task: Task<P>) -> bool {
        let event_loop = self.event_loop.borrow();
        let event_loop = event_loop.as_ref().unwrap();
        if event_loop.is_main_thread() {
            self.execute(task, true);
            true
        } else {
            // If the editor is open, and the host exposes the `IRunLoop` interface, then we'll run
            // the task on the host's GUI thread using that interface. Otherwise we'll use the
            // regular event loop. If the editor gets dropped while there's still outstanding work
            // left in the run loop task queue, then those tasks will be posted to the regular event
            // loop so no work is lost.
            #[cfg(feature = "editor")]
            if self.is_editor_open.load(Ordering::Relaxed) {
                match self
                    .plug_view
                    .read()
                    .clone()
                    .unwrap()
                    .do_maybe_in_run_loop(task)
                {
                    Ok(()) => true,
                    Err(task) => event_loop.schedule_gui(task),
                }
            } else {
                event_loop.schedule_gui(task)
            }

            #[cfg(not(feature = "editor"))]
            event_loop.schedule_gui(task)
        }
    }

    /// Get a parameter's ID based on a `ParamPtr`. Used in the `GuiContext` implementation for the
    /// gesture checks.
    #[allow(unused)]
    pub fn param_id_from_ptr(&self, param: ParamPtr) -> Option<&str> {
        self.param_ptr_to_hash
            .get(&param)
            .and_then(|hash| self.param_id_by_hash.get(hash))
            .map(|s| s.as_str())
    }

    /// Convenience function for setting a value for a parameter as triggered by a VST3 parameter
    /// update. The same rate is for updating parameter smoothing.
    ///
    /// After calling this function, you should call
    /// [`notify_param_values_changed()`][Self::notify_param_values_changed()] to allow the editor
    /// to update itself. This needs to be done separately so you can process parameter changes in
    /// batches.
    pub fn set_normalized_value_by_hash(
        &self,
        hash: u32,
        normalized_value: f32,
        sample_rate: Option<f32>,
    ) -> tresult {
        if !normalized_value.is_finite() {
            return kInvalidArgument;
        }

        match self.param_by_hash.get(&hash) {
            Some(param_ptr) => {
                if unsafe { param_ptr._internal_set_normalized_value(normalized_value) } {
                    if let Some(sample_rate) = sample_rate {
                        unsafe { param_ptr._internal_update_smoother(sample_rate, false) };
                    }

                    #[cfg(feature = "editor")]
                    {
                        let task_posted =
                            self.schedule_gui(Task::ParameterValueChanged(hash, normalized_value));
                        crate::nice_debug_assert!(
                            task_posted,
                            "The task queue is full, dropping task..."
                        );
                    }
                }

                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    /// Get the plugin's state object, may be called by the plugin's GUI as part of its own preset
    /// management. The wrapper doesn't use these functions and serializes and deserializes directly
    /// the JSON in the relevant plugin API methods instead.
    #[cfg(feature = "editor")]
    pub fn get_state_object(&self) -> PluginState {
        // MXM PATCH (suspended processing): with the pending edits applied, as `getState()` saves
        // it, so an editor that sends this back as a full state does not undo them (VST3 review,
        // 2026-10-29).
        self.settle_pending_edits();
        unsafe {
            state::serialize_object::<P>(
                self.params.clone(),
                state::make_params_iter(&self.param_by_hash, &self.param_id_to_hash),
            )
        }
    }

    /// Update the plugin's state from its editor, on the GUI thread.
    ///
    /// MXM PATCH (defect 8 and GUI-authored state dirty, ported to VST3): a state with no
    /// parameters whose fields are the ones installed only tells the host there is something to
    /// save, through `IComponentHandler2::setDirty`; anything else is
    /// [`set_state_inner()`][Self::set_state_inner]'s transaction, here, off the audio thread.
    /// Upstream handed the state to the audio thread over a zero-capacity channel and waited for a
    /// block to take it, without end while the host had the plugin processing but ran no block:
    /// DaVinci Resolve stopped responding (2026-10-29). It also restored the state, allocating, on
    /// the audio thread.
    #[cfg(feature = "editor")]
    pub fn set_state_object_from_gui(&self, mut state: PluginState) {
        if state::gui_state_is_dirty_only(&state, &self.params.serialize_fields()) {
            self.mark_dirty();
            return;
        }
        if self.set_state_inner(&mut state) {
            self.mark_dirty();
        }

        // After the state has been updated, notify the host about the new parameter values
        let task_posted =
            self.schedule_gui(Task::TriggerRestart(RestartFlags_::kParamValuesChanged));
        crate::nice_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    /// MXM PATCH (GUI-authored state dirty, ported to VST3): tells the host the plugin's state
    /// changed outside its parameters, where the host has `IComponentHandler2`.
    #[cfg(feature = "editor")]
    fn mark_dirty(&self) {
        if let Some(handler) = &*self.component_handler.borrow()
            && let Some(handler) = handler.cast::<IComponentHandler2>()
        {
            unsafe { handler.setDirty(1) };
        }
    }

    pub fn set_latency_samples(&self, samples: u32) {
        // Only trigger a restart if it's actually needed
        let old_latency = self.current_latency.swap(samples, Ordering::SeqCst);
        if old_latency != samples {
            self.request_restart();
        }
    }

    pub fn request_restart(&self) {
        let task_posted = self.schedule_gui(Task::TriggerRestart(RestartFlags_::kLatencyChanged));
        crate::nice_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    /// MXM PATCH (suspended processing): an edit to a parameter from the editor or the host's
    /// controller. Upstream applied it only while the host did not have the plugin processing, and
    /// otherwise left it for a block to bring; a host that keeps the plugin processing and runs no
    /// block (DaVinci Resolve with an idle track, REAPER with a bypassed plugin) never applied it,
    /// and the editor showed the edit undone. Now, while processing, it is a pending slot, applied
    /// here unless a `process()` call is running, and otherwise by that call as it ends
    /// ([`InProcess`]): applied either way, never by this thread during a call. The host's own copy,
    /// if it brings one, sets the same value again.
    pub fn set_or_defer(&self, hash: u32, normalized_value: f32) -> tresult {
        if !self.is_processing.load(Ordering::SeqCst) {
            return self.set_normalized_value_by_hash(hash, normalized_value, self.sample_rate());
        }
        let Some(&index) = self.param_index_by_hash.get(&hash) else {
            return kInvalidArgument;
        };
        if !normalized_value.is_finite() {
            return kInvalidArgument;
        }
        // The edit's generation is the transaction count it read: one made after a state
        // transaction began reads that transaction's own (odd) count, the boundary it keeps.
        // Published by compare-exchange and the count read again after: if it moved, the edit is
        // published again with the count now, so no slot keeps an edit under a generation older
        // than the transaction it was stored in, which would let that transaction's success drop
        // it, or a newer edit it overwrote (VST3 review, 2026-10-29).
        let slot = &self.pending_edits[index];
        loop {
            let generation = self.state_transactions.load(Ordering::SeqCst) as u32;
            let edit = pending(normalized_value, generation);
            let current = slot.load(Ordering::SeqCst);
            if slot
                .compare_exchange(current, edit, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
                && self.state_transactions.load(Ordering::SeqCst) as u32 == generation
            {
                break;
            }
        }
        self.edits_pending.store(true, Ordering::SeqCst);
        self.apply_pending_edits_if_idle();
        kResultOk
    }

    /// Applies the pending edits here, off the audio thread, if no `process()` call is running:
    /// Dekker's handshake with `InProcess::begin`, of `appliers` against `in_process`, so at least
    /// one of the two sees the other.
    fn apply_pending_edits_if_idle(&self) {
        self.appliers.fetch_add(1, Ordering::SeqCst);
        if !self.in_process.load(Ordering::SeqCst) {
            self.apply_pending_edits();
        }
        self.appliers.fetch_sub(1, Ordering::SeqCst);
    }

    /// Applies each pending edit once, whichever thread takes it; none while a state transaction
    /// runs, which supersedes the edits made before it. The caller is counted in `appliers`, and a
    /// transaction counts itself in `state_transactions` before it waits the appliers out, so of
    /// the two at least one sees the other.
    pub fn apply_pending_edits(&self) {
        if self.state_transactions.load(Ordering::SeqCst) % 2 == 1 {
            return;
        }
        if !self.edits_pending.swap(false, Ordering::SeqCst) {
            return;
        }
        let sample_rate = self.sample_rate();
        for (slot, &hash) in self.pending_edits.iter().zip(&self.param_hashes) {
            if let Some(value) = taken(slot.swap(0, Ordering::SeqCst)) {
                self.set_normalized_value_by_hash(hash, value, sample_rate);
            }
        }
    }

    fn sample_rate(&self) -> Option<f32> {
        self.current_buffer_config.load().map(|c| c.sample_rate)
    }

    /// A state transaction's start, counted in already: waits out the threads applying pending
    /// edits, for a second at most. `false` if one is still applying, and the transaction must not
    /// go on: it would write the parameters with that thread.
    fn appliers_drained(&self) -> bool {
        let started = Instant::now();
        while self.appliers.load(Ordering::SeqCst) != 0 {
            if started.elapsed() > Duration::from_secs(1) {
                return false;
            }
            std::thread::yield_now();
        }
        true
    }

    /// Drops the pending edits made before the state transaction of generation `boundary`, which
    /// the state it loaded supersedes: those of other generations. One made after it began carries
    /// `boundary` and stays, and the compare-exchange keeps one that replaced an older edit
    /// meanwhile. A refused transaction drops none (VST3 review, 2026-10-29).
    fn supersede_pending_edits(&self, boundary: u32) {
        for slot in &self.pending_edits {
            let edit = slot.load(Ordering::SeqCst);
            if taken(edit).is_some() && generation_of(edit) != boundary {
                let _ = slot.compare_exchange(edit, 0, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
    }

    /// MXM PATCH (suspended processing): before a save, the pending edits made so far applied to
    /// the parameters, so the state saved holds every edit the host was told of before it asked,
    /// saved as any value is (an enum by its stable ID). A running call applies them as it ends,
    /// this thread when none runs; it waits, off the audio thread, for that call or a state
    /// transaction on another thread to end, for `SETTLE_WAIT` at most, and the save then goes on
    /// without them rather than leave the host hanging (VST3 review, 2026-10-29).
    pub fn settle_pending_edits(&self) {
        let started = Instant::now();
        loop {
            self.apply_pending_edits_if_idle();
            // The flag first, then the appliers: an applier clears the flag before it writes, so
            // one caught between the two is still counted, and the save waits for its write.
            let settled = !self.edits_pending.load(Ordering::SeqCst)
                && self.appliers.load(Ordering::SeqCst) == 0;
            if settled || started.elapsed() > SETTLE_WAIT {
                return;
            }
            std::thread::yield_now();
        }
    }

    /// A state transaction's end, on every path, a refusal's too: counted out, then the pending
    /// edits applied if no call runs, a refused load's and those made while it ran.
    fn finish_state_transaction(&self) {
        self.state_transactions.fetch_add(1, Ordering::SeqCst);
        self.apply_pending_edits_if_idle();
    }

    /// Immediately set the plugin state, as one transaction: `false`, with the previous state
    /// back, if a parameter or field refused it or reactivation failed. Called by the host's
    /// `setState()` and by [`set_state_object_from_gui()`][Self::set_state_object_from_gui], never
    /// on the audio thread.
    ///
    /// Implicitly emits `Task::StateChanged`.
    ///
    /// # Notes
    ///
    /// `self.plugin` must _not_ be locked while calling this function or it will deadlock.
    pub fn set_state_inner(&self, state: &mut PluginState) -> bool {
        // MXM PATCH (defect 8, ported to VST3): the CLAP wrapper's transaction. The working
        // snapshot is taken and the plugin lock held across deserialize + reactivate, so a state
        // a field refuses (defects 9 and 11) or an activation that fails leaves parameters, fields
        // and engine as they were. Upstream's VST3 wrapper wrote parameters and fields first and
        // returned `false` with no rollback, leaving a mix of the old state and the new.
        let audio_io_layout = self.current_audio_io_layout.load();
        let buffer_config = self.current_buffer_config.load();
        let reactivate = buffer_config.filter(|_| self.is_active.load(Ordering::SeqCst));
        // Made before the lock and dropped after it, as `setActive()` does: dropping it may ask
        // the host for a restart, which a host may answer at once by taking the lock.
        let mut activate_context = self.make_activate_context();
        // Counted in, which also begins its edit generation, in one step before either wait:
        // appliers stop, and an edit made from here on reads this count and stays.
        let boundary =
            (self.state_transactions.fetch_add(1, Ordering::SeqCst) + 1) as u32 & GENERATION_MASK;
        if !self.appliers_drained() {
            self.finish_state_transaction();
            crate::nice_error!("Failed to load state: an edit was still being applied");
            return false;
        }
        let Some(mut plugin) = self.lock_plugin_for_state_transaction() else {
            self.finish_state_transaction();
            crate::nice_error!("Failed to load state: could not acquire the plugin lock");
            return false;
        };
        // The attempt and its rollback under one number (`state::state_transaction`).
        let transaction = state::TransactionScope::begin();
        let mut previous = permit_alloc(|| unsafe {
            state::serialize_object::<P>(
                self.params.clone(),
                state::make_params_iter(&self.param_by_hash, &self.param_id_to_hash),
            )
        });
        let mut success = permit_alloc(|| unsafe {
            state::deserialize_object::<P>(
                state,
                self.params.clone(),
                state::make_params_getter(&self.param_by_hash, &self.param_id_to_hash),
                buffer_config.as_ref(),
            )
        });
        // Whether the requested state was activated: only then does a rollback reactivate. A state
        // refused before it would otherwise rebuild a working engine that never changed.
        let mut activated = false;
        if success && let Some(buffer_config) = reactivate {
            activated = true;
            success = permit_alloc(|| {
                plugin.activate(&audio_io_layout, &buffer_config, &mut activate_context)
            });
            if success {
                process_wrapper(|| plugin.reset());
            }
        }
        if !success {
            let restored = permit_alloc(|| {
                state::as_rollback(|| unsafe {
                    state::deserialize_object::<P>(
                        &mut previous,
                        self.params.clone(),
                        state::make_params_getter(&self.param_by_hash, &self.param_id_to_hash),
                        buffer_config.as_ref(),
                    )
                })
            });
            let mut reactivated = restored;
            if restored
                && activated
                && let Some(buffer_config) = reactivate
            {
                // No reset: a rejected state must be observationally absent, as in the CLAP
                // wrapper, and the working engine keeps its history.
                reactivated = permit_alloc(|| {
                    plugin.activate(&audio_io_layout, &buffer_config, &mut activate_context)
                });
            }
            crate::nice_debug_assert!(restored && reactivated, "Rolling back plugin state failed");
        }
        if success {
            self.supersede_pending_edits(boundary);
        }
        drop(transaction);
        drop(plugin);
        self.finish_state_transaction();
        drop(activate_context);

        crate::nice_debug_assert!(
            success,
            "Loading plugin state failed; restored previous state"
        );

        // After a rollback too: either way the editor's view of the values may be stale.
        #[cfg(feature = "editor")]
        {
            let task_posted = self.schedule_gui(Task::StateChanged);
            crate::nice_debug_assert!(task_posted, "The task queue is full, dropping task...");
        }

        success
    }

    /// MXM PATCH (defect 8, ported to VST3): wait for the plugin's `TryLock` off the audio thread,
    /// as `setActive()` does, giving up after one second.
    fn lock_plugin_for_state_transaction(&self) -> Option<try_lock::Locked<'_, P>> {
        let started = Instant::now();
        loop {
            if let Some(plugin) = self.plugin.try_lock() {
                return Some(plugin);
            }
            if started.elapsed() > Duration::from_secs(1) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// MXM PATCH (suspended processing): a `process()` call, from its first statement to its last,
/// early returns included. Begun, it waits out an edit another thread is applying, then applies
/// the pending ones; ended, it applies those made during the call, after clearing `in_process`, so
/// an edit made as the call ends is applied by one side or the other.
pub(crate) struct InProcess<'a, P: Vst3Plugin>(&'a WrapperInner<P>);

/// How many times [`InProcess::begin`] spins on an edit another thread is applying: that is a few
/// atomic writes, so only a preempted thread outlasts it, and the call then goes on rather than
/// wait on the audio thread.
const APPLYING_SPINS: u32 = 10_000;

impl<'a, P: Vst3Plugin> InProcess<'a, P> {
    pub fn begin(inner: &'a WrapperInner<P>) -> Self {
        inner.in_process.store(true, Ordering::SeqCst);
        for _ in 0..APPLYING_SPINS {
            if inner.appliers.load(Ordering::SeqCst) == 0 {
                break;
            }
            std::hint::spin_loop();
        }
        Self::apply(inner);
        Self(inner)
    }

    fn apply(inner: &WrapperInner<P>) {
        inner.appliers.fetch_add(1, Ordering::SeqCst);
        inner.apply_pending_edits();
        inner.appliers.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<P: Vst3Plugin> Drop for InProcess<'_, P> {
    fn drop(&mut self) {
        self.0.in_process.store(false, Ordering::SeqCst);
        Self::apply(self.0);
    }
}

/// How long [`WrapperInner::settle_pending_edits`] waits for a running call or a transaction.
const SETTLE_WAIT: Duration = Duration::from_secs(2);

/// The flag marking a pending edit's slot: its value's bits below, its generation above.
const PENDING: u64 = 1 << 32;
const GENERATION_SHIFT: u32 = 33;
/// The generation bits a slot keeps; generations wrap within them.
const GENERATION_MASK: u32 = (1 << 31) - 1;

/// `value`, made in `generation`, as a pending edit's slot.
fn pending(value: f32, generation: u32) -> u64 {
    PENDING
        | (u64::from(generation & GENERATION_MASK) << GENERATION_SHIFT)
        | u64::from(value.to_bits())
}

/// A pending edit's generation.
fn generation_of(slot: u64) -> u32 {
    (slot >> GENERATION_SHIFT) as u32
}

/// A slot's pending edit, if any.
fn taken(slot: u64) -> Option<f32> {
    (slot & PENDING != 0).then(|| f32::from_bits(slot as u32))
}

impl<P: Vst3Plugin> MainThreadExecutor<Task<P>> for WrapperInner<P> {
    fn execute(&self, task: Task<P>, is_gui_thread: bool) {
        // This function is always called from the main thread
        match task {
            Task::PluginTask(task) => (self.task_executor.lock())(task),
            #[cfg(feature = "editor")]
            Task::StateChanged => {
                use nice_plug_core::editor::EditorHandle;

                if let Some(editor_window) = self.editor_window.borrow().as_ref() {
                    editor_window.get().handle.state_changed();
                }
            }
            #[cfg(feature = "editor")]
            Task::ParameterValueChanged(param_hash, normalized_value) => {
                use nice_plug_core::editor::EditorHandle;

                if let Some(editor_window) = self.editor_window.borrow().as_ref() {
                    let param_id = &self.param_id_by_hash[&param_hash];
                    editor_window
                        .get()
                        .handle
                        .param_value_changed(param_id, normalized_value);
                }
            }
            Task::TriggerRestart(flags) => match &*self.component_handler.borrow() {
                Some(handler) => unsafe {
                    crate::nice_debug_assert!(is_gui_thread);
                    let result = handler.restartComponent(flags);
                    crate::nice_debug_assert_eq!(
                        result,
                        kResultOk,
                        "Failed the restart request call with flags '{:?}'",
                        flags
                    );
                },
                None => crate::nice_debug_assert_failure!("Component handler not yet set"),
            },
            #[cfg(feature = "editor")]
            Task::RequestResize { size, scale_factor } => {
                if self.is_editor_open.load(Ordering::SeqCst) {
                    unsafe {
                        crate::nice_debug_assert!(is_gui_thread);
                        let _success = WrapperView::request_resize(
                            &self.plug_view.read().clone().unwrap(),
                            size,
                            scale_factor,
                        );
                    }
                } else {
                    crate::nice_debug_assert_failure!("Can't resize a closed editor");
                }
            }
            #[cfg(feature = "editor")]
            Task::CallMainThread => {
                use nice_plug_core::editor::EditorHandle;

                if let Some(editor_window) = self.editor_window.borrow().as_ref() {
                    let editor_window = editor_window.get();
                    editor_window
                        .handle
                        .host_main_thread_callback(&editor_window.window)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A pending edit is its value, taken once** (suspended processing): every value, 0.0 and
    /// the extremes included, round-trips, and an empty slot holds none.
    #[test]
    fn a_pending_edit_is_its_value() {
        for value in [0.0f32, 0.25, 1.0, -0.0, f32::MIN_POSITIVE] {
            for generation in [0, 7, GENERATION_MASK, u32::MAX] {
                let slot = pending(value, generation);
                assert_eq!(taken(slot).map(f32::to_bits), Some(value.to_bits()));
                assert_eq!(generation_of(slot), generation & GENERATION_MASK);
            }
        }
        assert_eq!(taken(0), None);
    }
}
