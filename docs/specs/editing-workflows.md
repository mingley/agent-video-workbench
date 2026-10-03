# Editing workflow specifications

Status: design requirements. Feature IDs refer to the [product plan](../product-plan.md).
The 0.3.1 service implements many higher-level workflows through `compose` and
`studio`; [implementation status](../implementation-status.md) identifies tested
subsets and unsupported rendering combinations. Requirements below also include
extensions. Use [the current interface](../agent-api.md) and generated schema
for requests; [the agent protocol](agent-protocol.md) retains broader design targets.

## Timeline operations and multiple sources — F01, F07, F08

Every video/audio element has a stable ID, source binding, half-open source
range, timeline range, sequence/track ID and relationship to linked elements.
Splitting creates child IDs and records lineage. Changing source footage keeps
an editorial slot ID while recording the changed source binding. Audio from a
video must have one intentional route into the mix; it cannot be duplicated
implicitly by inserting a linked audio element.

| Edit | Required behavior | Rejection / review case |
| --- | --- | --- |
| Trim | Set explicit source boundaries; move linked audio and source-bound captions under the selected link policy | Out-of-source range, invalid handles or protected coverage loss |
| Ripple remove | Remove a range and shift the explicit track/link group by its exact output duration | Cross-track elements outside the group would be unintentionally desynchronized |
| Lift remove | Remove selected material and retain its timeline gap | A deliverable with an unexplained gap receives a QC finding |
| Insert | Insert material and shift the specified downstream group | Ambiguous group membership must be resolved in the request |
| Overwrite | Replace the requested timeline range; split affected elements and record omitted-source lineage | Mismatched replacement duration requires an explicit timing policy |
| Move/reorder | Specify destination, sequence and gap/ripple policy; preserve source binding | Overlap on a non-overlapping track or illegal linked-media placement |
| J/L cut | Explicitly allow different linked picture/speech boundaries with sufficient source handles | Silent loss of the intended link or out-of-range audio |
| Speed | Specify rate and audio pitch policy; remap words, captions, source coverage and output duration | Unsupported variable-rate curve or a protected range requiring normal speed |
| Freeze | Bind the chosen source frame and an explicit output duration; specify held/silent/continuous audio | A freeze cannot silently duplicate speech |
| Transition | Name a supported transition, duration and available handles; record any overlap | Insufficient handles produce a precise error or an explicitly selected fallback |

The first richer editing release supports cuts, fades/dissolves and constant
speed. Variable speed ramps and elaborate transitions wait for timing fixtures.
All operation expansion is visible in dry-run: affected IDs, new IDs, changed
duration, source ranges, caption effects and warnings. The agent does not need
to assemble FFmpeg filter strings.

Multiple phone recordings retain separate transcripts, stream timebases and
color/orientation decisions. B-roll lives on a visual layer over continuous
dialogue. A replace-B-roll operation targets that layer; it cannot trim the
underlying speech. Stills/logos have an asset hash, duration, fit/transform and
alpha policy. Imported reusable assets remain available to old revisions.

Acceptance: a three-source sequence with linked dialogue, B-roll, logo and a
J-cut renders after reopen; reordering and undo preserve source/A/V lineage.
The operation diff explains what happened to each affected layer.

## Transcript editing and restoration — F03, F04

ASR output is an immutable analysis version with source-time words/segments,
language and optional confidence. Corrections are an editable overlay. Changing
a recognized word does not alter the audio. A re-transcription produces another
analysis version; existing word references stay pinned until an explicit rebind.
Uncertain or unmatched rebindings become unresolved references, not guessed cuts.

An omitted interval records its asset, source bounds, originating operation,
reason and sequence context. Restoring it references this omission ID plus a
destination and ripple policy. If the same interval already exists at that
destination, validation reports the duplicate; retries with the same request
key replay the prior outcome. Selective restoration preserves unrelated edits.

Removing a sentence requires explicit source ranges. Configurable handles
protect consonants/breaths at a cut. Silence analysis proposes removals; it does
not automatically authorize them. Protected demonstrations can cover silent
footage and apply to chosen sequences, with required video/audio streams.
Coverage uses the union of retained source ranges; repeating a small piece does
not satisfy preservation of the whole demonstration. Normal playback is the
default protection policy. Changing that policy is itself a recorded edit.

Acceptance: remove two sentences, make a later crop/style change, then restore
only the first sentence after reopening. The second removal and later styling
remain intact. A protected silent demonstration survives pause removal.

## Captions and language variants — F04, F10, F13

Keep source transcript, corrected text, displayed caption text, cue grouping,
language and visual style distinct. Captions normally bind to source word IDs;
manual cues declare timeline anchoring explicitly. A trim/reorder maps retained
words into output time. Removing/replacing a source invalidates affected bindings
and requires regeneration or an explicit manual override before final QC.

Style properties include pinned font/fallback assets, size relative to canvas,
weight, fill, outline/shadow/background, alignment, safe area, line count and
cue length limits. Templates may emphasize selected words using supported
styles. Advanced word animation is a capability with its own render tests.
Font substitution must be recorded; a missing required font cannot silently
change a final export. A preview fallback is allowed when visibly reported.

Translated captions reference the source analysis and translation revision,
retain corrections, and maintain an alignment map instead of assuming one
translated word per spoken word. Speaker labels and meaningful non-speech cues
are supported as editable annotations. RTL/bidirectional text, combining marks,
emoji/fallbacks and mixed scripts require rendered fixtures before claiming a
language/script is supported. Translation is optional and can be imported or
provided by the external agent; the editor need not call a proprietary model.

Generate burned captions and/or SRT/VTT from the same cue data. Container/font
limits and unsupported styling in sidecar formats appear in an export loss
report. Changing visual style does not change words or rerun ASR.

Acceptance: correct a product name, trim/reorder speech, reduce caption size and
export two language variants. Words and timing remain attributable to sources;
sidecars match the burned cues; supported scripts render without missing glyphs.

## Framing and motion — F05, F14

Each aspect-ratio variant has its own framing track. Coordinates refer to the
upright source display frame after orientation/SAR normalization; the output
canvas and safe-area coordinate spaces are separate and declared. Use contain,
cover or an explicit crop. Never switch among them silently.

Manual keyframes define position/scale, timestamp space and interpolation.
The renderer clamps nothing silently: validate crop bounds and report clipping.
Assisted reframing returns an editable proposal with subject/region references,
confidence when available, keyframes, algorithm/model version and cost evidence.
The creator or agent selects the product/person to follow; the tool does not
assume the most prominent face is the intended subject.

Use limits on motion speed, smoothing and minimum hold time to avoid distracting
movement. On occlusion or ambiguous tracking, the configured policy can hold the
last accepted crop, contain the frame, or flag the interval for review. Record
the fallback. A new source trim remaps crop keyframes by source time; a different
source binding requires revalidation.

Acceptance: a handheld portrait/landscape demonstration retains the selected
product in all approved crops. A failed tracking interval yields a visible
finding and predictable fallback. Vertical and square variants can differ
without modifying each other's crop tracks.

## Audio finishing — F06

Model dialogue, original ambient sound, music and added effects as named buses.
Provide clip gain, fades/crossfades, mute, pan, basic EQ, optional cleanup and
sidechain ducking. Loudness normalization is a measured two-pass job where the
selected method requires it, with target settings pinned to an export profile.
An audio-only change should reuse source video analysis.

Cleanup/de-noise is optional, parameterized and reversible. Expose a short A/B
preview before applying aggressive processing. Keep an unprocessed speech path
available through history. Music assets carry their provenance/license notes;
the project does not infer usage rights from a file being downloadable.

Acceptance: B-roll replacements do not change dialogue timing; ducking reduces
music around speech without changing its source; measured output meets the
chosen loudness/peak target and has no accidental double audio. Review speech
around cuts and processing changes on actual playback.

## Templates, clip sets and interchange — F11, F12, F21

A template is a versioned parameterized edit specification: named source slots,
required annotations, duration constraints, intro/outro/overlay elements, pinned
style and export defaults. It contains data and validated operations, not shell
commands or an embedded agent prompt. Instantiation resolves new source bindings
and produces a proposed batch with all concrete IDs and timing.

A clip set groups related sequences and their delivery variants. Common content
changes are offered as explicit updates to selected variants. Variant-specific
framing, text and overrides remain local; conflicts are listed before apply.
Freeze the set of revisions before batch export so an edit during rendering
cannot create a mixed delivery package.

Interchange starts with a documented subset such as cuts, source ranges, tracks
and markers. Consider AgentCut's existing interchange code behind an adapter.
Import/export must return a loss report for unsupported captions, effects,
speed curves, color transforms or unavailable media. A round trip is accepted
only when its supported subset retains source binding, order and timing on
fixtures. “Interchange available” never implies full editor compatibility.

Acceptance: instantiate one template against two different recordings without
reusing old source IDs. A multi-aspect export has a frozen manifest. A supported
timeline round trip matches expected edit timing and reports every unsupported
feature rather than dropping it without notice.
