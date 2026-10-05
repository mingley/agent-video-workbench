# Implementation completion and remaining qualification

October 5 addition: main's 0.4.0 source build implements plain-cut PQ/HLG
preservation, source-matched defaults, editorial preflight and separate verified
SDR previews. [Natural edits](natural-edit.md) and [current evidence](implementation-status.md)
describe that scope. HDR compositing/grading/captions, real camera/display
qualification and proprietary dynamic metadata remain extensions.

The former N01–N08 implementation gaps are addressed in the 0.3.1 Linux CLI/MCP
route. [Implementation status](implementation-status.md) links actual evidence;
[product plan](product-plan.md) retains the broader M1–M5 acceptance ambitions.
An implemented feature and camera/host/appearance certification are different gates.

| Epic | 0.3.1 implementation | Remaining gate or extension |
| --- | --- | --- |
| E01 History | Atomic revisions/replay/conflicts, selective omission restore and exact policy | Repeated real creator trials |
| E02 Phone ingest | H.264/HEVC/VFR, PQ/HLG→SDR, SAR/rotation/offset/audio-gap fixtures, resumable transfer | Actual iPhone/Dolby Vision compatible samples and appearance review; profile 5/Log/ProRes/Cinematic support |
| E03 Sources | ASR/import/corrections/explicit version selection, searchable cues, PTS indexes, source-mapped proxies, frames/scenes/silence | Real speech accuracy and word-level alignment providers |
| E04 Captions | Source mappings, imported font metrics, safe reflow, overflow/glyph errors, matching sidecars | Complex shaping/RTL and expanded font/script matrix |
| E05 Framing | Independent aspect variants, manual motion and tracked constant viewport crops | Human visibility/safe-area qualification on actual scenes |
| E06 Multi-source | Three-source composition, visual-only B-roll/image replacement, independent J/L dialogue, raw split/trim/transition edits | Repeated creator narrative review |
| E07 Library | Versioned profiles/fonts, explicit upgrade, portable libraries and read-only catalog | Expanded asset-library UX/search beyond CLI discovery |
| E08 Hooks/review | Frozen independent named variants, artifact/time feedback, source-point remap and verified comparison bundle | Region/range feedback mapping across speed curves and hosted UI |
| E09 Templates | Pinned versions, validated fresh slots/duration constraints and atomic instantiation | Additional creator templates rather than inferred private style |
| E10 Audio | Gain/EQ/compressor/limiter/buses/fades, measured ducking, two-pass loudness/peak QC | Human speech A/B, optional noise/voice providers |
| E11 Batch | Atomic frozen set, priority, independent retries/cancel, MP4/cover/SRT/VTT/sheet/manifest/HTML package | External host file delivery and account quotas |
| E12 Language | Explicit supplied translations with independent outputs and reviewed correction history | Translation provider and per-language/script qualification |
| E13 Workers | Ownership/fences/heartbeat, crash recovery, deadlines/log/AS/file/space limits and priority | Shared host-wide reservation pool and hosted runner supervision |
| E14 Portable | Verified consistent backup/restore/relink, retained historical references and lease-safe GC/automation | Repeated off-host/user rollback trials |
| E15 CLI/MCP | Same generated schema/validation/errors/outcomes; official SDK installed-binary workflows | Account-specific consumer-agent integration and remote authenticated transport |
| E16 Installation | Published checksum-verified native Linux x86_64/ARM64 archives; no-root/offline installer | macOS/Windows implementation and native qualification |
| E17 Long input | Measured one-hour low-resolution late seek and short 4K/60→full-HD render | Cold/warm one-hour 4K real phone and disk/cost measurements |
| E18 Compatibility | Schema 1–3 migrations/backups, unchanged schema 4, recovery/replay tests and versioned binaries | Repeated creator trials and broader 1.0 compatibility promise |
| E19 Tracking | Editable selected-region CPU proposals with confidence/occlusion hold; actual animated crop rendering | Face/semantic/multiple-subject ML providers and creator comparison |
| E20 Color | Versioned HDR→SDR transform, exact color tags/matrix, reversible basic SDR grade; 0.4.0 plain-cut PQ/HLG masters | HDR compositing, white balance/advanced grades and human display approval |
| E21 Interchange | OTIO normal-speed cuts/tracks/gaps and SHA binding with explicit loss report/roundtrip | Effects/captions/transitions and qualification with external editor implementations |
| E22 Providers | Versioned immutable analysis process contract, program/source hashes, cache, malformed-output rejection/cancel | Named real provider integrations and cost/data-transfer policies |

No feature above is labeled complete solely because a proposed command exists.
The narrower release contract is the executed Linux matrix. Real creator footage,
phone display review and unspecified hosted accounts cannot be fabricated from
generated tests; their remaining gates are stated openly. Unsupported rendering
paths return named errors instead of silently ignoring edits.
