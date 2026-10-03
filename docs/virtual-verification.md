# Virtual verification — October 3, 2026

Fresh tests used the **published 0.3.0 binary**, verified against its release
checksum, with generated media in new projects outside the checkout. All twelve
existing CLI/MCP/media/ASR/recovery suites passed. The user demo remained at
revision 15. [Machine-readable evidence](../evaluation/service-results/virtual-verification.json)
records input/output hashes, measured outcomes, command provenance and limits.

| Trial | Observed result |
| --- | --- |
| HDR and clocks together | 90° rotated 10-bit PQ HEVC with HDR10 mastering metadata; 2s container start; 228ms encoded audio delay; 33/100ms VFR intervals; mixed PQ/HLG/SDR output decoded to 150 frames |
| Independent A/V measurement | Source light flashes and audio bursts both appeared at output 0.5s and 2s; no observed onset difference at 30fps video/10ms audio measurement resolution |
| SDR compositing | Burned white caption peak was RGB 255 on PQ, HLG and SDR branches; output was limited-range BT.709 H.264; original hashes stayed unchanged |
| Agent editing trial | Official MCP SDK on a verified copy of the prepared project; evidence inspection, new vertical/square outputs, replay/conflict, frozen batch at revision 18, later omission/style/restore to revision 22, unchanged protected original and independent square, backup reopen |
| Isolated runtime | Fresh public archive installed as UID 1000 in Ubuntu 24.04 with network disabled and read-only root; no Cargo/Rust/Node/npm/Python present; readiness, HDR render and complete 90-frame decode passed |
| Actual HTTPS import | Scoped raw.githubusercontent.com request fetched a pinned public generated sample; SHA-256 matched before ingest and a new 60-frame render passed verification |
| Browser | Actual Chromium 151 playback and seeking, working VTT/manifest links; desktop and 390/320px phone viewports; visual inspection of generated sheets and phone screenshot |
| Refused paths | Strict schema rejects an HDR-output option; unqualified BT.2020 transform and unsupported white-balance grade refuse rendering; earlier verified delivery remains retrievable |
| Rust | Stable 1.99.0, rustfmt, strict Clippy `-D warnings`, 30 tests passed, zero ignored; generated request schema matches |

Browser emulation found a review-page defect in 0.3.0: a requested 390px phone
used a 980px desktop viewport. **0.3.1** adds the device viewport and responsive
video width. The new regression reproduces the old failure and passes on the fix,
including accurate seeking to 2s and both vertical/square/HDR review bundles.
The [native CI run](https://github.com/mingley/agent-video-workbench/actions/runs/37128053961)
passed the combined-media fixture on both architectures and browser qualification
on x86_64. The x86_64 run has twelve passing harness summaries; ARM64 has eleven,
with the browser step explicitly skipped. The initial browser CI run sampled
playback too early at 350ms; the corrected test requires actual clock advancement
within ten seconds and keeps the same playback/seek/layout assertions. Patch
publication verifies these completed results and archive/source/binary identities.

[0.3.1 is published](https://github.com/mingley/agent-video-workbench/releases/tag/v0.3.1).
Its public x86_64 archive was downloaded into another fresh installation and
passed readiness, the official MCP workflow, the combined HDR/sync/refusal suite
and browser checks on the prepared demo. The MCP configuration now selects
0.3.1, with the demo still at revision 15 and its protected range intact.
Reusable installation/start instructions were exercised, saved and read back.

The [updated Node 24 workflows](https://github.com/mingley/agent-video-workbench/actions/runs/37128475887)
passed on both native runners with zero annotations. Publication's workflow
token could upload/edit a release but received 403 when creating its draft.
The qualified tag and draft were created through the existing authorized
repository connection; the publisher then verified and uploaded all seven
assets successfully. No additional credential was needed.

These are generated camera-like fixtures and Chromium viewport emulation.
They do not certify physical phone playback, iOS Safari, real camera appearance,
Dolby Vision/Log transforms, HDR-output masters, macOS/Windows or unspecified
hosted agent accounts. The environment provides a Linux container runtime;
no usable additional OS guest was present. The qualified delivery contract
remains Linux x86_64/ARM64, Rec.709 SDR H.264/AAC.
