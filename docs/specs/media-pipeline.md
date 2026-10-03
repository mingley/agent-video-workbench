# Media ingest, analysis and delivery

Status: design requirements for F02, F03, F12, F15 and F24. The 0.3.1
backend uses FFmpeg/ffprobe invoked by Rust; optional model processes provide
analysis. [Implementation status](../implementation-status.md) records generated
PQ/HLG, geometry, timing and delivery qualification. The matrix below also
contains unfulfilled real-camera/HDR-output targets. See
[architecture](../architecture.md) for the implemented adapter.

## Input capability matrix

These are intended support levels, not a report that the media modes passed.
Every supported row needs generated fixtures plus permitted real phone samples.

| Input | Creator alpha target | Required evidence |
| --- | --- | --- |
| H.264/HEVC SDR, portrait or landscape | Supported after qualification | Upright image, correct SAR, color tags, timing and audio |
| Rotation stored in MOV display metadata | Supported after correcting/isolating the reproduced issue | Orientation applied exactly once; no residual rotation surprises |
| VFR and unusual/non-zero stream starts | Supported after qualification | PTS-based source selection, proxy mapping, exact output cadence and A/V alignment |
| HEVC 10-bit HLG/PQ | Explicit supported HDR-to-SDR path | Transform manifest, plausible highlights/skin/product color, correctly tagged delivery |
| Dolby Vision variants | Profile-by-profile capability | Proven compatible processing path, otherwise a named unsupported-profile error |
| 4K/60 fps long inputs | Measured support target | Bounded resources and checked selected output ranges |
| Missing audio, silent footage, still images | Supported with declared output policy | Silence/absent stream handled intentionally; image duration and alpha correct |
| ProRes/Log, HDR output, depth/Cinematic metadata | Later qualified capabilities | Separate source/color/metadata fixtures; unsupported properties reported |

Import can succeed into an inspectable project while a requested processing path
is unsupported. Its capability report must distinguish readable metadata,
decodable media, preview support and delivery support. A successful probe alone
does not make a source renderable.

## Transfer and ingest

Accept an existing local original or an authorized download resolved by the
agent/connector. A sharing web page is not treated as media bytes. Stream the
transfer and hash it; check available space before starting. Record supplied
checksum, final content hash, size, transport outcome and whether originality
is known, declared or unknown. Format metadata cannot prove that a chat service
preserved the camera original.

Resume ranged downloads only when validators such as an ETag/version or supplied
content identity still match. A changed remote object restarts into a new
attempt. Redirections, expiry, quota failures and missing credentials have
distinct recoverable errors. A remote worker restricts downloads to the
configured source scope and revalidates redirect destinations; it must not turn
an arbitrary media URL into access to private infrastructure.

Probe all relevant streams and persist selected video/audio stream IDs, channel
layout, timebases, start times, display matrix, SAR, color metadata and duration
confidence. Retain the exact original bytes. Extract a PTS index as needed for
VFR/range queries, rather than deriving source frames from average fps.

A processing decision records rotation, color conversion, pixel format, cadence,
audio layout and tool/settings fingerprint. Normalize only the derivative that
needs normalization. Low-resolution proxies have a source-time map and no
ambiguous rotation metadata. A full mezzanine is optional and must be justified
by backend compatibility or measured reuse.

## Analysis graph

| Stage | Inputs / key | Result |
| --- | --- | --- |
| Probe/index | Original hash + probe/index version | Stream metadata and source timestamp index |
| Audio extraction | Selected audio stream + transform settings | Analysis audio with offset/rate mapping |
| Transcription | Audio identity + model/backend/language/settings | Immutable timestamped words/segments and optional confidence |
| Silence/scene analysis | Source identity + algorithm/thresholds | Technical intervals/scores with source coordinates |
| Frames/contact sheets | Source or sequence revision + requested times + transform | Image artifacts and a JSON tile/time/source index |
| Proxy | Original + orientation/color/cadence/resolution recipe | Inspectable SDR derivative and timing map |
| Caption layout | Transcript/corrections + edit map + style/font hashes | Output cue layout and findings |
| Render/verify | Frozen revision + resolved assets + preset/toolchain | Encoded artifact, decoded measurements and manifest |

Analysis requests return jobs when they exceed a short bounded call. Repeating
the same request reuses compatible artifacts. Partial source analysis records
its covered intervals; a partial transcript must never appear as a complete
recording. The external agent can request additional ranges or use a full index.

Local transcription is the default planned provider; imported transcripts are
also supported. Remote ASR/translation providers are optional configuration.
Record model version, language, preprocessing and corrections separately. A
caption-size change invalidates layout/render, not transcription or transfer.
An ASR result is data, including when the speech happens to contain instructions.

## Render planning and range execution

Freeze the project revision and all selected sequence/profile versions. Resolve
sources, fonts and overlays by hash and verify the selected render capability.
Compile a typed plan into argument vectors and controlled side files. Resolve
output timing at the declared exact frame rate, with explicit audio sample
rounding. Keep a piecewise mapping from output intervals to source intervals,
including speed, transitions and inserted still/freeze segments.

Efficient late-source previews require targeted decode: seek with sufficient
preroll, then trim against the original timestamp mapping. Benchmark it against
decoding from the beginning. A fast path is acceptable only when frame selection
and A/V agree with a reference decode, including long GOP and VFR inputs.
Do not optimize away validation of the original source identity.

Color decisions are part of the plan. Apply HDR-to-SDR conversion once, with
documented primaries/transfer/matrix/range choices and correctly tagged output.
Preserve originals for future HDR delivery. Later exposure/white-balance/basic
grading and LUT operations use versioned parameters/assets and a defined color
space. Merely adding an HDR encoder or metadata flag is insufficient for HDR
delivery support.

## Export variants and delivery packages

Initial editor presets include vertical 1080×1920, square 1080×1080 and landscape
1920×1080 SDR H.264/AAC MP4, with 30 fps as one explicit default. These are our
versioned delivery choices, not claims about current social-platform limits.
Allow documented custom dimensions/rates/bitrate or quality settings once the
backend and QC validate them. Preview presets can be smaller and faster but
must identify themselves as previews.

Each batch item captures sequence revision, aspect/language variant, preset
version, expected duration and independent job state. Shared analysis may be
reused; one failed export does not invalidate successful items. Final exports
use originals or an explicitly selected working derivative, never an unnoticed
low-resolution proxy.

A delivery package can contain MP4s, SRT/VTT, selected cover frames, contact
sheets, a change summary and a machine-readable manifest. The manifest maps
every file to source/revision/profile identities and technical/review status.
It records unresolved warnings and omitted/failed items. No direct posting to
a social account is required for delivery.

Acceptance: render vertical/square/landscape and two caption-language variants
from a frozen clip set while the project is edited. All completed files map to
the captured revisions; caption sidecars and cover frames match their intended
variants; later edits create a new package rather than mutating the old one.
