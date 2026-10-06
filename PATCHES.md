# Patched `nice-plug`

This is **`nice-plug 0.3.0` exactly as published on crates.io**, plus eleven fixes and three
capability changes. It is wired in through `[patch.crates-io]` in the root `Cargo.toml`.

Every change is marked `MXM PATCH` in the source, so `grep -rn "MXM PATCH" src/` finds the complete
delta. It lives in `src/wrapper/clap/wrapper.rs` and `src/wrapper/state.rs`, plus the output-push
guard in `src/wrapper/clap/context.rs`.

**The last three are different in kind from the eleven** — they are not defect fixes. See
*GUI-authored state dirty*, *Floating windows* and *GUI-task process wake*, below.

## Why this exists

The first three defects surfaced when the collection's own host (`apps/mxm-player`) drove
mxm-mono-01 with an allocation-counting audio callback and a `clap-validator` run; later instrument
reviews found the later eight — the fifth with `plugins/mxm-chorus-06`, **6 and 7 during
`mxm-para-07`'s factory review**, and **8 through 11 during `mxm-fx-convolution`'s**. **None is
fixable from plugin code** — the affected buffers, state loader, GUI state restoration, host-value
application, port declarations, automation splitting, persistent-field transaction protocol and
host-tail notifications belong to the wrapper. All eleven are present in the pinned published
release. At the original investigation the first three were still present on upstream `main`
(`542daf1`, 2026-08-21).

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

**2 — bound the declared state size and reserve fallibly.** Lengths above `MAX_STATE_SIZE` (512
MiB), lengths not representable as `usize`, reservation failures and short streams are refused with
`false`. The stream read is sliced to exactly the declared payload even if `Vec` reserves more.
Rejecting a bad preset is recoverable; aborting or consuming a following stream item is not. Direct
unit regressions force a 512 MiB reservation to fail under a test allocator and put trailing bytes
behind a short declared payload; the player regression retains the real hostile-length host path.

**4 — drop a non-finite host value at the event.** Both arms of `update_plain_value_by_hash`
return early when the value is not finite: the event is consumed (the hash was known) and the
parameter keeps its value. Nothing else in the path checks — the range's `normalize` clamps with
`f32::clamp`, which returns NaN for NaN. Regression test:
`plugin_robustness.rs::a_non_finite_parameter_value_from_the_host_is_dropped`, through the
player's own host path: a NaN cutoff arrives while a note sounds, the audio stays finite, the note
keeps sounding, and the cutoff reads what it was.

**5 — pair the main ports in place only when they are the same shape.** `pair_stable_id` now
requires `main_input_channels == main_output_channels`; otherwise both ports report
`CLAP_INVALID_ID` and the host uses separate buffers, which the wrapper already handles by copying
the input over the output. Found by `plugins/mxm-chorus-06`, the collection's first effect, whose
faithful layout is mono in, stereo out (2026-09-04). Same-shape layouts pair exactly as before.
Regression test: `clap-validator` on `mxm-chorus-06` in both profiles — the two tests named above
fail without this hunk and pass with it; the instruments, all output-only, never reach it.

**3 — schedule `Task::RescanParamValues` in `set_state_inner()`.** Every completed state transaction,
including rollback, invalidates the host's parameter cache.

**6 — check the first unread event before applying it.** The wrapper's later events already receive
this look-ahead check. Applying the same predicate to the event at `resume_from_event_idx` splits the
leading audio span before changing the parameter. Regression test:
`mxm_para_07_behaviour.rs::trigger_parameter_edges_are_sample_accurate_and_restore_only_a_level`,
through the bundled CLAP shell with a low-high-low pulse at nonzero offsets.

**7 — clamp absolute event time before splitting.** A later clamp during event conversion cannot
make an already selected audio segment safe. Split selection now compares and returns the timestamp
clamped to the complete host buffer, and segment-relative timing is derived from it with saturating
subtraction. Regression test:
`mxm_state_tests::out_of_range_split_event_is_clamped_before_buffer_partitioning` drives the exact
timing primitive with an event beyond a 64-frame buffer, bounds the split at sample 63 and proves
that processing resumed there consumes the event at relative sample zero.

**8 — restore GUI state under the plugin lock on the GUI thread and snapshot before mutation.** The
zero-capacity audio-thread handoff is removed. Deserialization, persistent-field preparation and
reactivation complete while one lock excludes processing. Failure deserializes and reactivates the
snapshot before releasing that lock, but deliberately does not call `reset()`: a rejected transaction
must preserve live processor history as well as durable state. Its two guarded reactivations are
nested `if`s, not `let` chains, because this crate declares Rust 1.87 and the nice-plug fixture
builds at that floor (`docs/known-issues.md`, *Two crates declared an MSRV they could not build on*).

**9 — detect rejected persistent fields.** After `deserialize_fields()`, supplied fields must
serialize to the same JSON value (formatting and object order may differ). A mismatch returns false,
feeding defect 8's complete rollback rather than silently accepting a partial state. Regression:
`apps/mxm-player/tests/mxm_fx_convolution_behaviour.rs::active_reactivation_failure_rolls_back_response_parameters_engine_and_live_tail`
loads a valid but deadline-impossible embedded response into one of two matched active 384 kHz
instances while also changing parameters, requires the CLAP load to fail and prior state bytes to
return, then requires its still-nonzero tail to remain bit-identical to the untouched instance.

**10 — notify finite/infinite tail-class changes.** Initialization caches only the host extension's
`changed` function pointer. Each process publication atomically stores the new `ProcessStatus`, then
calls that pointer only when `KeepAlive` membership changes; an immediate host re-query therefore
observes the new class. The audio-thread path takes no lock and allocates nothing. The focused fake
host regression checks both edge-only delivery and store-before-callback ordering.

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

## What it buys

`clap-validator` on mxm-mono-01, before and after, **debug** bundle (the allocation guard only fires in
debug, so this is the run that matters):

| | Before | After |
|---|---|---|
| passed | 30 | **35** |
| failed | 5 | **0** |
| crashed | 2 (`0xc0000409`) | 0 |

## Upstream status — 2026-09-06

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

1. Extract the new `nice-plug` release over this directory, keeping `PATCHES.md`.
2. Re-apply the eleven defect hunks, the GUI-authored state-dirty hunk, the floating-window hunks and the GUI-task process-wake hunk; `grep -rn "MXM PATCH"` on the *old* copy shows exactly what they were. Defect 1 spans `wrapper.rs` and `context.rs`: it includes activation-time reservation, bounded prefix/suffix input inspection, guarded pushes and O(1) release replacement; reservation or bounded storage alone is not the fix. Defect 7 clamps absolute timestamps before split-point selection and derives relative timing from that clamped value; a clamp only during event conversion is too late. Defects 8, 9 and 11 span `wrapper.rs` and `state.rs`: lock-held restoration, snapshot rollback without `reset()`, rejected-field detection and scoped canonicalization acceptance only work together.
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

4. Rebuild every plugin in debug, run the native `editor_resize` regression below, then repeat
   with release bundles. This test is ignored by ordinary workspace runs and must be invoked
   explicitly. Preserve local-only floating resize handling, not merely floating advertisement.

## Removing this copy

Remove `vendor/nice-plug/` and the `[patch.crates-io]` redirect only when the replacement fixes all
eleven defects **and preserves GUI-authored state dirty reporting and floating-window support,
including local-only floating resizing**.
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
