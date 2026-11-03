# Creator-agent opportunities

Status: opportunities and remaining work, not a release commitment.
Priorities are hypotheses until creators have tried the workflow.
The goal is to help an agent turn recordings into useful, reviewable content
while the creator retains control over meaning, appearance and final choices.

The current source build already provides source-matched baselines, exact-time
paper edits, searchable transcript cues, named variants, frozen renders,
feedback, and portable backups. Opt-in [cross-cue search](transcript-search.md)
returns source-bound matches and, when supported, requests for source-context
proxy jobs; the resulting artifacts must still be rendered and verified. See [implementation
status](implementation-status.md), the [agent guide](../AGENT_GUIDE.md), and
the [API reference](agent-api.md) for actual behavior and qualification. The
existing [editor research](editor-research.md) describes relevant editing
workflows observed in official documentation; it is not a competitive benchmark.
Published 0.3.1 and current 0.4.0 source have different capabilities. The
[creator toolkit](creator-agent-toolkit.md) describes the current slices for
source evidence, optional provider search, reviewed word timing, translation
proposals, subject observations and checked batch delivery. These slices do
not replace the research and quality work below.

## First: learn from complete creator workflows

**Scenario:** A creator asks an agent to turn a recording into two distinct
shorts, then returns later with a precise revision. Existing tests establish
important technical contracts, but generated footage cannot establish whether
the resulting story, captioning or pacing is good for a real audience.

**Acceptance:** Conduct a small, documented trial with consenting creators and
permitted media. Record source/device/host, the creator's request, actions and
tool calls, time to first review, corrections, unsuccessful attempts, which
version the creator preferred and why, and whether the requested revision was
completed after reopening. Include a source-preserving baseline and an agent's
candidate; distinguish technical checks from the creator's opinion. Store
private footage and identifying details outside Git; publish only agreed,
non-sensitive findings. Include a physical display and real phone recording
before claiming phone appearance, and record missing samples as gaps.

**Dependency:** A suitable creator, host and footage. Do not treat the generated
media regressions or a single agent demo as a substitute for this trial. Use the
observations to reorder the items below rather than assuming every creator has
the same workflow.

## P1 — Broaden phrase discovery across scripts and uncertain speech

**Scenario:** “Find the part where I explain why we changed the packaging.”
Opt-in `acrossCues` performs a bounded literal search over adjacent,
nonoverlapping cues from one source and supplies source/transcript identities,
whole-cue times and a scoped preview request where available. It lowercases and
normalizes whitespace and is limited to eight cues. Optional canonical Unicode
matching and no-space boundary joining support additional literal searches;
defaults still insert a space and do not normalize Unicode. Continuations
require the reported project revision and the same query/options. It neither
establishes word alignment nor searches the meaning of an inaccurate transcript.

**Next slice:** Report when overlaps, gaps or the cue limit prevented a
candidate from being searched as a continuous phrase. Investigate combining
sequences and case context split across cue boundaries, plus language-aware
search as a distinct, explicitly qualified mode. Keep source playback and
editorial selection with the agent.

**Acceptance:** Licensed or generated multilingual text fixtures include a
non-space-separated phrase split across cues, composed/decomposed accents and
case variants. Explicit join modes find their expected cue spans without joining
different assets, overlapping speakers or distant passages; default-mode
results and revision-pinned pagination remain stable. CLI and MCP agree, and
an unavailable or partial transcript is not reported as proof of absence.
Check real speech and language-specific quality separately before claiming
multilingual search or timing accuracy.

**Dependencies:** Reviewed transcript, source preview jobs and a specified
normalization/join policy. A missing or inaccurate transcript remains incomplete
evidence; the agent still listens before selecting a cut.

## P1 — Explain changes beyond plain source cuts

**Scenario:** “Show me what changed between the natural baseline and the tighter
opening.” [Compare outputs](compare-edits.md) now maps source ranges between two
verified, queued plain-cut renders, including omissions, additions, proven
reordering and ambiguous repeats. It explicitly does not compare audio, color,
framing, cadence or appearance; composed and layered outputs are unsupported.

**Next slice:** Extend the comparison to independently named visual and audio
layers, captions and supported transitions, retaining the link to each frozen
artifact and exposing anything that cannot be mapped. Consider displaying the
structured comparison beside the review videos. Do not imply that a temporal
match proves identical pixels, sound or meaning.

**Acceptance:** Generated fixtures cover added or replaced B-roll, a changed
caption, a moved or muted audio layer and a transition. The report distinguishes
known changes from unknown effects and ambiguous repeats, names exact frozen
revisions, and leaves both outputs unchanged. CLI/MCP agree, and any review-page
presentation matches the structured result. Creator trials must separately
assess whether the explanation helps people make a decision.

**Dependencies:** Frozen artifact identities, exact timeline mappings and a
defined comparison contract for each layer or effect. Visual and artistic
judgments require separate qualification.

## P2 — Review word alignment without replacing corrected text

**Scenario:** “Keep the complete sentence, remove the false start, and ensure
the captions still match what I said.” Current source transcripts use timed
cues; cue boundaries do not guarantee word-level precision.

**Current slice:** An agent can submit a source- and transcript-bound alignment
proposal, inspect its validation and accept it with a reviewer and note. It
preserves corrected text, retains provider attribution and exposes stale
alignments without silently applying them to captions. A separate provider or
agent must produce the word spans; the workbench does not align speech itself.

**Acceptance:** Fixtures cover repeated words, cue-boundary words, overlapping
or out-of-bounds spans, missing words, changed source/transcript and correction
retention. A rejected or cancelled alignment leaves history unchanged; replay
is idempotent and backup retains accepted provenance. Compare alignment against
manually timed, licensed or permitted speech before publishing an accuracy
claim; report language, sample size and errors separately from generated tests.

**Dependencies:** The existing versioned provider boundary, transcript
selection and review workflow. No particular model, language or accuracy is
assumed, and a missing provider must not block ordinary cue-based editing.

## P2 — Check the creator's delivery constraints

**Scenario:** “Make a vertical cut under 45 seconds and a square cut under
100 MB, with captions I can edit.” Existing variants and verified packages make
the files; the constraints are still scattered across agent instructions.

**Current slice:** `delivery-check` evaluates one caller-supplied versioned
brief against a completed queued render. It reports per-constraint pass, fail
or unknown and the artifact identity; see [delivery checks](creator-delivery-checks.md).
The report is returned to the caller and does not itself modify a package.

**Current batch slice:** `batch-delivery-check` evaluates each requested output,
and `batch-delivery` binds reports and successful review artifacts into a
package while retaining failed and unavailable siblings as explicit results.
The package minimizes project metadata; its selected media and caller-authored
metadata still need review before sharing. Pre-render constraint planning and
maintained platform templates remain future work. Do not embed unverified
platform rules.

**Next-slice acceptance:** Exercise one passing and one failing output in a frozen batch,
including an actual file-size check, missing sidecar, mismatched duration and
unknown measurement. The manifest ties each result to its requested brief,
revision and verified artifact; a later project edit cannot alter the package.
CLI/MCP agree, and no result claims that a platform accepted the upload.

**Dependencies:** Existing batch, artifact verification and delivery manifests.
Platform-specific templates would require separately maintained sources and
account-specific tests; direct publication is outside this slice.

## P3 — Prepare a portable batch for another machine

**Scenario:** “Continue my three-output batch on the new computer.” Current
verified backups retain project history and originals, but exclude derived jobs
and exports; restored derived jobs are marked unavailable.

**Slices:** First add a read-only portability report that lists required source,
font, provider and tool identities, expected bytes, optional artifacts and
unavailable work. Then design an opt-in portable bundle for verified completed
artifacts plus a resumable description of unfinished jobs, without treating an
interrupted encoder as a resumable stream.

**Acceptance:** Report missing, changed and unsupported dependencies before
import; on a second clean workspace, verify all included hashes, retain prior
successful outputs, and requeue unfinished work with frozen input identities.
Interrupted transfer, tampering, unavailable providers, retry and duplicate
import must not modify the original project or overwrite a verified artifact.
Record what was actually transferred and measured; do not imply every host is
qualified. See the existing [portable batch target](product-plan.md#beyond-mvp-acceptance-scenarios).

**Dependencies:** Backup/restore, job idempotency and retained artifact policy.
Resume across other operating systems or hosted accounts needs its own tests.

## Validation and sequencing

Each behavior change needs meaningful Rust tests, formatting, strict Clippy and
the affected local media/CLI/MCP checks. Use generated or explicitly licensed
fixtures, compare decoded media where appropriate, and inspect actual results.
Run browser checks for changed review-page behavior. Document exactly which
tests ran and keep the [acceptance matrix](implementation-status.md) current.
Prefer local checks during development to conserve GitHub Actions quota; reserve
native CI and full release qualification for changes that need those results.
