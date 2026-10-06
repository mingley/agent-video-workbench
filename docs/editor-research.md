# Lessons from established editors

Official documentation reviewed October 5, 2026. This is documentation research
and executable FFmpeg testing, not a claim that Resolve or Kdenlive was installed
and compared against private footage.

| Source | Relevant behavior | Workbench decision |
| --- | --- | --- |
| [Kdenlive project settings](https://docs.kdenlive.org/en/project_and_asset_management/project_settings/general_settings.html) | Project profiles make resolution, aspect ratio, frame rate and color space explicit | `assemble` derives upright source dimensions/rate; preflight reports the actual output contract |
| [Kdenlive rendering](https://docs.kdenlive.org/en/exporting/render.html) | Proxy/render-preview and rescale choices are distinct from normal delivery; quality profiles are explicit | Encode the master directly from originals; generate a separate labelled SDR review preview; offer high-quality and lossless video choices |
| [DaVinci Resolve color](https://www.blackmagicdesign.com/products/davinciresolve/color) | Wide-gamut/HDR color management, high-precision processing and HDR delivery are explicit capabilities | Preserve native 10-bit YUV for plain PQ/HLG cuts; declare SDR conversion when composing. Do not claim Resolve-class HDR grading/compositing |
| [FFmpeg filters](https://ffmpeg.org/ffmpeg-filters.html) | Overlay pixel format, color-space conversion and linear-light tone mapping are configurable | Keep SDR composition in RGB until one explicit Rec.709 conversion; bypass that compositor entirely for source-preserving cuts |

The reported damage came from several independent decisions: trimming pauses,
adding dominant text, tone mapping HDR, an unintended composite color conversion
and an extra lossy working encode. A single quality slider cannot address all
of those. The existing `820e1e5` fixes keep the SDR composite in RGB and bound
input decoder threads. The new preserving path keeps the original color
contract, raises master encode quality and gives the agent an inspectable,
restrained baseline before styling.

The agent workflow is consequently: inspect the recording, retain a continuous
baseline, create an independent candidate, read preflight, review source handles
and speech across cuts, then freeze and verify delivery. Tiny cuts and removed
short pauses require explicit intent; transcript/silence analysis never edits
the timeline automatically. Technical QC and editorial approval remain distinct.

Further capability work needs representative camera samples and display review,
especially dynamic HDR metadata, unusual color profiles and HDR overlays. A
managed wide-gamut compositor is a separate project; preserving plain cuts now
avoids forcing recordings through an 8-bit SDR pipeline while that work remains.

## Source context and paper edits, October 6, 2026

The following current official pages were fetched and read. Premiere's Paper
Edit page was updated September 9, 2026, and Trim mode August 18, 2026. These
are documentation observations, not benchmark results from installed editors.

| Source | Observed workflow | Applied capability |
| --- | --- | --- |
| [Premiere Paper Edit](https://helpx.adobe.com/premiere/desktop/edit-projects/edit-video-using-text-based-editing/create-a-sequence-with-paper-edit.html) | Select source transcript passages, inspect selected duration, preview before creating a new sequence | Read-only `plan-edit`, retained/excluded cue excerpts, duration comparison and separate candidate creation |
| [Premiere Trim mode](https://helpx.adobe.com/premiere/desktop/edit-projects/trim-clips/about-trim-mode.html) | Inspect outgoing/incoming frames and play or step around the edit | Exact boundary coordinates and bounded playable source-context requests for both sides, using verified existing proxy jobs |
| [Premiere pause deletion](https://helpx.adobe.com/premiere/desktop/edit-projects/edit-video-using-text-based-editing/detect-and-delete-pauses-in-transcripts.html) | Explicit filtering and individual or bulk deletion of transcript pauses/fillers | Keep detection separate from selection; list excluded silent material and require reasons rather than automatically deleting all detected pauses |
| [Resolve editing](https://www.blackmagicdesign.com/products/davinciresolve/edit) | Source/timeline viewers, trim tools and source browsing keep shot selection close to refinement | Preserve source coordinates and return context beside the proposed cut, rather than forcing the agent to judge a transcript fragment alone |
| [Final Cut Pro](https://www.apple.com/final-cut-pro/) | Magnetic Timeline supports experimenting with edits while retaining synchronization; Transcript Search helps find spoken material | Independent candidates, existing atomic edit/history and frozen A/V rendering, with durable per-cut reasons and transcript-sensitive review invalidation |
| [Kdenlive editing](https://docs.kdenlive.org/en/cutting_and_assembling/editing.html) | Source/target zones and three-point editing separate selected source from timeline placement | Keep requested source ranges and actual frame-rounded output ranges visible together |

The resulting [planning workflow](edit-planning.md) records the objective and
individual cut decisions, defaults to 250ms speech handles, and refuses applying
an unreviewed speech-boundary/handle or deliberate pacing exception. Proposed
expansions are evidence for playback, never automatic editorial decisions.
Plan digests bind the exact base snapshot; later timing/source/transcript changes
mark prior review stale. This emulates useful workflow mechanics, not the full
GUI, AI features, optical flow, HDR compositor or artistic judgment of these tools.
