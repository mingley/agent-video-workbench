# Check a delivery and draft chapters

These read-only operations use a completed queued `render-start` job and its
frozen project revision. They do not change the project, repair old exports,
upload anything, or establish platform acceptance. If an older synchronous
`render` lacks a job-bound sequence, render the selected revision through
`render-start` before using these operations.

## Check a creator's brief

Put only the constraints the creator actually requested in `brief.json`:

```json
{
  "schemaVersion": 1,
  "dimensions": {"width": 1080, "height": 1920},
  "aspectRatio": {"width": 9, "height": 16},
  "maxDurationMs": 45000,
  "maxBytes": 100000000,
  "requiredSidecars": ["srt", "vtt"],
  "color": "sdr"
}
```

```sh
avw delivery-check project "$RENDER_JOB_ID" --request brief.json
```

The equivalent JSON/MCP request is `{"command":"delivery-check",
"project":"project","id":"...","brief":{...}}`. All constraints are optional
individually, but at least one is required. `color` accepts `sdr`, `hdr-pq` or
`hdr-hlg`. `maxBytes` applies to the master file, not the combined package;
`maxDurationMs` checks fresh-probed container duration and the duration and end
relative to container start of each audio, video and subtitle stream. Measurements use
exact decimal or rational arithmetic; missing metadata returns `unknown` unless
another measurement already fails. A stream starting before the declared
container start also makes an otherwise passing duration result `unknown`.
These are metadata, not a timed playback
measurement. Dimensions are the display-oriented raster; aspect also accounts
for sample aspect ratio. Ambiguous transforms or missing metadata return
`unknown`. Color checks only the video's tags.
For this check, `sdr` means BT.709 primaries, transfer and matrix; `hdr-pq`
and `hdr-hlg` mean BT.2020 primaries and nonconstant-luminance matrix with the
respective PQ or HLG transfer. Other SDR or HDR tag combinations do not match
these specific intents.

Each requested constraint has `pass`, `fail` or `unknown` and evidence. The
overall status is `fail` if any check failed, otherwise `unknown` if any is
unknown, otherwise `pass`. The tool rehashes the master and freshly probes it
when metadata-dependent constraints are requested;
required sidecars must match their stored hashes and reconstruct from nonempty
frozen caption cues. Older sidecars whose bytes do not match the current
reconstruction are `unknown`. A passing sidecar check does not establish that
speech and meaningful sounds are fully or accurately captioned. Caption text
and timing still need human review. The manifest itself is not independently
authenticated, and color tags do not prove appearance. If the probe is
unavailable, metadata-dependent checks are `unknown` while independent byte
and sidecar checks can still be assessed.

The report describes one artifact and is returned to the caller; it is not
automatically stored in a delivery package or applied to an entire batch. Run
it for each chosen render and retain the reports alongside the creator's brief.
Platform requirements can vary by account and change over time; check them at
the destination instead of interpreting a pass as an upload guarantee.

## Draft chapters from selected markers

Add markers to the sequence before queuing the render. Choose which markers
to publish and write a reviewed public title for each in `chapters.json`:

```json
[
  {"markerId":"intro", "title":"Introduction"},
  {"markerId":"method", "title":"How it works"},
  {"markerId":"takeaway", "title":"What to try next"}
]
```

```sh
avw chapter-draft project "$RENDER_JOB_ID" --request chapters.json
```

The equivalent request has `command: "chapter-draft"`, `project`, render job
`id`, and the array as `selections`. The report contains copyable `text`, the
chosen marker IDs and exact frozen times, whole-second timestamps rounded down,
and a warning when rounding occurred. Caller order is retained; titles must be
single-line text without control or bidirectional formatting characters.
Unselected markers, marker names and private comments are not included in the
chapter list.

`youtube.documentedTimestampChecks` reports the documented structural checks:
a first timestamp of `00:00`, at least three timestamps in ascending order, and
at least ten seconds per chapter including the last. It checks the rounded
timestamps against the recorded decoded video duration, reports collisions or
reordering, and leaves account/video eligibility `unknown`. The recorded stream
duration must agree exactly with the frozen output frame count and frame rate;
missing or inconsistent evidence is refused. See the current [YouTube chapter
guidance](https://support.google.com/youtube/answer/9884579?hl=en) before use.
Review the draft and resulting boundaries; no title is generated automatically.
