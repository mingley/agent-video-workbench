# Creator workflows for an external agent

These operations are available in the current source build through the `avw`
JSON/MCP tool and named CLI commands. The external agent chooses the story;
the workbench returns source-bound evidence, editable requests and verified
artifacts. A proposal or an automated technical check is not creator approval.

## Find and plan clips

Call `source-evidence` with a project, `expectedRevision` and a query such as
`{"assetId":"source","windowMs":30000,"limit":5}`. Read its scoped source
windows, transcript excerpts, hashes, truncation flags and `nextOffset`.
Execute each returned `previewRequest` unchanged through `analyze-start`, run
the worker, and retrieve its verified artifact. An unavailable transcript or a
window without cues does not establish that nothing useful occurred.

An external agent can inspect those windows and submit `prepare-highlights`
with a brief containing an objective and explicit candidate ranges, source
SHA-256 values, titles, reasons and evidence. The report attributes visual or
audio observations and confidence to the caller, checks transcript cue IDs
against the selected transcript, and returns source proxies and `plan-edit`
requests when available. Review the previews and planning findings before
applying a plan. Neither operation ranks or interprets media by itself.

For semantic or visual analysis, an operator may configure an existing
`--analysis-provider` executable that implements `moment-search-v1`.
`provider-search` returns a source-, provider- and revision-bound
`analysisRequest`; it runs only when explicitly submitted. The provider
receives the original source path and query under the documented provider
contract, and controls any network transfer. After the job succeeds,
`provider-search-results` checks the frozen job and output hashes, validates
candidate ranges and provenance, and returns reviewable plans. An empty result
is not proof that no matching moment exists. The workbench does not bundle a
semantic model; review provider findings and their cost/privacy policy.
Keys beginning `avw-search-v1-` are reserved for these generated requests and
bind the provider, backend, source and revision. Regenerate the request after
changing service configuration. The provider's normal analysis result must
have the same `sourceSha256` and contain data in this form:

```json
{
  "schemaVersion": 1,
  "kind": "analysis",
  "sourceSha256": "SOURCE_SHA256",
  "data": {
    "schemaVersion": 1,
    "query": "the requested question",
    "candidates": [{
      "id": "candidate1", "startMs": 1000, "endMs": 8000,
      "title": "A possible moment", "reason": "Provider-supplied reason",
      "confidence": 0.8,
      "evidence": [{"kind":"observation","modality":"video","description":"Provider observation"}]
    }]
  }
}
```

Evidence can instead name an active transcript cue with `kind:
"transcript-cue"`, `transcriptSha256` and `cueId`. Optional `targetDurationMs`
is a planning target, not an enforced export limit. Configure and evaluate the
provider separately; the workbench checks the returned structure and provenance.

## Send evidence to an agent

The MCP `avw-media` tool accepts `kind: "frame"` with project, asset ID and
`atMs`; `kind: "contact-sheet"` with project and render `jobId`; or
`kind: "audio-proxy"` with project, source-proxy `jobId`, `startMs` and
`durationMs`. It returns verified PNG image or WAV audio content in addition
to structured metadata. Images are capped at 2 MiB; audio excerpts are at
most 15 seconds. Audio requires MCP 2025-03-26 or later. Whether the agent can
consume these blocks depends on its host.

`agent-media` exposes the same bounded operation as JSON, with the content
base64 encoded. The CLI requires `--workspace ROOT` and reads a media request
object from `--request FILE`. Neither route accepts arbitrary file paths.
Inspection frames are SDR previews; their requested seek time is not an
independently measured decoded-frame timestamp.

## Review and revise

The offline delivery page can collect timestamped notes and an explicit
approve/request-changes decision for a particular frozen render. The creator
copies or downloads a versioned draft. Send it to `review-import` with the
current editing `expectedRevision`, a unique idempotency key and optional
`dryRun`; the importer verifies the frozen artifact and commits the draft
atomically. Saved decisions remain attached to that render, not a later version.
The `reviews` response includes recorded decisions for the selected sequence.
If timing evidence is unavailable, range notes require a new render; a
decision can still be recorded for a verified artifact.

## Words, translation and reframing

`preview-word-alignment` checks caller-supplied word spans against the verified
original and the selected corrected transcript. After listening, submit
`accept-word-alignment` with a reviewer and note. Accepted words and speaker
labels are paginated by `word-alignment`, with current/stale status. Offsets
refer to UTF-8 bytes in each corrected cue; selected cues must be fully covered
and source timings must not overlap. No speech recognition or alignment
accuracy is inferred, and existing text and captions are preserved.

`translation-context` returns revision-pinned caption text and hashes.
`plan-translation` checks an explicit caller/provider-supplied translation for
every caption, including hidden ones, and returns a dry-run variant request.
The accepted variant retains the attributable proposal and source bindings;
it does not establish translation quality, font coverage, shaping or appearance.

`suggest-reframe` accepts observed normalized upright-source subject boxes for
an item, source hash and sampled source times. It computes an editable crop
path when every supplied sample fits the requested safe inset; missing, sparse,
low-confidence or too-wide subjects return issues instead of an edit. Apply its
`edit` only after review through `studio` with the expected revision and a new
key. Subject detection is supplied by the agent/provider; unsampled frames,
moving subjects, overlays and platform controls still require playback review.
The proposal refuses existing transforms, crops, active effects, keyframes,
touching transitions and non-square canvas pixels where subject-fit geometry
would be uncertain.

## Package a batch

`batch-delivery-check` applies a versioned caller brief independently to each
frozen output. `batch-delivery` creates a new checked package containing the
per-output reports, caller-authored title/description/tags and successful
review packages. Failed checks and unavailable siblings remain explicit; a
complete package can contain outputs that failed their requested constraints.
The report does not establish platform acceptance or caption accuracy. The
destination must be new and the Linux atomic publication path refuses to
replace an existing directory.

The new batch package omits full project snapshots, saved reviews, edit
objectives and cut notes. It retains selected project, sequence and artifact
identities, media, captions, contact sheets and caller-authored metadata;
inspect these before sharing because their contents can still be sensitive. The older `delivery`
command includes a full frozen project snapshot and may also include saved
reviews and unrelated project information.

On Unix, batch staging and the completed directory are owner-only. An
interrupted process can leave a local `.avw-batch-*.incomplete` directory with
unsanitized intermediate files; delete it when it is no longer needed. The
minimized sharing boundary applies to the completed destination.

`thumbnail-candidates` accepts a completed queued render job, a new
destination and `{"schemaVersion":1,"atMs":[1000,2500]}`. It extracts up to
eight creator-selected frames from the verified SDR review video, verifies
their decoded timestamps and writes PNGs with a manifest. Duplicate frame
selections, unknown timing and changed artifacts are refused. These images
are scaled review previews; their appearance can differ from an HDR master.
The caller selects the moments; the tool does not rank thumbnails.

For noisy audio on a composed output, `audio-cleanup` accepts an explicit
`set` edit with sequence, item ID and bounded `noiseReductionDb` and
`noiseFloorDb` settings. It stores a reversible per-item setting and applies
FFmpeg spectral noise reduction after clip/track gain, pan and fades and before
mixing; the noise-floor setting refers to that post-gain signal. A `remove`
edit removes it.
Listen to the result and check synchronization: noise reduction can alter
speech, transients and timing and does not prove improved quality. A generated
tone fixture measured about 20 ms of added onset delay; that is not a latency
guarantee for other recordings. Source-preserving assemblies and nested
cleanup are refused.
