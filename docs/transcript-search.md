# Find a phrase across transcript cues

The current 0.4.0 source build can find literal text split across adjacent
transcript cues. Start with an imported, reviewed transcript; a missing or
incorrect word can still cause a false negative. This is text search, not
semantic search, speech recognition, or word-level alignment.

```sh
avw transcript-search project "we changed the packaging" \
  --across-cues --asset-id source --limit 20
```

The JSON request, also accepted as MCP `avw` arguments, is:

```json
{
  "command": "transcript-search",
  "project": "project",
  "assetId": "source",
  "query": "we changed the packaging",
  "acrossCues": true,
  "maxGapMs": 2000,
  "limit": 20
}
```

Without `acrossCues`, the previous single-cue substring search and its response
shape are unchanged. With it, each result includes the asset and source hash,
transcript hash/language/provider, matched cue IDs and text, half-open
`sourceStartMs`/`sourceEndMs`, and at most one neighboring cue on each side.
The times cover complete cues; they do not identify the exact spoken word.
Repeated occurrences in one identical cue span are returned once, while
distinct matching cue spans produce separate results.

The mode lowercases text, collapses whitespace and inserts one space between
cues. Punctuation is literal; Unicode normalization and full case folding are
not applied. Phrases in scripts that do not use spaces between words may not
match across cues. Cues are sorted by source start, end and ID. Search joins at
most eight successive, non-overlapping cues from one asset, and only when each
adjacent gap is at most `maxGapMs` (default 2000; accepted range 0–5000).
Overlapping or nested cues are never joined, because their spoken order may be
ambiguous. The mode accepts 1–256 normalized UTF-8 bytes and at most 1024
original query bytes. A page contains at most 100 results.

Read `revision` and `nextOffset`. To request a continuation, set
`expectedRevision` to the returned revision and `offset` to `nextOffset`;
the named CLI flags are `--expected-revision` and `--offset`. A changed project
returns a revision conflict, so restart the search instead of mixing pages from
different revisions. A nonzero offset requires the revision guard in this mode.

When the selected source has video, a source hash and a known duration, a result
may provide a complete `previewRequest`. It covers the whole matched cue span
plus up to one second of surrounding source, adjusted to stay within the source
and the five-minute proxy limit. If the span itself is too long or source
metadata is unavailable, `previewRequest` is null and
`previewUnavailableReason` explains why. Run the request unchanged, start the
worker and retrieve its verified attachment; it is an SDR listening preview,
not a final source or HDR appearance reference. Listen across the source before
making a cut.

Generate and submit a preview on the same configured CLI/MCP service. Preview
keys beginning `avw-preview-v1-` are reserved and bind the frozen job input,
backend, project identity and source hash; a request copied to a different
service configuration or project is rejected with an instruction to regenerate
it. Retrying the same accepted request reuses its queued job. Planning's source
context previews use the same rule. See [edit planning](edit-planning.md) for
reviewing and applying a candidate after listening.
