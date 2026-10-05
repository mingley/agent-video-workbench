# Preserve the recording before styling an edit

This workflow requires **0.4.0 from main**. The published 0.3.1 binary has the
[captioned SDR workflow](first-edit.md), but does not have `assemble` or
`edit-preflight`. Build with latest stable Rust using `cargo build --release
--locked`, then use `target/release/avw`. Configure the FFmpeg/ffprobe paths as in
[deployment](deployment.md). CLI JSON and MCP accept the same requests below.

Start by retaining one continuous take. Compare its framing, pacing, speech and
color with the original before removing anything. Silence detection and
transcript boundaries provide evidence; they do not identify dispensable
breaths, reactions or comedic pauses. Read [the editor research](editor-research.md)
for the design decisions behind these defaults.

## Create a source-matched baseline

Keep the input and project inside your CLI workspace or MCP root. With a tagged
Rec.709, 10-bit PQ or 10-bit HLG video at least ten seconds long:

```sh
avw doctor
avw create project --name "Natural edit"
avw import project inputs/source.mp4 --id source \
  --expected-revision 0 --key natural-source-import
```

Check `ok` after every call. For HDR preservation, `doctor` must also report
`result.sourcePreservingHdr.ready: true`; ordinary SDR readiness alone is
insufficient. Read `resume` for existing projects rather than reusing the revision
numbers here. No font or transcript is required for a plain edit.

Send this complete envelope with `avw request FILE` or as MCP `avw` arguments:

```json
{
  "command": "assemble",
  "project": "project",
  "expectedRevision": 1,
  "key": "natural-baseline-assemble",
  "edit": {
    "outputId": "natural",
    "name": "Original timing baseline",
    "cuts": [
      {"id": "take", "assetId": "source", "startMs": 0, "endMs": 10000}
    ]
  }
}
```

`assemble` defaults to `color: "preserve"`, `quality: "high"`, upright source
dimensions and source frame rate. It adds no captions, hook, grade or crop.
Source starts retain exact millisecond coordinates relative to the container
origin. Durations round to the output frame grid; `edit-preflight` reports the
actual source and output ranges. `assemble --request FILE` instead reads just
the inner `edit` object, with revision and key provided as CLI arguments.

The default requires chronological source ranges, cuts at least 500ms long and
no removal of adjacent source gaps shorter than 250ms. These are guardrails,
not an aesthetic scoring system. Set `allowReorder: true` or
`allowTightCuts: true` when that edit is deliberate and reviewed. Flags do not
waive source bounds or rendering restrictions. Multiple outputs are independent
sequences with distinct IDs; create a new candidate while retaining the baseline.

## Inspect, freeze and retrieve

```sh
avw edit-preflight project --sequence natural
avw render-start project --sequence natural \
  --expected-revision 2 --key natural-baseline-render --no-launch
avw worker /absolute/workspace/project
avw job-status project JOB_ID
avw artifact project JOB_ID
avw artifact project JOB_ID --preview
avw delivery project JOB_ID /absolute/workspace/new-review
```

Use the returned job ID. Require `state: "succeeded"` and verified artifact
retrieval. Preflight declares color, dimensions, frame rate, source ranges and
editorial findings. Reviewed transcripts can expose cuts through speech cues;
listen across those boundaries. A render manifest records the same decisions,
source hashes, full decoded frame count, color/bit-depth QC and resource policy.
Later edits leave the queued revision intact.

`artifact` retrieves the master; `preview: true` in JSON or `--preview` retrieves
the hash-verified SDR review video where a separate preview is needed. For an
ordinary SDR master, preview retrieval returns that master. `sheet` and `preview`
are mutually exclusive. The HTML bundle plays the review video and links to the
master. Covers and contact sheets use the SDR review rendering, too.

## Choose delivery intentionally

| Intent | Request and result |
| --- | --- |
| Keep source color and framing | `assemble`, default `color: "preserve"`; tagged limited-range Rec.709 stays Rec.709, 10-bit BT.2020 PQ/HLG becomes 10-bit HEVC with its transfer and static mastering metadata |
| Compatible SDR delivery | `assemble` with `color: "sdr"`; explicit HDR tone map, H.264/Rec.709 output, original dimensions and timing choices |
| Audit video precision | `quality: "lossless"`; lossless video encoding, potentially large files, plus a regular H.264 review preview |
| Captions, grade, crops, music, layers or transitions | Create a separate `compose` output from the selected source cuts; review its explicit SDR, aspect and caption choices |

High quality uses CRF 16 in one master video encode directly from originals.
Lossless describes **video encoding**, not audio or a byte-identical media file.
Audio is encoded once as 48kHz stereo AAC at 256kbps; initial gaps are padded
without shifting source audio. Orientation/SAR normalization and explicit frame
rate changes can alter the raster/cadence even with lossless encoding. A preview
is derived from the master and encoded separately; it is not a working source
or a color reference for judging HDR.

Preservation supports 1..100 contiguous normal-speed cuts, an even canvas no
larger than 4096 per axis, 1..120fps and at most one hour. Mixed dimensions,
mixed color/bit-depth contracts or mixed HDR static metadata require separate
masters or a qualified explicit SDR conversion. VFR or mixed rates require
an explicit rational `frameRate`, such as `{"numerator":30,"denominator":1}`;
preflight records that CFR conversion can repeat/drop frames.

Known SDR primaries/transfers outside the qualified Rec.709 path, including
Display P3 SDR, refuse delivery; assigning a Rec.709 tag is not a gamut/transfer
conversion. Untagged SDR may use the older assumed-Rec.709 route, and its
`assumesRec709Sdr` finding is explicit. Unknown color tags are not a basis for claiming preservation. Inspect them and
choose an explicit qualified SDR route if appropriate. Dolby Vision profile 8
can use its HDR10/HLG compatible base layer; proprietary dynamic metadata is
not preserved or qualified. Profile 5, Log and other unqualified wide-color
transforms refuse. HDR compositing, grades, captions and transitions are not
implemented: adding those to a preserving sequence refuses rather than silently
ignoring them. Use an independent SDR composition for those features.

Generated pixel, timing, memory, SDK and browser tests are recorded in
[implementation status](implementation-status.md). They establish technical
behavior, not natural pacing or matching a particular phone/display. Compare
the HDR master on a compatible display with the original and review every cut
before approving the edit.
