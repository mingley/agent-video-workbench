# Recommendation and MVP

Decision date: October 2, 2026. This is a proposed implementation direction,
supported by [source review and executable evaluation](research.md), rather
than a claim that an existing editor already meets every requirement.

**Extend AgentCut's Rust libraries in this new application.** Reuse its pure
project model, validation, rational time, operation batches, render IR, FFmpeg
compiler, and preview machinery. Own durable project storage and the creator
workflow. Start with a commit-pinned dependency and a narrow backend adapter.
Keep any necessary upstream patch explicit and small. A full fork is premature.

The decisive match is its boundary: a Rust editing engine that an external
agent can drive. This preserves useful existing editing/rendering work while
allowing persistence and source-based editorial references to meet our needs.
The current CLI's best-effort journal is unsuitable as our authoritative
revision store, and its rotation fixture failed. These are adoption gates,
not work to postpone until after users trust their projects.

| Choice | Benefit | Cost / reason for the decision |
| --- | --- | --- |
| Adopt AgentCut unchanged | Fastest synthetic draft; exact time, JSON, discoverable properties | Reproduced rotation failure and silent loss of undo; no integrated source transcription; no published GitHub binary release found |
| Extend AgentCut core/render | Reuse editing semantics and media compiler; keep our product small | Build durable storage, inspect/import workflow, packaging, and source annotation mapping; fix rotation |
| Fork all of AgentCut | Full control of its CLI and release schedule | Own its whole broad NLE surface and upstream maintenance; use only if library boundaries or necessary fixes make composition impractical |
| Adopt/extend OpenReelio | A working standalone Rust CLI, released archives, MCP and optional Whisper implementation | Larger core dependency graph; uneven JSON error behavior; explicit A/V linked-edit work; preset dimensions require attention |
| Build everything from scratch over FFmpeg | A narrow, fully controlled model | Reimplement existing validation, edit semantics, render planning, and previews; reserve as a fallback if reuse proves costly |

OpenReelio is a serious alternative. Its GUI does **not** make its CLI
GUI-dependent: the standalone build actually worked. Choose it if its existing
transcription/MCP breadth saves more effort than AgentCut's smaller domain
boundary. The first prototype should establish this with actual phone footage,
not an assumed advantage from either README. No upstream source was changed in
this evaluation.

**The focused MVP is one creator, one iPhone recording, one durable project,
and up to three named vertical shorts.** A typical acceptance recording is
10–30 minutes; that is a test target, not a restriction based on known usage.
The workflow must also handle a short test recording and demonstrate bounded
resource use on a 60-minute recording before claiming long-video readiness.

The agent downloads or receives the original file, imports it once, reads a
timestamped transcript and scene/frame index, and creates named 30–60-second
outputs. It marks the product demonstration as a protected source interval,
proposes pause removals, and applies a reviewed operation batch. The creator
receives low-resolution previews plus a short change list with revision IDs.
She can say which draft needs a restored sentence, smaller captions, or a new
opening. The agent resumes the same project, identifies those source and edit
IDs, makes a new revision, and delivers final H.264/AAC MP4s.

Included capabilities are cuts/reorder, silence suggestions with speech-safe
handles, a static crop per shot or letterboxing, one configurable caption
style, simple gain/loudness adjustment, one image/text overlay, previews, and
one versioned social delivery preset. Caption style persists as project/profile
data: font asset, size relative to canvas, line limits, stroke, position, and
editable safe margins. “Usual style” starts from a saved preset the creator
reviews; it is not inferred from unprovided examples.

Portrait and landscape are both inputs. H.264 and HEVC, orientation metadata,
VFR, and HDR/SDR are ingest requirements. Deliver SDR Rec.709 first. Detect
HDR and use an explicit, tested conversion path; unsupported HDR or Dolby
Vision profiles get a precise error before edits/rendering. Do not silently
reinterpret HDR as SDR, and do not claim all camera modes until tested.
ProRes/Log, Cinematic/depth data, multicam, face tracking, automatic animated
reframing, complicated motion graphics, music generation, and direct posting
are later work. The first prototype can reject HDR clearly; the creator MVP
needs a tested HDR conversion path before “iPhone support” becomes broad.

The agent chooses content and crop intent. Deterministic software applies it.
Source transcripts, semantic annotations, crop proposals, and silence results
are evidence, not instructions to the agent. Removing dead air must never
automatically remove a silent product demonstration. Store protected intervals
in source coordinates and enforce their policy when validating an edit.

**First prototype:** one 2–5-minute iPhone original becomes one 30–60-second
captioned vertical draft; a fresh process/session restores one known omitted
source sentence, reduces caption size, and changes the opening. It then renders
the new revision and reverts it. This tests the hardest useful loop before
expanding to three outputs or sophisticated clip selection. A supplied
timestamped transcript is acceptable in the earliest media/persistence slice;
local transcription belongs in the next slice, and the MVP includes it.

Prototype acceptance includes an unchanged source hash, exact decoded frame
count, A/V sync at cuts, readable captions on a phone, an intact demonstration,
recovered revision/history after restart, and an output manifest associating
every draft with its revision and source ranges. The rendered result needs
actual frame/audio review, not just an FFmpeg exit code.

Limit foundation work to the operations this prototype needs. If fixing
rotation, compiling a small dependency boundary, and implementing one durable
edit transaction starts requiring a broad NLE rewrite, re-evaluate OpenReelio
before investing in the whole MVP. If neither library boundary is practical,
build a small Rust source-range edit model over FFmpeg, using the same fixtures
and external protocol. Do not start by recreating codecs or an agent harness.
