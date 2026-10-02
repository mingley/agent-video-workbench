# Candidate evidence

Evaluated October 2, 2026. The initial descriptions were leads. This review
cloned actual source, read licenses/manifests and selected execution paths,
built three CLIs, ran an edit, decoded their outputs, and inspected extracted
frames. It is not a complete source/security audit. Upstream test suites,
published installers, ASR models, real phone footage, and hosted bot accounts
were not exercised.

The reproducible revision manifest is
[candidates.lock.json](../evaluation/candidates.lock.json). Every source link
below is pinned to the reviewed commit; release/installation information is a
dated observation, not a promise about future versions.

| Candidate | Reviewed commit | Actual source license | Execution evidence |
| --- | --- | --- | --- |
| AgentCut | `20ecdffc9d77` | MIT | Rust source build; CLI edit/render/revise/undo; fault injection and rotation test |
| agent-video-editor / ave | `71a75bda2d92` | MIT | Rust source build; silence detection, keep/crop/captions, decode |
| OpenReelio | `13993f3f8240` | MIT | Standalone Rust CLI source build and timeline/caption/render; error test |
| Kinewright | `b7f432c18041` | GPL-3.0-only | Source/license/build instructions inspected; not built or run |
| OpenReels | `edc1c973634a` | MIT | Source/license/package inspected; not installed or run |

## AgentCut

[Workspace manifest](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/Cargo.toml),
[license](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/LICENSE),
and [installation documentation](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/README.md)
establish an MIT Rust workspace with core, render, interchange and CLI crates.
The declared minimum Rust version is 1.75; this review used Rust 1.98.1, so it
does not verify that minimum. No GitHub release assets were published when
queried. The tested installation path was
`cargo build --release --locked -p agentcut-cli`; binary: `target/release/agentcut`.
FFmpeg/ffprobe remain external programs.

**Source-reviewed:** the
[model](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-core/src/model.rs),
[rational time](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-core/src/time.rs),
[operations](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-core/src/operations.rs),
and [render compiler](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-render/src/compile.rs)
are actual Rust implementations. Batches apply to a candidate snapshot and
validate before writing. CLI JSON envelopes, schemas, property capabilities,
an embedded agent guide, analysis, previews and caching exist in code. There
is no native MCP command in the inspected CLI, and no integrated ASR backend
was found. The analysis implementation covers technical signals such as
silence, black/frozen frames and loudness; it is not a semantic transcript or
scene understanding system.

**Runtime verified:** the generated edit produced a 360×640 H.264/AAC draft,
180 decoded frames at 30 fps, with both streams and container measuring 6.0 s.
Decoded pixels verified red intro then green demonstration; frame inspection
verified the captions. The retained source ranges were 0–3 s and 7–10 s.
Dedicated audio clips required disabling the video clips' embedded audio;
the final harness checks against accidental doubling. Stereo-normalized mean
audio level was −24.1 dB on the source versus −23.9 dB on the draft.

Separate CLI processes rejected an invalid batch without changing project
bytes, replayed a matching idempotency key without another revision, and
returned `E_REVISION_CONFLICT` on a stale request. The identical render hit
cache and produced identical bytes. A new batch restored source 3–4 s, moved
the demonstration/caption, reduced caption size from 22 to 18, and changed
the opening caption. Its output was 210 frames / 7.0 s. Undo restored the
original rendered bytes; redo completed. The grey restored interval is a
mechanical stand-in for a removed sentence, not a speech-recognition test.

Two important gaps are supported by executable evidence:

1. **Undo history can be lost silently.**
   [Repository commit](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-core/src/repository.rs#L170)
   writes the project before its separate journal and ignores journal-save
   errors. On a disposable project, replacing the journal path with a directory
   made the edit report success with no warnings; undo then returned
   `E_OPERATION_INVALID` / nothing left to undo. Atomic replacement of one
   snapshot is insufficient for atomic state, history and retry outcomes.
   The [journal](https://github.com/wuwangzhang1216/agentcut/blob/20ecdffc9d770bc280b294ee00414cafe4ce36ed/crates/agentcut-core/src/journal.rs)
   also defaults on corrupt/unreadable data and retains 100 entries by default.
   The reviewed read/check/write commit path has no writer lock; the concurrent
   race implication is source-based, not a reproduced concurrency test here.
2. **The rotation-tagged MOV fixture fails rendering.** A 640×360 stored frame
   with a 90-degree display matrix should display upright at 360×640. The
   current render exited with `E_RENDER_FAILED` and an invalid 360×640 crop.
   Input arguments leave FFmpeg autorotation enabled while the filter graph
   applies its own transpose. In a supplementary diagnostic, adding
   `-noautorotate` before this input made the same graph render 90 frames / 3 s
   at 360×640, and the extracted frame matched the upright reference visually.
   This supports the double-rotation explanation. It is not a committed
   upstream fix or coverage of every phone orientation/backend.

HDR tags and a pixel-format conversion do not establish correct tone mapping.
No explicit HDR tone-map path was found in this compiler. Real HLG/PQ/Dolby
Vision, HEVC/VFR timing, and original-to-proxy mapping need their own ingest
tests. These gaps justify reusing pure core/render components behind our own
store and ingest boundary rather than adopting the CLI unchanged.

## agent-video-editor / ave

[Manifest](https://github.com/MatheusBBarni/agent-video-editor/blob/71a75bda2d9289f9dc0ace802ec274f0f580bb8f/Cargo.toml),
[license](https://github.com/MatheusBBarni/agent-video-editor/blob/71a75bda2d9289f9dc0ace802ec274f0f580bb8f/LICENSE),
and [README](https://github.com/MatheusBBarni/agent-video-editor/blob/71a75bda2d9289f9dc0ace802ec274f0f580bb8f/README.md)
show a small MIT executable, version 0.2.0, edition 2024, declared Rust 1.85.
Dependencies are principally clap/serde/serde_json. A registry package is
documented at [docs.rs](https://docs.rs/crate/agent-video-editor/0.2.0);
`cargo install agent-video-editor` is documented, but this review tested a
pinned source build, not registry installation. GitHub release v0.2.0 had no
binary assets. Binary: `target/release/ave`.

**Source-reviewed:** typed commands generate subprocess argument vectors,
JSON responses and dry-run plans. Schema discovery, frame extraction, silence
detection, captions, crop/resize and hardware options are implemented. Its
[execution path](https://github.com/MatheusBBarni/agent-video-editor/blob/71a75bda2d9289f9dc0ace802ec274f0f580bb8f/src/exec.rs)
runs a plan sequentially and preserves completed outputs if a later step
fails. It does not provide a durable editable timeline, revision store or
conversation-independent restoration history. Using intermediate MP4s as
project state would put those responsibilities on the external agent.

**Runtime verified:** silence detection found approximately 3.008–7.019 s;
accurate keep of 0–3 / 7–10 s, vertical crop and caption burn all executed.
The result decoded, had 360×640 dimensions and 180 frames. Its measured video
duration was 6.021333 s, audio 6.050667 s and container 6.054785 s, with average
video rate `33750/1129`. Source inspection shows re-encoded kept segments
joining through MPEG-TS/stream copy. This particular rounding/timestamp result
needs attention for frame-exact delivery; it does not establish that every
ave operation has the same drift. Attempted in-place output was rejected and
the original hash remained unchanged.

Useful as a simple operational tool or reference for command design; not the
main durable editing foundation for this project.

## OpenReelio

[License](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/LICENSE),
[CLI manifest](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/crates/openreelio-cli/Cargo.toml),
and [core shim](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/crates/openreelio-core/Cargo.toml)
verify MIT Rust, CLI version 0.1.13, and an explicitly GUI-free core dependency
configuration. `cargo build --release --locked -p openreelio-cli` succeeded
without starting a desktop window or installing the React/Node application.
Default CLI features include Whisper; that feature built, but no model was
downloaded or run. The broader backend dependency graph is a reuse tradeoff,
not proof that headless operation requires Tauri's GUI.

[Release v0.1.13](https://github.com/openreelio/openreelio/releases/tag/v0.1.13)
publishes standalone CLI archives and SHA-256 files for Linux x86_64 GNU,
macOS arm64/x86_64 and Windows x86_64. The
[npm wrapper](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/npm/openreelio-cli/package.json)
uses platform packages (`npm install -g openreelio-cli` / `npx openreelio-cli`).
These installation paths were inspected, not downloaded/installation-tested;
the runtime tests used the source-built binary. No Linux arm64 or musl CLI
archive appeared in that release's asset list.

**Source-reviewed:**
[MCP](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/crates/openreelio-cli/src/commands/mcp.rs)
implements stdio JSON-RPC, read-only by default with explicit write capability;
[transcription](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/crates/openreelio-cli/src/commands/transcription.rs)
and inspection/verification commands are real code. Neither an MCP handshake
nor ASR output was tested here. Its
[operation log](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/src-tauri/src/core/project/ops_log.rs)
has file locks/session watermarks. The
[plan executor](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/crates/openreelio-cli/src/commands/plan.rs)
has sequential execution/rollback with explicit incomplete-rollback reporting.
These are stronger persistence mechanisms than a best-effort journal, but
were not crash/concurrency tested or certified as a fully isolated batch
transaction by this review.

**Runtime verified:** project creation, source import, linked video/audio
insertion, explicit trims of both linked clips, caption addition and rendering
worked headlessly. The rendered file decoded to 180 frames and measured 6.0 s
for both streams and the container. Insertion creates a muted video clip plus
separate linked audio; editing only one ID does not automatically trim the
other in this path. The test intentionally used `mp4_h264_1080p`, which produced
1920×1080 despite a 360×640 project canvas. The
[preset implementation](https://github.com/openreelio/openreelio/blob/13993f3f824081f1d83204aa0c08f0288ce8e075/src-tauri/src/core/render/export.rs)
also offers a vertical Shorts preset; the observed landscape output is preset
behavior, not absence of vertical rendering. Missing-project `project info`
returned exit 1 with plain stderr and empty stdout, so uniform JSON errors
would need an adapter or improvement.

A credible alternative, especially if existing MCP/ASR/export breadth saves
more work than adapting its larger editing/state model. Run the same phone,
durability and host tests before selecting it over the smaller AgentCut core.

## Kinewright and OpenReels

Kinewright's
[manifest](https://github.com/CanadaApollo6/Kinewright/blob/b7f432c180414dc23851c3bbab9f4a66a9eb7f31/Cargo.toml)
and [license](https://github.com/CanadaApollo6/Kinewright/blob/b7f432c180414dc23851c3bbab9f4a66a9eb7f31/LICENSE)
specify GPL-3.0-only, Rust 1.92 and native FFmpeg/wgpu/egui/Whisper dependencies.
Its documented build targets Windows/Linux with shared FFmpeg 8.x. The
[application entry point](https://github.com/CanadaApollo6/Kinewright/blob/b7f432c180414dc23851c3bbab9f4a66a9eb7f31/crates/kinewright-app/src/main.rs)
starts the desktop app. Shared project I/O exists, but
[headless operation](https://github.com/CanadaApollo6/Kinewright/blob/b7f432c180414dc23851c3bbab9f4a66a9eb7f31/docs/AW1-HEADLESS-KINEWRIGHT.md)
is an evolving design/implementation track, not a shipped standalone CLI
verified at this commit. The inspected desktop workflow launches agent CLIs
inside its own harness. No GitHub release existed. It may offer useful design
references, but changing that boundary is unnecessary for this MVP. GPL is
open source; the reason to pass is workflow/platform fit and implementation
scope, not that copyleft makes it unusable.

OpenReels'
[license](https://github.com/tsensei/OpenReels/blob/edc1c973634a3e179a527d00857b85de7b52a1dd/LICENSE),
[package](https://github.com/tsensei/OpenReels/blob/edc1c973634a3e179a527d00857b85de7b52a1dd/package.json),
and [entry point](https://github.com/tsensei/OpenReels/blob/edc1c973634a3e179a527d00857b85de7b52a1dd/src/index.ts)
show MIT TypeScript/Node 22+, Remotion and its own topic-to-script/voice/visual
generation pipeline and model-provider orchestration. That is a different
product from a Rust tool for revising existing iPhone footage. No runtime claim
is made from this source review.

Its MIT repository license does not replace Remotion's separate
[license](https://github.com/remotion-dev/remotion/blob/main/LICENSE.md).
That dependency has eligibility-based free use and company licensing, unlike
an unrestricted MIT dependency. This is an additional fit issue for a generally
free/open-source foundation; no Remotion-based code is planned here.

## Evidence boundaries and licensing

The checked-in [results](../evaluation/README.md) include the command ledger,
summary, tiny generated drafts and extracted frames. The harness completed 71
commands, including intentionally failing requests. Render exit status was
supplemented with full decode, frame counts, duration measurements, source
hashes, selected pixel checks and visual frame review. This proves the small
generated cases on macOS/arm64 with FFmpeg 9.0.2, not production correctness,
performance on long footage, all iPhone input modes or any hosted-bot account.
Recorded elapsed times are diagnostic; these differing pipelines/build caches
do not make a fair comparative benchmark.

The slim Homebrew FFmpeg build lacked required caption filters. The tested
`ffmpeg-full` build supplied drawtext/libass and HDR-related filters. Presence
of a filter is still not a successful HDR test. Check the actual toolchain at
installation time rather than accepting a version string alone.

MIT covers this repository and the selected upstream source. FFmpeg builds
have their own LGPL/GPL obligations depending on configuration; see
[FFmpeg's license guidance](https://ffmpeg.org/legal.html). Model artifacts,
fonts and other redistributed assets require their own recorded licenses.
The system font used for evaluation was not copied into Git. No model API was
called. Local processing has compute/storage costs, and a hosted agent or
remote model service has its own pricing. This review does not establish a
per-video cost or a trademark clearance. The descriptive repository name is
provisional; [Reelwright](https://github.com/v0idum/reelwright) is already used.
