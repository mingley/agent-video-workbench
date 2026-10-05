# Lessons from established editors

Official documentation reviewed October 5, 2026. This is documentation research
and executable FFmpeg testing, not a claim that Resolve or Kdenlive was installed
and compared against private footage.

| Source | Relevant behavior | Workbench decision |
| --- | --- | --- |
| [Kdenlive project settings](https://docs.kdenlive.org/en/project_and_asset_management/project_settings/general_settings.html) | Project profiles make resolution, aspect ratio, frame rate and color space explicit | `assemble` derives upright source dimensions/rate; preflight reports the actual output contract |
| [Kdenlive rendering](https://docs.kdenlive.org/en/exporting/render.html) | Proxy/render-preview and rescale choices are distinct from normal delivery; quality profiles are explicit | Encode the master directly from originals; generate a separate labelled SDR review preview; offer high-quality and lossless video choices |
| [DaVinci Resolve color](https://www.blackmagicdesign.com/products/davinciresolve/color) | Wide-gamut/HDR color management, high-precision processing and HDR delivery are explicit capabilities | Preserve native 10-bit YUV for plain PQ/HLG cuts; declare SDR conversion when composing. Do not claim Resolve-class HDR grading/compositing |
| [FFmpeg filters](https://ffmpeg.org/ffmpeg-filters.html) | Overlay pixel format, color-space conversion and linear-light tone mapping are configurable | Keep SDR composition in RGB until one explicit Rec.709 conversion; bypass that compositor entirely for source-preserving cuts |

The reported damage came from several independent decisions: trimming pauses,
adding dominant text, tone mapping HDR, an unintended composite color conversion
and an extra lossy working encode. A single quality slider cannot address all
of those. The existing `820e1e5` fixes keep the SDR composite in RGB and bound
input decoder threads. The new preserving path keeps the original color
contract, raises master encode quality and gives the agent an inspectable,
restrained baseline before styling.

The agent workflow is consequently: inspect the recording, retain a continuous
baseline, create an independent candidate, read preflight, review source handles
and speech across cuts, then freeze and verify delivery. Tiny cuts and removed
short pauses require explicit intent; transcript/silence analysis never edits
the timeline automatically. Technical QC and editorial approval remain distinct.

Further capability work needs representative camera samples and display review,
especially dynamic HDR metadata, unusual color profiles and HDR overlays. A
managed wide-gamut compositor is a separate project; preserving plain cuts now
avoids forcing recordings through an 8-bit SDR pipeline while that work remains.
