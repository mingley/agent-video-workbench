# Compare two frozen edits

Use a current 0.4.0 source build to compare the source footage in two completed,
queued renders. This helps an agent explain which portions of a source were
kept, added, omitted or reordered between a baseline and a candidate. Render
each source-preserving assembly with `render-start`, run the worker, and wait
for both jobs to succeed. Supply the two **job IDs**, not output IDs or the
attempt-specific `artifactId`:

```sh
avw compare-outputs project BASELINE_JOB_ID CANDIDATE_JOB_ID
```

The equivalent CLI JSON or MCP request is:

```json
{
  "command": "compare-outputs",
  "project": "project",
  "baselineJob": "BASELINE_JOB_ID",
  "candidateJob": "CANDIDATE_JOB_ID"
}
```

The result identifies both frozen revisions, output names, canvas/frame rate,
verified master and review file paths, SHA-256 values and sizes. `retained`,
`added` and `omitted` contain half-open source intervals with exact rational
`sourceStart`/`sourceEnd` and the corresponding `outputStart`/`outputEnd`
placements in each render. Start/end times are source and output coordinates,
not guaranteed individual frame identities when frame rates differ or a source
is variable-frame-rate. The review file can be played at those output times;
retrieve it with `artifact` using the named job ID to recheck it later.

Source identity combines the stable asset ID and its frozen SHA-256. Reusing an
ID for different bytes is reported as an omission plus an addition; identical
bytes imported under different asset IDs also remain distinct. Each source
interval is partitioned at all cut boundaries. `baselineCount` and
`candidateCount` show how many placements cover the interval. An interval
present on only one side has `mapping: "unpaired"`; a retained interval with
multiple placements has `mapping: "ambiguous"` and lists every placement
without inventing a pairing. For uniquely placed retained intervals,
`reordered: true` identifies a proven change in relative order. `false` means
no reorder was found among uniquely placed retained intervals; `null` means
there is no pair or repeated material makes the negative conclusion uncertain.
A simple shift after an earlier cut is removed is not by itself a reorder.

`status: "complete"` means the supported **source-video ranges** were mapped;
it does not mean the outputs are identical. Audio, color, encoding quality,
framing, cadence, effects, pixels, captions and appearance are not compared.
The first implementation maps only source-preserving plain-cut assemblies.
Layered or composed outputs, transitions, speed changes, nested material and
other unsupported structures return `status: "unsupported"` with reasons,
not an empty assertion that nothing changed. A synchronous `render` job also
returns unsupported: its stored job record does not independently bind the
selected sequence, so use `render-start` and a worker for a new comparison.

The command rehashes each master and any separate SDR review file and compares
their bytes with the hashes in the render manifest. It also checks the frozen
snapshot against project revision history and the queued render's recorded
sequence and attempt. **The manifest is not independently authenticated**:
changing both a file and its hash in the manifest cannot be detected by this
check. It is not a signature or proof of artistic approval. Reading large video
files can take time. A comparison is limited to 100 cuts per output and 4,000
interval placements; a larger result is refused rather than truncated.

Comparison creates no edit, render job, or media file and does not change the
project head. Opening an older project still follows the normal store migration
rules. Review actual video and listen to the original before deciding whether
an edit is good.
