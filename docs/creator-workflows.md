# Creator workflows with an editing agent

These examples use the current 0.4.0 source build through CLI JSON or the MCP
`avw` tool. The agent chooses the story with the creator; the workbench supplies
source evidence, editable cuts, frozen renders and review history. Search results
and technical checks do not decide whether a moment is compelling or approved.

The connected examples below assume a project named `project` at revision 2,
with one imported video asset `source` at least ten seconds long and a reviewed
transcript already attached. They also assume its color, dimensions and frame
rate are supported by source-preserving `assemble`; inspect the original and
[preflight](natural-edit.md) first. Use the creator's actual footage, words and
timestamps in place of the illustrative phrase and cuts. Importing the original
and the reviewed transcript produces revisions 1 and 2 in a fresh project;
otherwise, read `resume` and substitute the current revision and unused IDs.

Each JSON block marked **request** is a complete envelope: send it as MCP `avw`
arguments, or save it in a file and run `avw --workspace ROOT request FILE`.
Set the same FFmpeg and ffprobe configuration for planning and preview jobs.
Check `ok` after every call. Analysis and rendering do not advance the editing
revision, while applying an edit or recording feedback does.

## 1. Find and listen to a possible opening

A creator asks, “Find where I explain why we changed the packaging.” Search for
actual words from their reviewed transcript; this example finds a literal phrase,
including one split across neighboring cues:

**Request:**

```json
{
  "command": "transcript-search",
  "project": "project",
  "assetId": "source",
  "query": "changed the packaging",
  "acrossCues": true,
  "expectedRevision": 2,
  "limit": 20
}
```

Read the matching cue IDs, whole-cue source times and nearby text. If a result
has `previewRequest`, send that object unchanged on the same configured service,
run the worker, then retrieve its verified `artifact` and listen to the attached
proxy. A `previewUnavailableReason` means the request could not be offered;
inspect the original or request a suitable in-bounds proxy instead. If there is
another page, pass its `nextOffset` with the returned `revision` as
`expectedRevision`. See [transcript search](transcript-search.md).

Listen for the setup, the actual point and its natural ending. A strong opening
may need a preceding sentence or a reaction; a transcript hit is a lead, not a
complete clip. Search is literal, limited to eight nonoverlapping cues and cannot
infer meaning or exact word boundaries. Missing or inaccurate transcript text
is not evidence that the original lacks the moment.

## 2. Keep a baseline and make a reviewed candidate

Retain a continuous comparison before removing a reset or a false start. This
example creates `natural` from the first ten seconds; it advances revision 2 to
3. Do not assume those seconds are the right story in another recording.

**Request:**

```json
{
  "command": "assemble",
  "project": "project",
  "expectedRevision": 2,
  "key": "keep-natural-baseline",
  "edit": {
    "outputId": "natural",
    "name": "Continuous source baseline",
    "cuts": [{"id": "take", "assetId": "source", "startMs": 0, "endMs": 10000}]
  }
}
```

For a separate candidate, save the following **inner plan** as
`candidate-plan.json`. Its one-second omission represents a reset only if the
creator and agent actually review that part of the source.

```json
{
  "brief": {"objective": "Keep the complete explanation and reaction; remove the reviewed reset", "pacing": "natural"},
  "edit": {
    "outputId": "candidate",
    "name": "Reviewed opening",
    "cuts": [
      {"id": "opening", "assetId": "source", "startMs": 0, "endMs": 4500},
      {"id": "ending", "assetId": "source", "startMs": 5500, "endMs": 10000}
    ]
  },
  "decisions": [
    {"cutId": "opening", "reason": "Keep the complete opening and reaction"},
    {"cutId": "ending", "reason": "Resume after the reviewed reset and retain the ending"}
  ]
}
```

Run `avw --workspace ROOT plan-edit project --request candidate-plan.json
--expected-revision 3`, or send an MCP `plan-edit` request with that object as
`plan`. Review `omittedSourceRanges`, findings and both `contextWindows` for
each cut. Play the returned context requests and listen across each boundary.
Check whether the omission changes the claim, clips a word or removes a useful
pause, movement or reaction. Correct the ranges and re-plan when needed; a
specific review note is required for each intentional flagged risk. Do not
invent a review note merely to make a plan pass.

Only when `readyToApply` is true, apply the exact plan and its returned SHA:

```sh
avw --workspace ROOT apply-edit-plan project --request candidate-plan.json \
  --expected-revision 3 --key apply-reviewed-candidate \
  --plan-sha256 SHA_FROM_READY_REPORT --dry-run
avw --workspace ROOT apply-edit-plan project --request candidate-plan.json \
  --expected-revision 3 --key apply-reviewed-candidate \
  --plan-sha256 SHA_FROM_READY_REPORT
```

The first command makes no change; the second creates `candidate` at revision
4. A revision conflict or a changed plan requires a fresh report. The baseline,
source and reasons remain available. Check `edit-preflight` for the actual
frame-rounded timing and review state before rendering. This plain-cut workflow
adds no captions or automatic reframing; use a separate SDR composition when
those are requested. See [edit planning](edit-planning.md).

## 3. Render, compare and share the review

Freeze both outputs at revision 4 so the creator can compare the continuous
baseline with the candidate. Set `noLaunch` when a foreground worker is needed.

**Request:**

```json
{
  "command": "batch-start",
  "project": "project",
  "sequences": ["natural", "candidate"],
  "expectedRevision": 4,
  "key": "render-baseline-and-candidate",
  "noLaunch": true
}
```

Save `result.id` as the batch ID. Run `avw worker /absolute/path/to/ROOT/project`
and poll `{"command":"batch-status","project":"project","id":"BATCH_ID"}`.
Wait for `complete: true`, inspect every child state and retry failed children
independently if appropriate. For each succeeded child, use its `id` with
`artifact` and require `verified: true`; use `preview: true` for the SDR review
video when a separate preview exists. The child's `input.sequence` identifies
which named output it rendered. Later project edits do not change the frozen
render.

For an optional source-time comparison, use the `id` of each succeeded child:

**Request:**

```json
{
  "command": "compare-outputs",
  "project": "project",
  "baselineJob": "NATURAL_CHILD_JOB_ID",
  "candidateJob": "CANDIDATE_CHILD_JOB_ID"
}
```

Check `status` and inspect `retained`, `added`, `omitted` and any ambiguous or
reordered placements. This read-only report compares verified queued renders of
source-preserving plain cuts; an unsupported output is reported with its
limitations. The report is temporal evidence, not a judgment of the story,
visual appearance or audio. Watch and listen to both outputs as well. See
[compare edits](compare-edits.md).

Package the succeeded batch into a new destination inside the workspace:

**Request:**

```json
{"command":"delivery","project":"project","id":"BATCH_ID","destination":"review-set"}
```

Inspect the returned manifest for failed or missing outputs before sharing the
bundle through the host's file tools. Its `index.html` presents successful videos
and lets a creator copy a render ID, revision and playback time into feedback.
For an interval, mark a start and end in the player and copy the reference with
its `atMs` and `endMs` values.
Compare pacing, comprehensibility, product visibility and the ending against the
baseline. An SDR preview is for review, not an HDR color reference; use the HDR
master and a suitable display for appearance decisions.

## 4. Attach feedback to the reviewed render

Suppose the creator asks to keep the product visible at a specific moment in
`candidate`. Use the candidate child's **job ID**, not the batch ID, and the
reviewed output's playback interval in milliseconds. Substitute the actual
current editing revision; if the preceding example has no intervening edits,
it is still 4.

**Request:**

```json
{
  "command": "studio",
  "project": "project",
  "expectedRevision": 4,
  "key": "record-product-feedback",
  "edit": {
    "action": "review-add",
    "id": "product-visible",
    "jobId": "CANDIDATE_CHILD_JOB_ID",
    "atMs": 3000,
    "endMs": 3500,
    "actor": "creator",
    "text": "Keep the product visible at this moment"
  }
}
```

This saves the note at revision 5 with the artifact hash and source anchors.
After subsequent edits, query `{"command":"reviews","project":"project",
"sequence":"candidate"}` and examine `mapping`, `rangeMappings` and their
limitations. `partial`, `rearranged`, `ambiguous` or `removed_or_unmapped`
requires inspection; a temporal match alone cannot identify the relevant screen
region or prove visibility. Keep the original artifact for comparison, and make
a new candidate with a new output ID and current revision if a new planned edit
is needed. Resolve feedback only after checking the requested change with the
creator. See [review feedback](review-feedback.md).
