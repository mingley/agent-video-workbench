# Project lifecycle, revisions and review

Status: proposed, building on the existing transactional SQLite prototype.
Features: F01, F09, F10, F17, F20 and F23. See
[current implementation](../implementation-status.md) for the narrower code
available today.

## Entities and authority

| Entity | Identity / ownership | Durable information |
| --- | --- | --- |
| Workspace catalog | Workspace ID; contains locators, not authoritative project state | Project names/IDs, tags, last known location, availability and profile library |
| Project | Project UUID, portable directory | SQLite authority, revision graph, sources, styles, jobs and artifact manifests |
| Original asset | Asset ID plus SHA-256 | Byte length, streams/timebases, provenance, import state and immutable managed object |
| Analysis version | Asset hash + algorithm/model/settings fingerprint | Transcript/scene/frame index and optional confidence; reproducible or explicitly imported |
| Sequence | Stable ID and creator name | Tracks, clips, captions, protected ranges and output settings |
| Variant | Sequence ID plus origin sequence/revision | Independent later edits and explicit inherited profile version |
| Edit revision | Monotonic project revision and parent | Snapshot, authored operation batch, omitted ranges and exact request outcome |
| Review item | Review ID + artifact hash/revision | Time/range/region, source mapping, author, text and resolution state |
| Job / artifact | Job/attempt ID; immutable artifact hash | Captured revision, plan, dependencies, outcome, verification and delivery records |

The catalog can be rebuilt from available projects; it never becomes a second
write authority. Initially each project manages its own originals. Workspace
deduplication is a later optimization with explicit reference accounting and
project access checks. Deleting a project cannot delete an asset still required
by another project or retained history.

## Revision behavior

The current SQLite store commits head, revision and idempotency outcome in one
transaction. Extend that boundary to operation results, created IDs, omission
records and editorial metadata. No media render runs while holding the project
write transaction. Long preparation produces immutable candidate artifacts;
the final short transaction rechecks the expected revision before attaching them.

Dry-run returns the normalized edit plan and predicted effects against one
revision. It provides no reservation; apply may return a conflict after another
writer commits. Compare/rebase is explicit. Automatic semantic merging of two
agents' arbitrary timelines is outside the initial model.

Whole-project restore creates a new revision with the old snapshot as its
content and records the restored-from revision. Selective restoration creates
fresh operations against the current snapshot and keeps unrelated later edits.
History distinguishes whole restore, selective restore, normal edit and profile
update. A diff lists created/removed/moved elements, source-range and style
changes, output duration and affected sequences. Full snapshots remain an
available export, not the default response to every history query.

Named variants are separate sequences created from an explicit snapshot. They
share immutable assets and analysis while keeping independent timeline/caption/
framing state. There is one serialized project revision stream. Variant creation
does not introduce a distributed merge engine. Applying a common change across
variants is a visible batch that can fail if any selected variant conflicts.

Acceptance: a stale writer cannot overwrite a newer head; a lost response is
recoverable by key; a variant edit cannot alter the parent sequence. A profile
change and selective restoration survive reopen with the expected diff.

## Profiles, templates and the library

A creator profile has its own ID/version and references font/logo assets,
caption defaults, colors, audio targets, crop preferences, safe areas and export
defaults. A project pins a snapshot of that version. Per-project and per-sequence
overrides are explicit. Updating the library offers an upgrade diff; it never
changes old project output implicitly.

Templates pin their profile and required slots. The library stores reusable
content with metadata and provenance, not a transcript of the creator's chats.
An agent can list/search projects, styles and permitted source annotations in
bounded pages. Cross-project semantic/embedding search is optional and must
name its index/model version; exact text/tag search is the first implementation.

Acceptance: a new font/style version is previewed and applied to selected work;
an old revision still resolves its previous font/style and can be re-rendered.
Exporting a template omits private media unless explicitly included.

## Artifact-based review

A comment is anchored to the file actually reviewed: artifact hash, sequence,
revision, output time/range, and optional normalized display region. Also store
its resolved source references when available. A later edit remaps comments via
those source references; ambiguous/removed content remains visibly unresolved.
The original comment position is always retrievable.

Review states are `open`, `addressed` and `dismissed`, with a resolving revision
and note. Multiple reviewers can be represented as metadata in a single-creator
project; remote authenticated collaboration requires the worker's access model.
The tool records actor attribution but does not infer a human's agreement from
an agent-generated label.

An optional static review bundle contains lightweight MP4s, contact sheets,
captions, a comparison/change summary and a machine-readable comment manifest.
It can be opened locally or shared through the agent's supported file mechanism.
Editing remains headless. Interactive hosted comments can be added later through
the same review-item API, without making a hosted website mandatory.

Technical verification and editorial approval are separate records. A passed
decode check can mark an artifact technically verified. Approval, when used,
names its actor and exact artifact/revision. Export can produce a file awaiting
review; delivery metadata must describe that state accurately. A later revision
does not inherit approval of different rendered bytes.

Acceptance: feedback on an older preview still locates the intended shot after
a new opening changes output timestamps. A missing shot is reported as such.
The final delivery manifest never assigns an old review to a different file.

## Files, retention and portable recovery

Import stages are `transferring`, `verifying`, `ready` and `failed`. Download to
an attempt-specific partial object, verify hash/probe results, then atomically
publish the immutable object and attach its manifest. A restart reconciles an
orphaned published object or interrupted partial without claiming a ready asset.
Signed URLs and credentials live in transient transport configuration, not in
project snapshots or export logs.

Retain originals, pinned fonts/overlays, revision metadata, user corrections,
imported analysis that cannot be reproduced, and designated finals by default.
Proxies, generated analysis and preview caches have explicit retention classes.
Derived is not synonymous with safe to delete: a pinned review artifact or an
irreplaceable imported transcript must survive ordinary cache cleanup.

Garbage collection lists candidates and their referrers, applies a grace period,
then removes only unreferenced objects within its managed root. Active jobs
lease their inputs. Historical revisions and portable backup manifests count
as references. Source deletion is a distinct operation with a dependency report;
cache eviction cannot perform it. Storage quotas should stop new jobs before
they threaten retained project data.

A portable bundle contains a consistent SQLite backup, a format/version manifest,
referenced-object hashes, original/proxy inclusion policy and estimated bytes.
Bundle creation runs while edits continue against a captured revision/backup;
it must never copy a live WAL database as unrelated files. Restore stages into
a new directory, checks manifest/object hashes, then publishes a usable project.
External original references can be relinked by hash, with precise missing-object
reports. A project is editable with missing media, but affected renders fail.

Migrations record schema versions, create a recoverable backup and run
transactionally. Unsupported future schemas are rejected without mutation.
Tool rollback is possible only if the older binary supports the current schema;
otherwise restore the corresponding backup into a separate directory. Never
silently downgrade a project or throw away post-upgrade edits.

Acceptance: restore a project on a fresh supported machine and reproduce its
editable history, styles and selected outputs. Missing media is relinkable.
Interrupt bundle creation, restore and cache collection; no retained original,
active-job input or committed revision becomes unavailable.
