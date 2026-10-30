# Map creator feedback across edits

Use a current source build. A review refers to a specific rendered artifact,
not the project's current head. Record the reviewed output interval in
milliseconds, the actor, and the requested change. `jobId` is the completed
render's artifact ID; `expectedRevision` is the current editing head.
In a delivered review page, use **Set range start** and **Set range end** at
the desired playback positions, then copy the reference for the agent. The
page reports the render job ID, frozen revision, `atMs` and `endMs`; it does
not itself save a review. Times are rounded to milliseconds, and the end must
be after the start and within the frozen output. The single-moment reference
remains available when a range is not needed.

```json
{
  "command": "studio",
  "project": "project",
  "expectedRevision": 9,
  "key": "review-keep-product",
  "edit": {
    "action": "review-add",
    "id": "keep-product",
    "jobId": "RENDER_ARTIFACT_ID",
    "atMs": 3000,
    "endMs": 5000,
    "actor": "creator",
    "text": "Keep the closeup and product visible"
  }
}
```

Send this request through CLI JSON or MCP. The artifact bytes are verified
before the review is saved. Its original artifact hash, revision, interval,
text and source anchors stay attached to the review after later edits.

Use `{"command":"reviews","project":"project","sequence":"OUTPUT_ID"}` to
locate that feedback in a current output. For new reviews, `mappingBasis` is
`source-range`. `rangeMappings` lists each captured source interval and the
current placements, including exact rational source and output boundaries.
`atMs` and `endMs` on placements are rounded conveniences for playback;
use the rational boundaries for edits. Each new source range records its
`mediaKind` as video or audio. A visual clip with enabled source audio records
both, so removing the picture while retaining sound, or the reverse, is partial.
Audio may remain on the visual clip or move to an aligned audio track.

| `mapping` | Meaning |
| --- | --- |
| `resolved` | All mapped source intervals survive with one consistent timeline shift. |
| `rearranged` | Every mapped interval survives, but the pieces have moved apart or changed order. Review each placement. |
| `partial` | Some material or destination could not be mapped completely. Inspect each range and its limitations. |
| `ambiguous` | The same source interval appears at different output positions. Choose the intended placement. |
| `removed_or_unmapped` | No supported placement could be identified. Consult the original artifact and source. |

`arrangement` is `preserved`, `changed`, or `unknown`. A resolved mapping is
temporal evidence, not approval or a promise that the image or sound is
unchanged. Multiple placements of the same media kind at the same source and
output time are not treated as different temporal destinations.

`sourceRangeCoverage` describes whether normal-speed video/audio clip ranges
cover the reviewed interval. `sourceRangeLimitations` identifies encountered
non-normal speed, nested sequences, transition handles, or a non-temporal or
mismatched source. The same limitations can appear on an individual destination
range. Nested destinations are conservatively incomplete even when another
clip maps. A gap can also make coverage partial. Speed curves, freeze frames,
transition handles and nested contents do not yet have source-range remapping.

Temporal source coverage does not determine which layer was visible, whether a
clip was obscured, which screen region the actor meant, or whether a caption,
graphic, effect or mix still conveys the same meaning. Listen and view the
artifact and the new output before acting. Multiple layers are candidates, not
an inferred subject selection.

Reviews saved by older builds have `mappingBasis: "legacy-point"`; only their
original start point can be remapped, and `arrangement` is `unknown`. Their
stored intervals and text remain available, but a full source range cannot be
recovered from that point alone. Range records created without `mediaKind` stay
readable but return `partial` with `media-kind-unknown` when placements exist,
because their original video/audio identity cannot be established.
`review-resolve` records a separate addressed
or dismissed decision; it does not rewrite the artifact or its original note.
