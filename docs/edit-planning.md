# Plan an edit with source context

Use a current 0.4.0 source build. Keep the continuous `natural` baseline from
[natural edits](natural-edit.md), then create a separate candidate. These tools
turn an editing brief into an inspectable paper edit: the external agent still
chooses the story, listens to footage and judges the result.

`plan-edit` is read-only. It validates the proposed source-preserving assembly
and protection policy, lists retained and excluded material, and checks the
actual frame-rounded boundaries against reviewed source cues. It returns
surrounding transcript text, playable source-context job requests, and proposed
boundary expansions. It never cuts silence, shortens reactions or adds styling
automatically. A target duration is a comparison, not an instruction to force
the performance into that length.

Save this inner plan as `candidate-plan.json`. It assumes the imported asset
`source` lasts at least ten seconds; choose ranges from your own footage.

```json
{
  "brief": {
    "objective": "Keep the delivery natural; remove only the reviewed reset between takes",
    "pacing": "natural",
    "speechHandlesMs": 250,
    "contextMs": 1000
  },
  "edit": {
    "outputId": "candidate",
    "name": "Reviewed reset removal",
    "cuts": [
      {"id":"opening","assetId":"source","startMs":0,"endMs":4500},
      {"id":"ending","assetId":"source","startMs":5500,"endMs":10000}
    ]
  },
  "decisions": [
    {"cutId":"opening","reason":"Keep the complete opening and reaction"},
    {"cutId":"ending","reason":"Resume after the reviewed reset; keep the original ending"}
  ]
}
```

After creating the baseline, the walkthrough's head is revision 2:

```sh
avw plan-edit project --request candidate-plan.json --expected-revision 2
```

For CLI JSON or MCP, send `command: "plan-edit"`, `project`,
`expectedRevision`, and `plan` containing that same object. Check `ok`, then
read `result.readyToApply`, `cuts`, `omittedSourceRanges` and `preflight`.
Every cut needs exactly one reason. The report includes source hashes, exact
rational source/output ranges and requested milliseconds; the actual duration
can differ slightly after frame rounding.

## Review before committing

For each cut, inspect both `contextWindows`. Each contains the exact boundary,
its offset in the surrounding source, and a complete `previewRequest` with
scoped project, revision and stable key. Send that request unchanged, then run
the worker and retrieve the verified analysis artifact. Its `attachments`
include `proxy.mp4` with audio and an explicit source-time map. Analysis leaves
the editing head unchanged. These are SDR review proxies, never final sources
or HDR appearance references. Inspect the original on a suitable display when
judging HDR color.

Listen before and after each boundary, then compare the assembled candidate
with the baseline. Look for clipped words, breaths, reactions, silent actions
and changes in meaning. Excluded opening/ending and internal gaps are listed
even when no transcript text exists. Excerpts are bounded to twelve cues per
list; search the full reviewed transcript when needed. No transcript means
`speechEvidence: "unavailable"`, not that a cut is safe. Cue times can describe
whole phrases rather than exact word boundaries.

Risk codes are `speech-boundary`, `speech-handle`, `tight-cut`, `short-pause`
and `reordered-source`. Unreviewed risks make `readyToApply: false`. Prefer
correcting source ranges; `boundaryExpansionProposal` offers a possible
expansion, but it can include another thought or collide with a later cut.
Listen and re-plan before using it. No proposal is applied automatically.

When a risky cut is intentional after playback, add a specific note to that
cut's decision, then run `plan-edit` again:

```json
{
  "cutId":"ending",
  "reason":"Use the clean second take",
  "reviews":[
    {"risk":"speech-handle","note":"Played both sides; the cue includes trailing room tone and the full spoken ending remains"}
  ]
}
```

`speechHandlesMs` defaults to 250 and accepts 0..2000; `contextMs` defaults to
1000 and accepts 250..5000. `pacing` records `natural` or `montage` intent;
neither automatically cuts or waives speech review. Assembly's explicit
`allowTightCuts`/`allowReorder` flags still govern structural exceptions, and
the plan requires individual review notes for detected exceptions. Optional
`targetDurationMs` accepts 1..3600000 and reports the duration difference.

## Apply the exact reviewed plan

Use the new `planSha256` from the ready report:

```sh
avw apply-edit-plan project --request candidate-plan.json \
  --expected-revision 2 --key apply-candidate-plan \
  --plan-sha256 SHA_FROM_REPORT --dry-run
avw apply-edit-plan project --request candidate-plan.json \
  --expected-revision 2 --key apply-candidate-plan \
  --plan-sha256 SHA_FROM_REPORT
avw edit-preflight project --sequence candidate
```

The JSON/MCP command is `apply-edit-plan` with `plan`, `project`,
`expectedRevision`, `key`, `planSha256` and optional `dryRun`. Its SHA binds the
complete plan and base project snapshot. Changed plan or project means re-plan;
same-key/same-request replay returns the committed outcome. Dry-run saves no
history. Applying saves the sequence, brief, reasons, risk notes and review
fingerprint in the same revision transaction. All originals remain untouched.

`edit-preflight.result.editPlan.status` is `current`, `stale` or `unplanned`.
Changed timing, source identity, style/policy or reviewed transcript makes the
record stale; an unrelated project rename does not. Stale means review the
new edit again, not that the old notes disappeared. Use a new output ID for a
new planned candidate. Planned edits currently create source-preserving plain
assemblies; styled SDR compositions still use `compose` and the existing studio
workflow. Other raw edits remain available and can invalidate a plan record.

Render, retrieve and deliver `candidate` using the [natural-edit commands](natural-edit.md#inspect-freeze-and-retrieve)
at the resulting revision. The frozen render manifest includes the plan's
review state and decisions. Later feedback belongs to the exact artifact via
`review-add`. Risk notes are an agent's recorded intent; neither they nor
technical QC certify human approval or artistic quality.
