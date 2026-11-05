# Agent workflow

Start with [the source-preserving edit](docs/natural-edit.md). For a captioned SDR
trial, follow [the first edit](docs/first-edit.md); response
fields are in [the interface reference](docs/agent-api.md).
Read `agent-guide`, `capabilities` and `schema` first. CLI JSON and the MCP `avw`
tool share the generated typed schema. `avw request FILE|-` accepts every
operation, including those without a named CLI alias. MCP paths resolve inside
its existing workspace root. Keep sources, projects and delivery destinations
there. Inspect JSON `ok` and job states; a PID is not completion.

1. Create a project and import original media with stable
   IDs distinct across assets, sequences and items. Mutations use current
   `expectedRevision` and a unique key of 8–200 bytes.
   `import-url` accepts direct HTTPS objects only on configured download hosts;
   unchanged strong ETags allow interrupted transfers to resume. Signed URLs
   are transient and do not enter project history. Import does not change originals.
   Import a licensed TTF font when captions are requested; plain cuts need no font.
2. Inspect metadata, source frames, scenes and silence. Known unqualified SDR
   color tags refuse delivery; untagged SDR reports assumesRec709Sdr. A color
   label is not a conversion, so never relabel unsupported footage to bypass QC.
   For scene and silence inspection, check `analysisStatus`, `unavailableReason`
   and coverage before using evidence. An empty or unavailable result is not
   proof that nothing happened: stream-edge coverage is unmeasured, scene scores
   can span video gaps, and a silence end may be decoded-stream EOF. Long-GOP,
   sparse or variable-rate seeking can be unavailable; see [inspection timing](docs/inspection-timing.md).
   `analyze-start` queues a
   `frame-index`, `proxy`, `track` or configured `provider` task. Requests identify
   `assetId`; frame/proxy/tracking ranges are at most five minutes. Frame indexes
   retain original PTS; proxies are previews with explicit source maps. Analysis
   validates the request before queueing, returns evidence and never edits history.
   Poll and retrieve its verified artifact. Generate and submit scoped preview
   requests on the same service configuration; their reserved keys bind project,
   source and media backend.
   `source-evidence` pages source windows and transcript excerpts for external
   reasoning. `prepare-highlights` validates attributed candidates and returns
   previews and planning requests; it does not select stories. An explicitly
   configured `moment-search-v1` provider can be queued with `provider-search`
   and its verified suggestions read with `provider-search-results`. The
   provider receives the original source path; follow the operator's data policy.
   MCP `avw-media` can return bounded verified frames, sheets or proxy audio as
   content blocks; the host must support consuming them. See the
   [creator-agent toolkit](docs/creator-agent-toolkit.md).
3. Optionally queue `transcribe-start` when local Whisper is configured. Review
   machine text before `transcript-import`. Cues contain ID, startMs, endMs and
   literal text, plus assetId/language/provider. `transcript-search` can optionally
   find a literal phrase across adjacent cues, with revision-pinned pagination and
   playable source context where available; opt into canonical Unicode matching
   or no-space cue joining when appropriate. See [transcript search](docs/transcript-search.md).
   `studio` action `transcript-correct` updates reviewed words and selected bound
   outputs. New analysis is archived without replacing corrections. Choose a
   version explicitly with `transcript-select`; unchanged cue IDs/timing preserve
   corrections. Changed alignment requires explicit review and recomposition.
   `preview-word-alignment` and `accept-word-alignment` validate and retain
   supplied word timing and reviewer provenance without rewriting text.
   `translation-context` and `plan-translation` prepare an explicitly supplied
   translation variant with atomic provenance; neither operation translates or
   establishes speech, language, font or shaping accuracy.
4. Begin with `assemble`: outputId/name and explicit cuts with IDs, assetId,
   startMs/endMs. Keep a continuous baseline and original reaction pauses before
   proposing tighter cuts. Defaults match source dimensions/rate, preserve its
   qualified color contract, and add no captions, crop, hook or grade. High quality
   is one CRF-16 master video encode from originals; `quality:lossless` means video
   encoding only, with 48kHz stereo AAC audio. PQ/HLG masters stay 10-bit HEVC.
   VFR/mixed rates require explicit frameRate; mixed dimensions/color contracts
   need separate masters or explicit qualified SDR conversion. Cuts below 500ms,
   removal of adjacent source gaps below 250ms and source reordering require
   deliberate allowTightCuts/allowReorder flags. Read `edit-preflight` before
   rendering: actual source ranges, color/rate/canvas and speech-boundary findings
   need editorial review. Silence/transcript detection must never automatically
   justify cutting breaths, reactions or pauses. Compare independent candidates
   against the retained baseline; change one editorial dimension at a time.

   Use `plan-edit` for each candidate before committing. Supply `plan.edit`
   (the assembly), `plan.brief` (objective, natural/montage intent, default 250ms
   speech handles/1000ms context), and one `plan.decisions` reason per cut.
   Read retained/omitted ranges, actual rounded boundaries, nearby reviewed cues
   and both contextWindows, including the boundary-specific cue excerpts.
   Execute each complete previewRequest using the
   existing proxy analysis job, and listen to its verified attachment.
   No cues means unavailable speech evidence. Prefer restoring breathing room;
   proposed boundary expansions require playback and re-planning. Specific
   per-cut risk review notes are necessary for intentional speech/tight-cut/
   short-pause/reordering exceptions. Apply only a ready report with
   `apply-edit-plan` and its exact planSha256. Brief, reasons and risk notes commit
   atomically. Changed timing/policy/source/transcript makes `editPlan.status`
   stale in preflight and manifests. See [edit planning](docs/edit-planning.md).
   These are recorded decisions, not an artistic score or proof of user approval.

   For requested captions, grading, layers, music or reframing, create a separate
   `compose` SDR output from the same source cuts. Do not remove the assembly
   policy to bypass its restrictions. `compose` takes outputId/name/fontAssetId and ordered cuts with IDs, assetId,
   startMs/endMs as half-open intervals relative to the container origin.
   Defaults: 1080×1920, 30fps, font size 48. Source cues intersect
   retained ranges to create captions. Cut-boundary text needs review. If a
   nonempty cue intersection rounds to zero frames, compose refuses with the
   source, cut and cue so timing can be reviewed; see [caption timing](docs/caption-frame-timing.md).
   Captions
   wrap within 80% width, at most three lines, using owned font metrics and a dark
   stroke. Corrected, translated and restyled captions reflow; overflow refuses
   atomically. Bind a font covering every authored character.
5. Revise with a typed `studio` edit or validated `apply` batch. `describe` shows
   raw operations and properties; dry-run unfamiliar batches. Profiles and
   templates have immutable explicit versions. Apply upgrades intentionally;
   templates bind fresh source slots. `variant` clones a frozen sourceRevision
   into independent vertical, square or landscape IDs. Language variants require
   a supplied translation for every caption and preserve original speech.
6. `omit` records a whole clip and its linked captions, then ripples the timeline.
   Split partial omissions first. `restore-omission` inserts only that recorded
   source at atMs, retaining other omissions and later styling. Ripple edits that
   cross a continuous layer/transition refuse until its relationship is resolved.
   `layer` and `replace-layer` explicitly keep B-roll/image audio disabled. J/L
   cuts use independent video and dialogue clips with embedded audio disabled;
   avoid mixing the same dialogue twice. Exact source protection still applies.
7. Audio clips/buses expose gain, EQ, compression, limiting and fades. The
   `audio-duck` studio action names dialogue/music item IDs and sidechain settings.
   `audio-cleanup` can explicitly set or remove bounded spectral noise reduction
   on a composed audio item. Listen for artifacts and check A/V sync: a generated
   fixture measured about 20 ms of added onset delay. This is not voice isolation.
   Optional profile loudness/true-peak targets trigger measured two-pass processing
   and QC. `grade` adds reversible SDR brightness/exposure/contrast/saturation.
   PQ/HLG inputs receive a recorded per-source tone map before SDR compositing.
8. `track` analyzes an explicitly selected textured region in upright normalized
   source coordinates; inspect confidence and held-position findings. It is CPU
   template matching, not automatic semantic face selection. `reframe-apply`
   explicitly accepts an editable proposal into crop keyframes. Supported animated
   geometry is linear/step position and constant-size crop pans, normal blend,
   without simultaneous transitions. Unsupported channels/effect animation,
   easing, viewport resizing and basic white-balance parameters refuse delivery.
   `suggest-reframe` also creates a reviewable crop proposal from externally
   observed subject boxes. It does not perform detection or verify unsampled
   frames; check the full output and overlay visibility before accepting it.
9. `render-start` freezes a sequence at expectedRevision and returns a durable job.
   Priority is -100..100; larger values run first, with stable ties. `batch-start`
   freezes 1..32 distinct sequences in one transaction. Poll `job-status` or
   `batch-status`; retry/cancel children independently. Later edits do not change
   queued snapshots. Encoding is followed by full decode and verification.
10. `artifact` returns verified path, MIME, SHA-256 and revision; sheet retrieval
    verifies its hash too. Each render has matching SRT/VTT, cover, indexed sheet
    and manifest. `delivery` packages a succeeded render or batch into a new
    directory with verified copied files, manifest and local HTML review page.
    Its legacy manifests include full frozen project snapshots and current
    review history; inspect them before sharing. Prefer the new checked
    `batch-delivery` package when a reduced sharing manifest is appropriate.
    HDR/10-bit and lossless masters have a separately verified SDR review video.
    Use `artifact` with preview:true to retrieve it; otherwise retrieve the master.
    A normal SDR master is also its preview. The HTML plays the SDR preview and
    links to the master. Its offline composer exports frozen-identity notes and
    explicit version decisions; `review-import` validates and saves them atomically.
    Multi-output pages offer independent A/B players. Marking a range does not
    itself save a review. Preview color is not an HDR
    appearance reference.
    `compare-outputs` takes two completed queued job IDs and reports retained,
    added, omitted and uniquely reordered source ranges for plain-cut assemblies.
    Repeats are ambiguous; the report does not compare audio, color or appearance.
    See [compare edits](docs/compare-edits.md).
    Before delivery, `delivery-check` can check a caller-supplied brief against
    one frozen render's verified bytes and measured properties. `chapter-draft`
    prepares public chapter text only from explicitly selected markers and
    caller-supplied titles. Neither operation uploads or establishes acceptance;
    see [delivery checks and chapters](docs/creator-delivery-checks.md).
    `thumbnail-candidates` extracts creator-selected frames from a verified SDR
    review video; `batch-delivery` packages per-output briefs, results and
    caller-authored metadata while retaining successful siblings.
    The delivery manifest records failed batch outputs and unresolved feedback;
    the HTML presents succeeded videos. Use host file tools
    to return the bundle or MP4. Technical QC cannot approve content or appearance.
11. `review-add` anchors actor/text/time interval to an exact artifact hash and
    frozen source ranges. `reviews` remaps those ranges after edits and reports
    partial, rearranged, removed or ambiguous results; older notes retain their
    point-only mapping. Inspect the [feedback guide](docs/review-feedback.md)
    for limitations and exact placements. A source match does not establish
    visibility or a screen region. `review-resolve` records addressed/dismissed
    state without erasing the original feedback.
12. Use compact `resume` for output IDs and top-level `protectedRanges`; `status`
    returns the full project snapshot. Resume with paginated history/decisions,
    diff, job lists and
    `request-outcome`. `studio-state` supports prefix/offset/limit and summarizes
    values over 64KiB. `catalog` discovers projects read-only. Portable libraries
    include profiles/templates and fonts, without footage. OTIO interchange covers
    normal-speed cut tracks/gaps; inspect loss reports for unsupported features.
13. `backup` preserves consistent history and every historical original.
    `backup-restore` opens a verified fresh copy; derived jobs are unavailable and
    need fresh render/analysis requests with new keys. Backups exclude renders,
    analysis and delivery bundles; retain delivery separately. `verify-project` reports missing/corrupt objects; `relink` requires
    the exact retained hash. `cache-gc` defaults to a preview. Opt-in studio
    `retention` runs after workers drain; `maintain` supports scheduled collection.
    Busy project leases defer collection. Originals/history/successful artifacts
    and recoverable attempts remain retained.

Raw `apply` nests an AgentCut batch with `baseRevision`/`idempotencyKey`;
creator request envelopes use `expectedRevision`/`key`.
Same key and same intent replays the committed outcome. Changed intent needs a
new key. On revision conflicts read current diff and reconsider. Dry-run commits
no history or success outcome. `restore` appends an old whole snapshot; use
selective omission restore to retain unrelated later edits. Protected source
coverage remains enforced; `unprotect` is explicit and audited.

A restarted `avw worker /absolute/path/to/project` acquires its owner lock and marks abandoned
attempts interrupted. `job-retry` starts a fresh attempt with the same logical
job. Cancellation kills the owned process group and preserves earlier finals.
On hosts that reap detached processes, enqueue with noLaunch:true and supervise
the foreground worker. Keep the complete project on persistent local storage
with working locks; no live process is assumed to survive cloud snapshots.

Linux delivery supports composed Rec.709 SDR H.264/AAC and source-preserving
plain Rec.709/PQ/HLG cuts. Read the current acceptance matrix for version and
platform evidence. Review HDR masters on a compatible display against originals.
Compatible Dolby Vision profile 8 base layers are recognized but real camera
qualification is separate. Profile 5, unqualified wide-color transforms, HDR
compositing/grading/captions/transitions, complex script shaping and other platforms are outside this release's
support matrix. Source coverage is a temporal guarantee; inspect crops,
occlusion, speech, captions and actual appearance before approving a result.

## Browse large sources within media budgets (0.5.0)

Use `source-storyboard` jobs for cached pages of sampled frames and active
transcript excerpts; continue with `nextOffset`. Pair the verified PNG with
observed source PTS/time bases, tile coordinates and coverage metadata. MCP
`avw-media` kind `storyboard` returns both. Sparse sampling can miss important
content; request denser frames or a `budget-preview` window before choosing cuts.

`budget-preview` accepts source start/end, `maxDurationMs`, `maxBytes`, `width`
and `height`. Its verified attachment reports measured limits, encoding attempts
and original/proxy frame mappings. Impossible budgets and ambiguous clocks fail.
Use original asset IDs/source coordinates for edits; previews are SDR review
derivatives. Respect the host's supported input types and limits. See
[the source-view guide](docs/source-views.md).
