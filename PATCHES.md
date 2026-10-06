# Patched `nice-plug`

**Since 2026-10-06 this is `nice-plug 0.4.2` exactly as published on crates.io**, plus the MXM
patches that 0.4.2 has not made redundant: seven of the eleven numbered fixes (1, 2, 7, 8, 9, 10 and
11; 1, 2 and 8 reworked for 0.4.2) and all three capability changes. Fixes 3, 4, 5 and 6 were
dropped because 0.4.2 does the same thing. The numbering is unchanged, so every older reference
still resolves. See [*Refresh to 0.4.2*](#refresh-to-042--2026-10-06) for the per-patch status.

*Before the refresh (superseded):* This is **`nice-plug 0.3.0` exactly as published on crates.io**,
plus eleven fixes and three capability changes. It is wired in through `[patch.crates-io]` in the
root `Cargo.toml`.

Every change is marked `MXM PATCH` in the source, so `grep -rn "MXM PATCH" src/` finds the complete
delta. Since the 0.4.2 refresh it lives in `src/wrapper/clap/wrapper.rs`, `src/wrapper/state.rs`
and the one re-export in `src/wrapper.rs`. Hunks reworked by the refresh say
`refreshed onto 0.4.2` in their marker. *(0.3.0, superseded: it lived in
`src/wrapper/clap/wrapper.rs` and `src/wrapper/state.rs`, plus the output-push guard in
`src/wrapper/clap/context.rs`. 0.4 removed the wrapper's output queue that the guard bounded.)*

**The last three are different in kind from the eleven** — they are not defect fixes. See
*GUI-authored state dirty*, *Floating windows* and *GUI-task process wake*, below.

## Refresh to 0.4.2 — 2026-10-06

**What.** `nice-plug` 0.3.0 → 0.4.2, which pairs with `nice-plug-core` 0.4.2 and `nice-plug-egui`
0.5.1. Branch `refresh-0.4.2` starts from the licence commit `11a1176`. One commit imports the
published crate, `nice-plug-0.4.2.crate`, sha256
`3f47be74bed3e8aa17bb055d4ff89af1fb016aa0d2756cd9b6c029a3743676a5` (the crates.io index checksum),
from upstream `91ba75d20c43ca0cd21afa802d0ae8c0511f48ed`, `crates/nice-plug`. A second commit
re-applies these patches: it cherry-picks `ba34543` and resolves it against 0.4.2. `main` still
holds the 0.3.0 copy.

**Why.** On macOS, plugin editors embedded in a DAW (Bitwig) scaled instead of reflowing and came
out twice too large. 0.3.0's CLAP wrapper passes physical pixels where CLAP's Cocoa API uses logical
points. Upstream 0.4.2 fixes this with `NativeSize`: physical pixels on Windows and Linux, logical
points on macOS, across `nice-plug` 0.4.2, `nice-plug-core` 0.4.2 and `nice-plug-egui` 0.5.1. The
owner chose to refresh onto 0.4.2 rather than patch 0.3.0. **No MXM size conversion is added;
upstream's `NativeSize` is the fix.** The floating `request_resize` early return still holds, and
embedded editors forward the request in `NativeSize` units exactly as upstream does.

**Status of every patch.** *Ported* means re-applied with the same logic, possibly at a moved
location. *Adapted* means upstream changed the code underneath, so the hunk was reworked. *Dropped*
means 0.4.2 already does the same thing, so its version stands. Upstream commits are on
<https://codeberg.org/RustAudio/nice-plug>.

| Patch | Status | Why |
|---|---|---|
| 1 — event capacity | **adapted** | **Input, kept.** 0.4 sizes the queue from the new `Plugin::INPUT_EVENT_CAPACITY` (default 1024) and, when that fills, grows it under `permit_alloc` with a warning (`a02ceac4bb`, the fix for #87). It accepts the allocation. Our hard bound, prefix/suffix inspection and O(1) newest-termination admission stay: upstream's `push_event` closure in `handle_input_event` now calls `push_input_event()`. `INPUT_EVENT_CAPACITY` is honoured as a floor beside our sizing, so a plugin that keeps the default gets at least 1,024 instead of 512. **Plugin output, dropped.** 0.4 removed the wrapper's output queue: `ProcessContext::try_send_event()` pushes straight into the host's `out_events` and reports `SendEventError::HostBufferFull`, so the `context.rs` guard has nothing left to bound. **GUI-authored parameter output, kept.** That queue is still upstream's fixed 2,048, and our inventory-scaled capacity stays. |
| 2 — state length | **adapted** | Upstream added `MAX_STATE_BYTES` (256 MiB) and `try_reserve_exact` to `ext_state_load` (`19cad8147d`, the fix for #85). Our `MAX_STATE_SIZE` (512 MiB) is dropped in its favour, **so the cap halves**. Kept: `read_declared_state()`, which reads exactly the declared span and refuses a short stream. Upstream reads into the whole spare capacity and parses whatever prefix arrived; `read_stream` now returns `Option<usize>` and stops at end of stream. A new unit test covers the short stream. |
| 3 — rescan after state load | **dropped** | Upstream's `Task::StateChanged` now calls the host's `rescan(CLAP_PARAM_RESCAN_VALUES)` and is no longer editor-only (`f5235690`, the #63 follow-up). Upstream schedules it only after a successful deserialization. Fix 8's transaction schedules it after every completed load, including a rollback, so fix 8's hunk now carries the rollback case. |
| 4 — non-finite host value | **dropped** | `update_plain_value_by_hash` returns early on a non-finite value in both arms (`0899f50062`, the fix for #86). It returns `false` where ours returned `true`, but all three callers ignore the result. |
| 5 — in-place pairing | **dropped** | `ext_audio_ports_get` pairs only when `main_input_channels == main_output_channels` (`can_process_in_place`, `f4afbb7581`, the fix for #84). |
| 6 — first split event | **dropped** | Upstream rewrote `handle_in_events_until` to check every event, the first included, before applying it. Never reported; fixed upstream independently. |
| 7 — split timestamp clamp | **ported** | Upstream still returns the raw timestamp as the split point and still subtracts before clamping in `handle_input_event`. Re-seated in upstream's rewritten split loop, which also skips null events. |
| 8 — GUI state restore | **adapted** | Upstream's plugin lock is now a non-blocking `TryLock`, and upstream no longer reactivates after a state load. See *Defect 8 on 0.4.2* below. |
| 9 — rejected persistent fields | **ported** | `state.rs` applied cleanly. |
| 10 — host tail notification | **ported** | Unchanged. |
| 11 — canonicalization acceptance | **ported** | Unchanged in substance. The `thread_local` initializer is now `const` and the test module sits after the last item, both for Clippy 1.98. |
| GUI-authored state dirty | **ported** | Unchanged. Its parking_lot rationale is annotated for the `TryLock`. |
| GUI-task process wake | **ported** | Unchanged. |
| Floating windows | **ported** | 0.4.2 still refuses `is_floating` in `ext_gui_is_api_supported`. #88 was closed after the maintainer explained that VST3 cannot host plugin-owned windows, so upstream support would have to exclude the `vst3` feature. `request_resize` keeps the floating early return; embedded editors forward `NativeSize` as upstream does. |

**Defect 8 on 0.4.2.** Upstream 0.4 changed two things underneath it:

- The plugin lock is a non-blocking `try_lock::TryLock`. `process()` returns `CLAP_PROCESS_ERROR`
  and logs with `nice_error!` when it cannot take the lock. `activate`, `deactivate`,
  `start_processing`, `stop_processing` and `reset` poll for it with a timeout.
- `set_state_inner()` no longer reactivates or resets the plugin after a state load, in either the
  CLAP or VST3 wrapper; the standalone wrapper still does. Upstream's own `Plugin::activate()` docs
  still describe reactivation.

The hunk keeps the 0.3.0 behaviour MXM plugins were built against: snapshot, deserialize, reactivate
and reset under the lock, with a complete rollback (without `reset()`) on failure. It polls for the
lock off the audio thread for up to one second, as upstream's `activate()` does. A timeout refuses
the load without mutating anything.

While the transaction holds the lock, a concurrent `process()` skips the rest of its buffer: it
writes silence from that point to every output channel (`silence_outputs_from`) and returns
`CLAP_PROCESS_CONTINUE`. Under 0.3.0 the mutex made it block until the transaction finished
instead. A new `state_transaction_active` flag tells this skip from upstream's misbehaving-host case
(which still logs and returns `CLAP_PROCESS_ERROR`) and keeps it silent, because `nice_error!` is not
allocation-permitted on the audio thread and a log line there could abort a debug
`assert_process_allocs` build. The load resets the plugin, so a note-off skipped here cannot leave a
note hanging. Regression: `mxm_state_tests::a_block_skipped_during_a_state_load_is_silence_from_the_skip_onwards`.

*First version of this refresh (same day), superseded:* the skip returned `CLAP_PROCESS_ERROR`.
Porting MXM Player showed what that does to a host: the player marks the plugin failed
(`RunState::Failed`) and keeps it silent until a reset, and a DAW may react the same way — when the
plugin is only loading a preset, which the editors do during playback.

The audio-thread GUI-state handoff, which 0.4.2 still has, is removed as before. That leaves two
pieces of upstream code to annotate rather than delete: `Task::RescanParamValues` has no sender any
more and carries `#[allow(dead_code)]`, and the `let result = loop` binding in `process()` carries
`#[allow(clippy::let_and_return)]`. The two guarded reactivations are now `let` chains, because 0.4.2
declares Rust 1.88 and Clippy's `collapsible_if` asks for them.

**Consequences for consumers.** These were not checked here; check them when the plugins and the
player are rebuilt.

- **Plugin code.** 0.4.2's public API breaks 0.3.0 plugin code:
  - `ProcessContext::send_event` → `try_send_event`
  - `NoteEvent`'s `note: u8`, `channel: u8` and `voice_id: Option<i32>` → `key: Key`,
    `channel: Channel` and `voice_id: VoiceID`
  - `Plugin::track_info_updated` → `Editor::track_info_updated`
  - `TrackInfo` and `TrackColor` now require the `editor` feature
  - `SysExMessage::to_buffer` → `as_buffer`
  - `PhysicalSize` → `NativeSize` in custom `Editor`/`EditorHandle` implementations
  - the MSRV is now Rust 1.88
- **Output fixture.** The nice-plug output fixture emits through `send_event` and expects exactly 512
  events through the wrapper's bounded output queue. That queue no longer exists, so the fixture and
  its expectation must be rewritten against `try_send_event`.
- **Input overflow regression.** The debug regression ("64 frames, configured limit 512") now gets a
  limit of at least 1,024 from `INPUT_EVENT_CAPACITY`. Its 2,001 events no longer exceed twice the
  limit, so it stops exercising the skipped middle. To restore that, send more than twice the limit
  or declare `INPUT_EVENT_CAPACITY = 512` in the test plugin.
- **State load during processing.** This now silences this plugin's blocks for the length of the
  transaction instead of stalling the audio thread, and the host carries on. A regression that
  loads state while processing concurrently and compares tails bit for bit will see that.
  *Checked when the player was ported (2026-10-06):* no player test does; two load state while the
  fake backend streams and check only the command's answer.
- **State cap.** The cap is 256 MiB instead of 512 MiB.

**Checks run for this refresh** (2026-10-06, Windows, Rust 1.98.0, this crate only):

- `cargo check --locked` and `cargo check --locked --no-default-features`: clean.
- `cargo clippy --locked --all-targets -- -D warnings`: only upstream's two `unnecessary_cast` hits
  in `src/wrapper/vst3/util.rs`'s own `#[cfg(test)] mod miri`, which our patches do not touch. With
  just that lint allowed, every target is clean.
- `cargo clippy --locked --all-targets --no-default-features -- -D warnings`: only upstream's
  `needless_return` at `src/wrapper/util.rs:118`, in a file our patches do not touch. With just that
  lint allowed, every target is clean.
- `cargo test --locked --lib --no-default-features -- mxm_state_tests canonicalized_field`: 11
  passed.
- `rustfmt --edition 2024 --check` passes on the patched `src/wrapper.rs`,
  `src/wrapper/clap/wrapper.rs` and `src/wrapper/state.rs`, as it does on 0.4.2's published
  versions of the files the patches touch. The 0.3.0 patch left 17 rustfmt differences in
  `wrapper.rs`; its hunks were reformatted, with whitespace changes only.

## Why this exists

The first three defects surfaced when the collection's own host (`apps/mxm-player`) drove
mxm-mono-01 with an allocation-counting audio callback and a `clap-validator` run; later instrument
reviews found the later eight — the fifth with `plugins/mxm-chorus-06`, **6 and 7 during
`mxm-para-07`'s factory review**, and **8 through 11 during `mxm-fx-convolution`'s**. **None is
fixable from plugin code** — the affected buffers, state loader, GUI state restoration, host-value
application, port declarations, automation splitting, persistent-field transaction protocol and
host-tail notifications belong to the wrapper. All eleven are present in the pinned published
release. At the original investigation the first three were still present on upstream `main`
(`542daf1`, 2026-08-21). *(0.4.2 refresh, 2026-10-06: "the pinned published release" was 0.3.0. In
0.4.2, defects 3, 4, 5 and 6 are fixed upstream, defects 1 and 2 are partly fixed, and 7 to 11 are
still present. See the status table above.)*

They are diagnosed in full, with the bisection, in [`../../docs/known-issues.md`](../../docs/known-issues.md).

| # | Defect | Effect before the patch |
|---|---|---|
| 1 | `input_events` / `output_events` are `VecDeque::with_capacity(512)` and pushed without a bound, so the 513th event reallocates; traversing every host-reported input event also leaves callback work host-count-controlled; the overflow classifier also treats a zero-velocity input NoteOn as ordinary even though collection instruments interpret it as NoteOff; separately, GUI-authored parameter output has a fixed 2,048-event queue regardless of parameter inventory | Debug: abort on process-event growth. Release: a **silent heap allocation on the audio thread** plus unbounded callback work; under bounded overflow, a dropped zero-velocity NoteOn can leave an earlier note held indefinitely; a large GUI preset silently loses its suffix—mxm-drum-machine changed only its first four slots. The fix preallocates, bounds raw inspection and queue storage separately, sizes complete host and GUI patch bursts from parameter inventory, drops the hostile middle and excess ordinary events, and admits newest input/output releases plus zero-velocity input NoteOns in O(1) |
| 2 | The CLAP state loader passes an unbounded 8-byte length read from the stream straight to `Vec::with_capacity` | A corrupt or truncated preset **aborts the process**, taking the host with it |
| 3 | `set_state_inner()` notifies only the editor, never `params.rescan(VALUES)` | Every host but ours shows **stale parameter values** after loading state |
| 4 | `update_plain_value_by_hash` applies a host's `CLAP_EVENT_PARAM_VALUE` and `PARAM_MOD` without checking the value is finite; `f32::clamp` passes NaN, so it reaches the smoother and the plugin | One NaN from a host **silences the instrument for good** — `NaN * 0` is `NaN` in every mix |
| 5 | `ext_audio_ports_get` declares the main input and main output an in-place pair whenever both exist, whatever their channel counts; CLAP allows a pair only between ports of the same shape | A mono-in, stereo-out effect — any effect that makes its own stereo — advertises a pair that cannot exist, and `clap-validator` **refuses to process it at all** (`process-audio-basic-in-place`, `layout-audio-ports-config`) |
| 6 | `handle_in_events_until` applies the first unread parameter event before checking whether its timestamp is after the current split point | The first automation event in a block moves to sample zero even with `SAMPLE_ACCURATE_AUTOMATION`; an edge-derived trigger fires early |
| 7 | `handle_in_events_until` compares and returns an unclamped raw timestamp even though `handle_in_event` clamps it later | An out-of-range parameter or transport event makes `block_end` exceed `frames_count`, so `BufferManager` constructs audio slices beyond the host buffers |
| 8 | GUI `set_state()` sends the object to the audio callback, where persistent fields and reactivation may allocate, then mutates fields/parameters before knowing activation succeeds | Asset state can allocate on audio and an activation failure leaves a mixed old/new working state |
| 9 | `Params::deserialize_fields()` cannot report that a `PersistentField` rejected malformed or unpreparable content | The wrapper reports success and keeps incoming parameters around an old durable field |
| 10 | The CLAP wrapper exposes `clap_plugin_tail` but never acquires `clap_host_tail` or calls `changed()` when `ProcessStatus` crosses the finite/infinite boundary | A host may cache a finite tail after Feedback becomes sustaining, or keep an infinite tail after it becomes finite, and sleep or retain the effect incorrectly |
| 11 | Defect 9's generic round-trip check cannot distinguish a rejected persistent field from one that accepted input and installed a canonical representation | A legitimate visible clamp is rolled back unless the plugin temporarily saves the illegal request, making durable state disagree with the panel and prepared audio until activation |

The fourth was found later than the first three, by mxm-mono-00's code review (round 4, 2026-09-03):
its DSP contract says host input is untrusted and every range is bounded in the DSP too, and a
reviewer reading the code noticed that a NaN is not bounded by a clamp. It is a wrapper defect, not
an instrument's — the parameter types are not vendored, and the wrapper is the one place every
host value passes — so it is fixed here, once, for every instrument in the collection.

## The fixes

**1 — preallocate and hard-bound event storage and inspection.** Follows the wrapper's own existing
idiom: it already preallocates `buffer_manager` in `activate()`, on the main thread. Queue storage is
likewise reserved there, while `event_queue_limit` is enforced at every input and output push. The
process/flush capacity adds one simultaneous value event per exposed parameter to the frame-scaled
event budget, because CLAP permits a complete patch at one sample. That parameter budget is present
from wrapper construction, since `params.flush()` is valid before activation; activation adds the
frame budget. The separate GUI-authored parameter-output queue keeps its original 2,048-event
live-edit reserve and adds three events per exposed parameter, because a complete editor preset is a
begin/value/end gesture for each. Without both sides, a large inventory passes only a prefix through
one path while the other applies all of it; with the fixed 2,048-event GUI queue,
mxm-drum-machine's then-3,210-parameter factory preset changed only about 682 parameters—the first four
slots—and silently left the rest of the kit behind. CLAP does not bound event
count by frame count, so this remains a capacity policy, never the realtime argument. A host list
above twice capacity is reduced to one capacity-sized prefix and
suffix: the prefix preserves initial state, the suffix preserves newest automation and termination,
and the middle is skipped in O(1). Split processing may fetch a selected boundary twice, so raw host
`get()` calls are bounded at four times capacity independently of the reported count. Excess ordinary
input and output are dropped newest-first; an incoming note release, choke, All Sound Off or All
Notes Off replaces the oldest queued event in O(1), so newest termination is admitted without a
queue scan, shift or growth. The input-only classifier also admits a zero-velocity NoteOn because
instrument handlers interpret it as NoteOff; output classification remains explicit. The first debug
input regression activates for 64 frames (configured limit 512), establishes an audible note in an
earlier callback, then sends 2,000 ordinary events followed by its explicit release; after the real
release tail, a later callback is exactly silent under `assert_process_allocs`. The companion drives
mxm-para-07 through the raw offline CLAP list, establishes its note in the prior callback, then ends
the saturated callback with a zero-velocity NoteOn carrying the same note id and proves exact inert
silence after the tail. A vendored unit regression proves that a 1,000,000-event report selects
exactly the first and last 512 indices. A separate nice-plug test plugin emits 1000 output note-ons
through `ProcessContext::send_event`, then
terminates a distinct note admitted before saturation; exactly 512 events reach the host, including
that note-on and its post-saturation note-off, and removing the output guard aborts on the 513th push.

*0.4.2 refresh, adapted.* The input side is ported. Upstream's new `push_event` closure in
`handle_input_event` routes through `push_input_event()`, and the bounded iterator now drives
upstream's rewritten split loop. `Plugin::INPUT_EVENT_CAPACITY` (new in 0.4, default 1024) is a
floor beside this sizing, so the minimum limit for a default plugin is 1,024 rather than 512.
Activation reserves the queue inside upstream's new `try_borrow_mut` retry loop.

The output side is superseded. 0.4 has no wrapper output queue:
`ProcessContext::try_send_event()` pushes directly into the host's `out_events` and returns
`SendEventError::HostBufferFull` when the host's list is full. So the `context.rs` guard, the
output reservation and the output drop counter are gone, and the output-fixture regression above
needs rewriting for `try_send_event`. The GUI-authored parameter-output capacity is ported
unchanged.

Upstream's own answer to #87 (`a02ceac4bb`) is to accept an allocation when a host sends more
than `INPUT_EVENT_CAPACITY` events, so this bound stays a local policy.

**2 — bound the declared state size and reserve fallibly.** Lengths above `MAX_STATE_SIZE` (512
MiB), lengths not representable as `usize`, reservation failures and short streams are refused with
`false`. The stream read is sliced to exactly the declared payload even if `Vec` reserves more.
Rejecting a bad preset is recoverable; aborting or consuming a following stream item is not. Direct
unit regressions force a 512 MiB reservation to fail under a test allocator and put trailing bytes
behind a short declared payload; the player regression retains the real hostile-length host path.

*0.4.2 refresh, adapted.* Upstream now has the bound and the fallible reservation
(`MAX_STATE_BYTES`, 256 MiB, then `try_reserve_exact`, in `ext_state_load`). Ours give way to them:
`MAX_STATE_SIZE` (512 MiB) is gone, so the cap is now 256 MiB. The reservation regression now
reserves `MAX_STATE_BYTES`, still at or above the test allocator's 256 MiB refusal threshold. The
`usize` conversion is upstream's `as usize`, which the bound makes lossless on every CLAP target.

`read_declared_state()` is kept, because upstream reads into the vector's whole spare capacity and
then `set_len`s whatever arrived. 0.4's `read_stream` returns `Some(bytes_read)` and stops at end of
stream, so upstream parses a truncated payload as a prefix instead of refusing it. The helper now
requires `read_stream(..) == Some(length)`. The new regression
`state_reader_refuses_a_stream_shorter_than_the_declared_payload` covers that case.

**4 — drop a non-finite host value at the event.** Both arms of `update_plain_value_by_hash`
return early when the value is not finite: the event is consumed (the hash was known) and the
parameter keeps its value. Nothing else in the path checks — the range's `normalize` clamps with
`f32::clamp`, which returns NaN for NaN. Regression test:
`plugin_robustness.rs::a_non_finite_parameter_value_from_the_host_is_dropped`, through the
player's own host path: a NaN cutoff arrives while a note sounds, the audio stays finite, the note
keeps sounding, and the cutoff reads what it was.

*0.4.2 refresh, dropped.* Upstream's `update_plain_value_by_hash` has the same early return in both
arms (`0899f50062`, the fix for #86). It returns `false` where this returned `true`; all three
callers ignore the result.

Neither version guards the poly-modulation paths in `handle_input_event`. There, a non-finite
`CLAP_EVENT_PARAM_VALUE` for a poly-modulated parameter still produces a `MonoAutomation` event,
and a non-finite poly `CLAP_EVENT_PARAM_MOD` still produces a `PolyModulation` event. This was
already true under 0.3.0 with this patch; it is not a refresh regression. Keep the player
regression.

**5 — pair the main ports in place only when they are the same shape.** `pair_stable_id` now
requires `main_input_channels == main_output_channels`; otherwise both ports report
`CLAP_INVALID_ID` and the host uses separate buffers, which the wrapper already handles by copying
the input over the output. Found by `plugins/mxm-chorus-06`, the collection's first effect, whose
faithful layout is mono in, stereo out (2026-09-04). Same-shape layouts pair exactly as before.
Regression test: `clap-validator` on `mxm-chorus-06` in both profiles — the two tests named above
fail without this hunk and pass with it; the instruments, all output-only, never reach it.

*0.4.2 refresh, dropped.* Upstream's `ext_audio_ports_get` makes the identical check
(`can_process_in_place`; `f4afbb7581`, the fix for #84). Keep the `clap-validator` regression.

**3 — schedule `Task::RescanParamValues` in `set_state_inner()`.** Every completed state transaction,
including rollback, invalidates the host's parameter cache.

*0.4.2 refresh, dropped.* Upstream's `Task::StateChanged` now also calls
`rescan(CLAP_PARAM_RESCAN_VALUES)` and is no longer editor-gated (`f5235690`, the #63 follow-up).
Upstream schedules it only after a successful deserialization: on a failed one it returns early, and
the host keeps a stale cache of whatever the partial deserialization changed. Defect 8's adapted
`set_state_inner()` schedules `StateChanged` after every transaction that took the lock, success or
rollback, so the guarantee above still holds. It is now carried by defect 8's hunk.

`Task::RescanParamValues` has no sender left and is kept with `#[allow(dead_code)]`. Its only
sender in 0.4.2 was the audio-thread handoff path in `set_state_object_from_gui()`, which defect 8
removes.

**6 — check the first unread event before applying it.** The wrapper's later events already receive
this look-ahead check. Applying the same predicate to the event at `resume_from_event_idx` splits the
leading audio span before changing the parameter. Regression test:
`mxm_para_07_behaviour.rs::trigger_parameter_edges_are_sample_accurate_and_restore_only_a_level`,
through the bundled CLAP shell with a low-high-low pulse at nonzero offsets.

*0.4.2 refresh, dropped.* Upstream rewrote `handle_in_events_until` without the read-ahead. It
checks each event, the first included, before applying it ("Check the current event before applying
it, including the first event in the buffer"). This was never reported upstream. Defects 1 and 7
still modify the same loop. Keep the regression.

**7 — clamp absolute event time before splitting.** A later clamp during event conversion cannot
make an already selected audio segment safe. Split selection now compares and returns the timestamp
clamped to the complete host buffer, and segment-relative timing is derived from it with saturating
subtraction. Regression test:
`mxm_state_tests::out_of_range_split_event_is_clamped_before_buffer_partitioning` drives the exact
timing primitive with an event beyond a 64-frame buffer, bounds the split at sample 63 and proves
that processing resumed there consumes the event at relative sample zero.

*0.4.2 refresh, ported.* 0.4.2 still returns the raw `(*event).time` as the split point and still
computes `raw_event.time - current_sample_idx` before clamping. The clamp is re-seated in upstream's
rewritten split loop, after upstream's new null-event check.

**8 — restore GUI state under the plugin lock on the GUI thread and snapshot before mutation.** The
zero-capacity audio-thread handoff is removed. Deserialization, persistent-field preparation and
reactivation complete while one lock excludes processing. Failure deserializes and reactivates the
snapshot before releasing that lock, but deliberately does not call `reset()`: a rejected transaction
must preserve live processor history as well as durable state. Its two guarded reactivations are
nested `if`s, not `let` chains, because this crate declares Rust 1.87 and the nice-plug fixture
builds at that floor (`docs/known-issues.md`, *Two crates declared an MSRV they could not build on*).

*0.4.2 refresh, adapted.* See *Defect 8 on 0.4.2* above. In short:

- The plugin lock is now upstream's non-blocking `TryLock`. It is polled for off the audio thread for
  up to one second, and a timeout refuses the load before anything is mutated.
- Upstream dropped post-load reactivation; it is kept here.
- A concurrent `process()` writes silence for the rest of its buffer and returns
  `CLAP_PROCESS_CONTINUE` instead of blocking. The `state_transaction_active` flag keeps that path
  from logging on the audio thread. (The refresh's first version returned `CLAP_PROCESS_ERROR`
  there, which MXM Player treats as a failed plugin; see *Defect 8 on 0.4.2*.)
- `StateChanged`, which now also triggers the host rescan, follows every transaction that took the
  lock.
- The nested-`if` note above is superseded: 0.4.2 declares Rust 1.88, so the reactivations are
  `let` chains, as Clippy's `collapsible_if` asks.

**9 — detect rejected persistent fields.** After `deserialize_fields()`, supplied fields must
serialize to the same JSON value (formatting and object order may differ). A mismatch returns false,
feeding defect 8's complete rollback rather than silently accepting a partial state. Regression:
`apps/mxm-player/tests/mxm_fx_convolution_behaviour.rs::active_reactivation_failure_rolls_back_response_parameters_engine_and_live_tail`
loads a valid but deadline-impossible embedded response into one of two matched active 384 kHz
instances while also changing parameters, requires the CLAP load to fail and prior state bytes to
return, then requires its still-nonzero tail to remain bit-identical to the untouched instance.

*0.4.2 refresh, ported.* `state.rs` applied cleanly. 0.4.2's `deserialize_object` now warns about
unknown parameters and enum IDs instead of debug-asserting, and it still ends in the field check.
The fix's verdict also reaches the VST3 wrapper's `set_state_inner`, as it did on 0.3.0.

**10 — notify finite/infinite tail-class changes.** Initialization caches only the host extension's
`changed` function pointer. Each process publication atomically stores the new `ProcessStatus`, then
calls that pointer only when `KeepAlive` membership changes; an immediate host re-query therefore
observes the new class. The audio-thread path takes no lock and allocates nothing. The focused fake
host regression checks both edge-only delivery and store-before-callback ordering.

*0.4.2 refresh, ported.* 0.4.2 still never acquires `clap_host_tail`. Unchanged.

**11 — distinguish accepted canonicalization from persistent-field rejection.** The wrapper opens a
same-thread acknowledgement scope only while `Params::deserialize_fields()` runs. After a field has
accepted and installed a canonical value, it may acknowledge its exact stable persisted key through
`accept_canonicalized_persistent_field`; defect 9 then permits a mismatch only for that key. Marks
outside restoration are discarded, scopes clear on unwind, and an unmarked mismatch still returns
false and enters defect 8's complete rollback. This is deliberately not a blanket mismatch allowance.
`over_budget_state_is_accepted_with_the_applied_size_reading_back_at_the_exact_boundary` loads and
immediately re-saves without activation, requiring the applied maximum; the existing active deadline
rejection test still requires byte-identical parameters and fields plus unchanged live history.
Defect 11 has not been reported upstream.

*0.4.2 refresh, ported.* Unchanged in substance. For Clippy 1.98 the `thread_local!` initializer is
now `const { … }` (`missing_const_for_thread_local`), and `state.rs`'s test module moved below
`deserialize_json` (`items_after_test_module`).

## What it buys

`clap-validator` on mxm-mono-01, before and after, **debug** bundle (the allocation guard only fires in
debug, so this is the run that matters):

| | Before | After |
|---|---|---|
| passed | 30 | **35** |
| failed | 5 | **0** |
| crashed | 2 (`0xc0000409`) | 0 |

*(Measured on 0.3.0 with all eleven fixes. Not re-measured after the 0.4.2 refresh; that happens
when the plugins are rebuilt.)*

## Upstream status — 2026-10-06 (0.4.2)

Checked against the published 0.4.2 code (upstream `91ba75d2`) and the Codeberg issues, which are
all closed:

| Local patch | Report | Upstream outcome | In 0.4.2 |
|---|---|---|---|
| 1 — event capacity | [#87](https://codeberg.org/RustAudio/nice-plug/issues/87), closed 2026-09-09 | `a02ceac4bb`. The maintainer did not want large preallocated buffers, so the input queue now starts at `Plugin::INPUT_EVENT_CAPACITY` and accepts an allocation when a host exceeds it. Plugin output was made direct and fallible (`try_send_event`). | **Partly.** The output side is gone; the input bound is still ours (adapted). |
| 2 — state length | [#85](https://codeberg.org/RustAudio/nice-plug/issues/85), closed 2026-09-07 | `19cad8147d`: `MAX_STATE_BYTES` (256 MiB) plus `try_reserve_exact`. | **Partly.** The exact-span read and short-stream refusal are still ours (adapted). |
| 3 — state rescan | [#63 follow-up](https://codeberg.org/RustAudio/nice-plug/issues/63#issuecomment-22428703), #63 closed | `f5235690`: `Task::StateChanged` rescans the host's values. | **Yes**, except after a failed deserialization, which defect 8's hunk covers. Dropped. |
| 4 — non-finite parameters | [#86](https://codeberg.org/RustAudio/nice-plug/issues/86), closed 2026-09-08 | `0899f50062`. | **Yes.** Dropped. |
| 5 — in-place pairing | [#84](https://codeberg.org/RustAudio/nice-plug/issues/84), closed 2026-09-07 | `f4afbb7581`. | **Yes.** Dropped. |
| Floating editors | [#88](https://codeberg.org/RustAudio/nice-plug/issues/88), closed 2026-09-07 by `maxmcorp` | The maintainer said VST3 does not support plugin-owned windows, so the feature would have to exclude the `vst3` feature. | **No.** `ext_gui_is_api_supported` still refuses `is_floating`. Ported. |

Defect 6 was never reported but is fixed in 0.4.2 by upstream's rewritten split loop, so it is
dropped. **Defects 7 to 11 are still not reported and still present in 0.4.2**, and so are the
GUI-authored state-dirty and GUI-task process-wake gaps. Upstream also dropped post-load
reactivation, which defect 8 now preserves locally. Report that difference together with defect 8.
The commit hashes above are the ones the maintainer cited in each issue; they were not checked
against upstream's history here.

## Upstream status — 2026-09-06

*(Superseded by the 2026-10-06 status above; kept as the record of what was submitted.)*

Checked current `main`, `13e2473cfdfea6c0b9e1955f8bf677227efc7d7c`, plus all listed open/closed
issue and PR titles/bodies. All five bug paths checked then, and the floating refusal, remain in the inspected code.
Those five defects have now also been reproduced with upstream-side CLAP tests.

**Defects 6 to 11 are not in this submission and have not been reported.** They were found later —
6 and 7 during `mxm-para-07`'s factory review, and 8 through 11 during `mxm-fx-convolution`'s.
Defect 10 was searched separately on 2026-09-16 across current upstream `main` and open/closed issue
and pull-request text; the wrapper still lacked host-tail notification and no matching report was
found. Report these defects before treating the upstream picture below as complete.

**The rescan defect already has a report:** [#63](https://codeberg.org/RustAudio/nice-plug/issues/63),
fixed in [#64](https://codeberg.org/RustAudio/nice-plug/pulls/64). The `set_state_inner` rescan was
removed again in merge [#68](https://codeberg.org/RustAudio/nice-plug/pulls/68), commit `4450436d`.
The follow-up is linked below. No matching report was found for the other five topics before posting;
that search does not cover private reports or every unrelated comment.

Six focused patch candidates, tests and submission texts are in
[`../../plans/upstream-nice-plug/`](../../plans/upstream-nice-plug/README.md). Each candidate was built
and tested independently in a scratch clone. The combined package passes **35 tests**, including
**16 new regressions**, headless, with editor support and the debug allocation guard, and in release
on Windows. Removing the fixes reproduces **ten failing checks**. Strict Clippy has an existing
`needless_return` warning on unpatched upstream; it passes with only that lint allowed.

**Submitted as `maxmcorp` on 2026-09-06**, with the owner's approval and concise issue-tracker wording:

| Local patch | Upstream report |
|---|---|
| 1 — event capacity | [#87](https://codeberg.org/RustAudio/nice-plug/issues/87) |
| 2 — state length | [#85](https://codeberg.org/RustAudio/nice-plug/issues/85) |
| 3 — state rescan | [#63 follow-up](https://codeberg.org/RustAudio/nice-plug/issues/63#issuecomment-22428703) |
| 4 — non-finite parameters | [#86](https://codeberg.org/RustAudio/nice-plug/issues/86) |
| 5 — in-place pairing | [#84](https://codeberg.org/RustAudio/nice-plug/issues/84) |
| Floating editors | [#88](https://codeberg.org/RustAudio/nice-plug/issues/88) |

Each report carries its matching patch. Public text, author and downloaded attachment hashes were
verified; [`published.json`](../../plans/upstream-nice-plug/published.json) records them. These are
reports with patch attachments, not PRs. No forks or remote branches were created. Await maintainer
feedback, particularly on event overflow, state-size policy and floating-window scope. No acceptance
or merge is claimed; the local commits record AI assistance.

Event capacity remains a mitigation, the 512 MiB state cap remains a local policy rather than complete
OOM protection, and floating support is a separate feature proposal. Its code builds, but no fresh
native GUI, validator or Linux/macOS check was performed. Do not infer that any of these explains
the player's still-unlocated stop timeout. No source in this vendored copy was changed.

## Refreshing this copy

1. Since this copy became a git fork, as done for 0.4.2 on 2026-10-06:
   - Branch from the licence commit (`11a1176`).
   - Verify the new `.crate`'s sha256 against the crates.io index `cksum`
     (`https://index.crates.io/ni/ce/nice-plug`).
   - Commit the published files over the branch, keeping `LICENSE` and leaving `PATCHES.md` out of
     that commit. Record the checksum and upstream commit (`.cargo_vcs_info.json`) in the message.
   - Cherry-pick the previous patch commit and resolve it.

   *(Before the fork, superseded: Extract the new `nice-plug` release over this directory, keeping
   `PATCHES.md`.)*
2. Re-apply the hunks still carried:
   - the **seven** defect hunks (1, 2, 7, 8, 9, 10 and 11)
   - the GUI-authored state-dirty hunk
   - the floating-window hunks
   - the GUI-task process-wake hunk

   `grep -rn "MXM PATCH"` on the *old* copy shows exactly what they were: 47 markers after the 0.4.2
   refresh, 39 before it. For each hunk, first check whether upstream now does the same thing,
   including the edge cases its paragraph names. That check dropped 3, 4, 5 and 6 in the 0.4.2
   refresh; their paragraphs say where upstream does it. If unsure, keep the hunk and say so.

   - **Defect 1** lives in `wrapper.rs` only since 0.4.2. It includes activation-time reservation,
     bounded prefix/suffix input inspection, guarded pushes through upstream's `push_event`
     closure, O(1) release replacement and the inventory-scaled GUI parameter-output queue.
     Reservation or bounded storage alone is not the fix.
   - **Defect 2** is the exact-span, short-stream-refusing read only. The bound and the fallible
     reservation are upstream's.
   - **Defect 7** clamps absolute timestamps before split-point selection and derives relative
     timing from that clamped value. A clamp only during event conversion is too late.
   - **Defects 8, 9 and 11** span `wrapper.rs` and `state.rs`. Lock-held restoration (polling
     upstream's `TryLock`), post-load reactivation, snapshot rollback without `reset()`, the quiet
     `state_transaction_active` discard in `process()`, `StateChanged` after every completed
     transaction (which also carries defect 3's rollback case), rejected-field detection and
     scoped canonicalization acceptance only work together.

   *(0.3.0 wording, superseded by the counts above: Re-apply the eleven defect hunks, the
   GUI-authored state-dirty hunk, the floating-window hunks and the GUI-task process-wake hunk;
   `grep -rn "MXM PATCH"` on the *old* copy shows exactly what they were. Defect 1 spans
   `wrapper.rs` and `context.rs`: it includes activation-time reservation, bounded prefix/suffix
   input inspection, guarded pushes and O(1) release replacement; reservation or bounded storage
   alone is not the fix. Defect 7 clamps absolute timestamps before split-point selection and
   derives relative timing from that clamped value; a clamp only during event conversion is too
   late. Defects 8, 9 and 11 span `wrapper.rs` and `state.rs`: lock-held restoration, snapshot
   rollback without `reset()`, rejected-field detection and scoped canonicalization acceptance only
   work together.)*
3. Run the regression tests — they are what proves the refresh did not drop a fix. Debug bundles
   must be staged before the allocation regressions; release bundling overwrites them. Validate every
   consumer because shared wrapper success through one plugin does not prove another plugin's port,
   parameter, state or editor configuration:

   ```bash
   cargo test --manifest-path vendor/nice-plug/Cargo.toml --target-dir target/vendor-nice-plug --locked --lib --no-default-features mxm_state_tests
   cargo build -p nice-plug-output-fixture

   plugins="mxm-mono-01 mxm-mono-03 mxm-poly-06 mxm-mono-00 mxm-mono-02 mxm-chorus-06 mxm-folded-spring mxm-para-07 mxm-fx-convolution mxm-fx-curve"
   for plugin in $plugins; do
     cargo xtask bundle "$plugin"
     clap-validator validate "target/bundled/$plugin.clap"
   done
   cargo test -p mxm-player --test plugin_robustness
   cargo test -p mxm-player --test mxm_para_07_behaviour trigger_parameter_edges_are_sample_accurate_and_restore_only_a_level
   cargo test -p mxm-player --test mxm_fx_convolution_behaviour active_reactivation_failure_rolls_back_response_parameters_engine_and_live_tail

   for plugin in $plugins; do
     cargo xtask bundle "$plugin" --release
     clap-validator validate "target/bundled/$plugin.clap"
   done
   cargo xtask fixtures --release
   cargo test -p mxm-player --tests
   ```

   The first command uses the monorepo layout (`vendor/nice-plug`). In this repository the
   in-crate regressions are:

   ```bash
   cargo test --locked --lib --no-default-features -- mxm_state_tests canonicalized_field
   ```

   Since 0.4.2, `nice-plug-output-fixture` must be rewritten before it builds, because
   `ProcessContext::send_event` is gone and the wrapper has no output queue (see defect 1). The
   debug input-overflow regression needs more than twice the new 1,024-event floor to reach the
   skipped middle.

4. Rebuild every plugin in debug, run the native `editor_resize` regression below, then repeat
   with release bundles. This test is ignored by ordinary workspace runs and must be invoked
   explicitly. Preserve local-only floating resize handling, not merely floating advertisement.

## Removing this copy

Remove `vendor/nice-plug/` and the `[patch.crates-io]` redirect only when the replacement fixes all
eleven defects **and preserves GUI-authored state dirty reporting and floating-window support,
including local-only floating resizing**.

*(0.4.2 refresh: upstream 0.4.2 already covers 3, 4, 5 and 6. A replacement must still cover:*

- *defect 1's input bound*
- *defect 2's exact-span read*
- *defect 7*
- *defect 8, including reactivation after a state load, which upstream 0.4 removed*
- *defects 9, 10 and 11*
- *the three capabilities)*
Bump the pin and run the existing regressions against freshly rebuilt bundles. Keep those tests
after removing the patch: they prove the upstream replacement actually preserves behavior.

## Licence

`nice-plug` is ISC; see `README.md` and the upstream repository at
<https://codeberg.org/RustAudio/nice-plug>. Offer local patch contributions under the same terms;
this states the intended licensing, not a claim that a contribution has been submitted.

---

## GUI-authored state dirty — a capability, not a fix

A persistent field can change the plugin's saved state without changing a parameter. CLAP does not
infer that mutation: the plugin must call `host.state.mark_dirty`, or a host may close a project or
replace a preset without offering to save the edited curve or imported asset. nice-plug's public GUI
state transaction had no such notification.

The wrapper now queries `clap_host_state` during initialization and, after a successful
`GuiContext::set_state()` transaction, calls `mark_dirty()` on the main thread. The call is kept at
the GUI entry point rather than in `set_state_inner()`: host-driven preset/project restore uses the
same inner function and must not mark itself dirty. A host that does not expose `clap.state` keeps the
existing behavior.

A parameter-free GUI transaction whose fields already equal the canonical current fields is the
capability's **dirty-only form**. It skips `set_state_inner()` entirely — no plugin mutex,
reactivation or reset — and only issues `mark_dirty()`. `mxm-fx-curve` prepares and publishes each
completed authored gesture first, then sends this no-op transaction. The distinction is realtime
critical: its former full restore held the plugin mutex while preparing every 1,024-interval curve
segment; with audio playing, repeated two-stage wavefolder edits made `process()` contend, and
parking_lot's first contended lock allocated its 1,024-byte global parking table under
`assert_process_allocs`, aborting the host. The dirty-only predicate is narrow and semantic: an empty
parameter map and fields byte-equal to `Params::serialize_fields()`. Any actual restore retains defect
8's lock-held rollback transaction.

*0.4.2 refresh, ported.* Unchanged. The plugin lock is now upstream's non-blocking `TryLock`, so the
parking_lot parking-table allocation described above can no longer happen. But a full restore
that held the lock during playback would now make `process()` discard every block for the length
of the preparation. The dirty-only form is still the point. 0.4.2 still never calls
`clap_host_state.mark_dirty`.

The plugin regression host proves edits and undo send no parameters, carry the already-committed
canonical field, and take the host-visible path. The vendored unit test pins the dirty-only predicate,
while MXM Player implements `HostStateImpl::mark_dirty` as its real CLAP endpoint.

## GUI-task process wake — a capability, not a fix

CLAP separates `request_callback` (main-thread work) from `request_process` (resume an audio
processor the host has slept). Upstream's `schedule_gui(Task::PluginTask)` performs or requests the
main-thread task only. That is sufficient for background results and insufficient for a transient
editor action whose consumer is `Plugin::process`: an inert plugin may never be called again.

Before executing or queueing an explicit plugin GUI task, the wrapper now calls
`clap_host.request_process`. Parameter edits retain their existing `params.request_flush` path;
this applies only when plugin code deliberately schedules its own task. `mxm-mono-08` schedules the
no-value `EditorTask::WakeAudio` exactly once after each accepted Fire once activation. Its editor
proof verifies accepted/rejected action scheduling without creating a parameter gesture;
`apps/mxm-player/tests/mxm_mono_08_behaviour.rs::editor_once_wakes_a_sleeping_real_host_and_produces_one_burst`
loads the debug bundle in a child with immutable startup configuration and verifies the complete
bounded editor producer → this wrapper's `request_process` → sleeping real host → one audible
firing chain without mutating a multithreaded process environment.

*0.4.2 refresh, ported.* Unchanged. 0.4.2's `schedule_gui` still only executes or queues the task.

## Floating windows — a capability, not a fix

Upstream refuses **every** floating configuration: `ext_gui_is_api_supported` returns `false` the
moment `is_floating` is set, and `ext_gui_get_preferred_api` reports `is_floating = false`. The two
functions that only matter for floating windows — `ext_gui_set_transient` and
`ext_gui_suggest_title` — are stubs whose own comments say *"This is only relevant for floating
windows"*. Upstream left the door labelled and shut.

**Why we open it.** `apps/mxm-player` shows a plugin's own interface. Embedding it — the plugin's
window inside the host's — needs a container window in three platform implementations, and on
Wayland it is impossible: there is no cross-process embedding primitive, which is why CLAP's own
`GuiApiType::WAYLAND` supports floating and forbids embedding. Floating works everywhere and needs
no per-platform code in the host.

**It is a small change, because the window layer already supports it.** `Editor::spawn` takes
`parent: Option<ParentWindowHandle>`, and this wrapper already passes `None` — the parent arrives
later through `set_parent`. Floating simply stops waiting for one.

| Function | Change |
|---|---|
| `ext_gui_is_api_supported` | Accept `is_floating`. The API check below it is untouched, so floating is accepted for exactly the platform APIs embedding is |
| `ext_gui_create` | `spawn(None, !is_floating, …)` — `wait_for_parent` is what defers window creation until `set_parent`, and a floating editor has no `set_parent` to wait for |
| `ext_gui_set_parent` | **Refuse when floating.** baseview's `Window::set_parent` documents a panic for a window created with no parent and `wait_for_parent` false — exactly this configuration — and that panic would be in the host's process |
| `Wrapper::editor_is_floating` | One field, because `create` and `set_parent` are separate host calls and `SpawnedEditor` records no creation mode. Refusing *every* reparent instead would break embedded hosting in every DAW that uses it today |
| `ClapHostCallbacks::request_resize` | Capture `is_floating` during creation and return success locally for floating windows. CLAP's callback asks for a parent's client area; baseview already resized this parentless window. Embedded editors still forward the request, including host refusals |

*0.4.2 refresh, ported.* 0.4.2 still refuses every floating configuration, in the same functions.
#88 was closed after the maintainer noted that VST3 has no plugin-owned windows, so upstream support
would have to be exclusive with the `vst3` feature. All five rows above are re-applied.

`request_resize` now converts with upstream's `NativeSize::from_size(new_size, scale_factor)`:
physical pixels on Windows and Linux, logical points on macOS. That conversion is upstream's fix
for macOS embedded editors scaling instead of reflowing and coming out twice too large; this copy
adds no size conversion of its own. The floating early return comes before the conversion and
still returns `Ok(())` without calling the host. Embedded editors forward the `NativeSize`
request, including host refusals, exactly as upstream does. `spawn(None, !is_floating, …)` is
unchanged: `Editor::spawn` keeps its `wait_for_parent` argument in 0.4.2.

**`ext_gui_get_preferred_api` is deliberately untouched.** It is a hint a host may ignore; the
player asks for floating explicitly. Changing it would tell third-party X11 hosts to stop embedding
mxm-mono-01 to fix a problem those hosts do not have.

**`ext_gui_set_transient` and `ext_gui_suggest_title` remain stubs.** Implementing them needs a
`set_owner` in `baseview` and a `set_transient` on `nice-plug-core`'s `EditorHandle` — three crates
that are not vendored here. The player works around the first from its own side
(`apps/mxm-player/src/ownership.rs`); the proper fix belongs upstream.

**Regression tests:** `apps/mxm-player/tests/t7_editor.rs::mxm_mono_01_advertises_a_floating_editor`
holds the advertisement. `apps/mxm-player/tests/editor_resize.rs` holds the native resize path:
open each of the eight real bundles, resize 24 times, close and repeat, requiring accepted sizes
and **zero host resize round-trips**. On the broken Windows release baseline, mono-00 and mono-01
each produced 24/24 round-trips; the corrected bundles produce none. Forwarding those requests
woke the player on every drag event, defeating its deliberately throttled frame requests and
making a second GUI compete with the editor. This test measures that unnecessary work, not FPS.
The owner subsequently confirmed that the rebuilt editors resize smoothly (“smooth as butter”);
that manual confirmation closes the reported symptom, not the untested platform/DAW cases below.
See [the issue record](../../docs/known-issues.md#floating-editors-resize-slowly) for the evidence
and the distinction from layout-only optimizations.

Run explicitly after bundling every plugin, once with debug bundles and again with release bundles:

```bash
cargo test -p mxm-player --test editor_resize -- --ignored --nocapture
```

The driver needs a Windows desktop. Linux/macOS and embedded DAW resize behavior are not verified
by it. Reapply the callback hunks as part of floating support on refresh, not just its advertisement.
