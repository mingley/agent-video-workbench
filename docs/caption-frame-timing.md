# Keep short captions visible

The current source build refuses a `compose` when a retained transcript cue
would disappear after its start and end are rounded to the 30 fps output frame
grid. The structured error identifies the cue, source asset, cut and the exact
intersecting source interval. This can happen with a very short cue or when a
cut retains only a few milliseconds at the beginning or end of a longer cue.
The failed request does not create an output, advance history or reserve its
idempotency key. A dry run applies the same check.

Listen to the source and review the cue timing and cut together. Extend or move
the cut if the retained speech needs more room, or correct the cue timing when
the reviewed transcript is inaccurate. Then run a new compose request. Cue
timestamps are not guaranteed word-level alignment, and the tool does not
invent a one-frame duration or decide whether a clipped word can be discarded.
Short cues that occupy at least one output frame remain present; adjacent cues
stay separate. Overlapping cues on the same caption track are rejected by the
existing timeline collision validation rather than silently merged.

Successful older requests still replay their original outcomes. If an output
was composed before this check, replaying the same key does not repair a cue
that was omitted at the time. Review the existing caption/source bindings and
SRT/VTT alongside the video; after adjusting the cut or cue if needed, compose
a new named output with a new key and render it again. Originals and existing
history remain available.
