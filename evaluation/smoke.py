#!/usr/bin/env python3
"""Evaluate upstream CLIs on generated media; this is not application code.

Requires prebuilt candidate binaries, FFmpeg 8+ with drawtext/libass, and a
local TTF font. Creates a fresh directory and records commands and outputs.
No footage, model API, agent runtime, or credentials are required.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("agentcut", "ave", "openreelio", "ffmpeg", "ffprobe", "font", "output"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    bins = {}
    for name in ("agentcut", "ave", "openreelio", "ffmpeg", "ffprobe"):
        value = shutil.which(getattr(args, name))
        if value is None:
            raise RuntimeError("Missing binary: " + name)
        bins[name] = str(Path(value).resolve())
    font = Path(args.font).resolve()
    if not font.is_file():
        raise RuntimeError("Missing font: " + str(font))
    env = dict(os.environ, OPENREELIO_FFMPEG_PATH=bins["ffmpeg"],
               OPENREELIO_FFPROBE_PATH=bins["ffprobe"])
    replacements = {str(output): "$RUN", str(font): "$FONT"}
    replacements.update({value: "$" + key.upper() for key, value in bins.items()})
    logs = []
    report = {"platform": platform.platform(), "machine": platform.machine(),
              "fixture": "12s generated SDR H.264/AAC; red intro, grey pause, green demo, blue ending",
              "limitations": ["No real iPhone footage, ASR, HDR, long-video benchmark, or external agent tested"]}

    def clean(text):
        for old, new in sorted(replacements.items(), key=lambda pair: -len(pair[0])):
            text = text.replace(old, new)
        return text

    def run(argv, expected=0, binary=False):
        start = time.monotonic()
        p = subprocess.run([str(v) for v in argv], cwd=output, env=env,
                           capture_output=True, text=not binary, timeout=120)
        stdout = "<binary>" if binary else p.stdout
        stderr = p.stderr.decode(errors="replace") if binary else p.stderr
        logs.append({"argv": [clean(str(v)) for v in argv], "exit": p.returncode,
                     "elapsed_seconds": round(time.monotonic() - start, 4),
                     "stdout": clean(stdout), "stderr": clean(stderr)})
        (output / "commands.json").write_text(json.dumps(logs, indent=2) + "\n")
        if expected is not None and p.returncode != expected:
            raise RuntimeError(str(argv) + "\n" + stderr + "\n" + stdout)
        return p

    def jr(argv, expected=0):
        p = run(argv, expected)
        return p, json.loads(p.stdout) if p.stdout.strip() else None

    def ac(*argv, expected=0):
        return jr([bins["agentcut"], "--json", "--ffmpeg", bins["ffmpeg"],
                   "--ffprobe", bins["ffprobe"], "--cache-dir", "cache", *argv], expected)

    def av(*argv, expected=0):
        return jr([bins["ave"], "--ffmpeg", bins["ffmpeg"], "--ffprobe", bins["ffprobe"], *argv], expected)

    def ore(*argv, expected=0):
        return jr([bins["openreelio"], *argv], expected)

    def state(name="project.agentcut.json"):
        return json.loads((output / name).read_text())

    def write(*argv):
        return ac(*argv, "--if-revision", str(state()["revision"]))[1]

    def sha(name):
        return hashlib.sha256((output / name).read_bytes()).hexdigest()

    def probe(name):
        return jr([bins["ffprobe"], "-v", "error", "-count_frames", "-show_streams",
                   "-show_format", "-of", "json", name])[1]

    def media_check(name, frames=None):
        d = probe(name)
        v = next(s for s in d["streams"] if s["codec_type"] == "video")
        audio = next((s for s in d["streams"] if s["codec_type"] == "audio"), None)
        if frames is not None:
            assert int(v["nb_read_frames"]) == frames, (name, v)
        run([bins["ffmpeg"], "-v", "error", "-i", name, "-f", "null", "-"])
        return {"width": v["width"], "height": v["height"],
                "video_frames": int(v["nb_read_frames"]), "video_duration": v.get("duration"),
                "video_rate": v["avg_frame_rate"], "audio_duration": audio.get("duration") if audio else None,
                "container_duration": d["format"]["duration"], "sha256": sha(name)}

    def frame(name, at, target):
        run([bins["ffmpeg"], "-v", "error", "-ss", str(at), "-i", name,
             "-frames:v", "1", target])

    def audio_mean(name):
        p = run([bins["ffmpeg"], "-hide_banner", "-nostats", "-ss", "1", "-t", "1", "-i", name,
                 "-vn", "-af", "aformat=channel_layouts=stereo,volumedetect", "-f", "null", "-"])
        return float(re.search(r"mean_volume: ([-\d.]+) dB", p.stderr).group(1))

    def pixel(name, at):
        p = run([bins["ffmpeg"], "-v", "error", "-ss", str(at), "-i", name,
                 "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], binary=True)
        offset = (10 * 360 + 10) * 3
        return list(p.stdout[offset:offset + 3])

    def op(kind, params, target=None):
        return {"id": "op_" + str(time.monotonic_ns()), "op": kind,
                "target": target, "params": params}

    def batch(name, operations, description):
        s = state()
        b = {"schemaVersion": "1.0.0", "projectId": s["projectId"], "baseRevision": s["revision"],
             "idempotencyKey": "evaluation-" + name, "description": description,
             "author": {"type": "agent", "name": "evaluation"}, "operations": operations}
        (output / (name + ".json")).write_text(json.dumps(b, indent=2) + "\n")
        return b

    def rt(value):
        return {"value": value, "rate": {"numerator": 30, "denominator": 1}}

    report["tool_versions"] = {key: run([value, "-version" if key in ("ffmpeg", "ffprobe") else "--version"]).stdout.splitlines()[0]
                               for key, value in bins.items()}
    run([bins["ffmpeg"], "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:r=30:d=12",
         "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=12",
         "-vf", "drawbox=x=0:y=0:w=iw:h=ih:color=red:t=fill:enable='lt(t,3)',"
         "drawbox=x=0:y=0:w=iw:h=ih:color=gray:t=fill:enable='between(t,3,7)',"
         "drawbox=x=0:y=0:w=iw:h=ih:color=lime:t=fill:enable='between(t,7,10)',"
         "drawbox=x=0:y=0:w=iw:h=ih:color=blue:t=fill:enable='gte(t,10)',"
         "drawbox=x=270:y=120:w=100:h=120:color=white:t=fill",
         "-af", "volume=volume=0:enable='between(t,3,7)'", "-c:v", "libx264", "-pix_fmt", "yuv420p",
         "-g", "30", "-c:a", "aac", "-shortest", "source.mp4"])
    report["source_hash_before"] = sha("source.mp4")
    (output / "captions.srt").write_text("1\n00:00:00,000 --> 00:00:03,000\nOpening\n\n"
                                        "2\n00:00:03,000 --> 00:00:06,000\nProduct demonstration\n")

    ac("init", "project.agentcut.json", "--name", "Representative edit", "--width", "360", "--height", "640", "--fps", "30")
    write("asset", "add", "project.agentcut.json", "source.mp4", "--id", "phone")
    write("asset", "add", "project.agentcut.json", str(font), "--id", "font")
    for track, suffix in (("track_v1", ""), ("track_a1", "_audio")):
        for name, at, source_in in (("intro", "0s", "0s"), ("demo", "3s", "7s")):
            write("clip", "add", "project.agentcut.json", "--id", name + suffix, "--asset", "phone",
                  "--track", track, "--at", at, "--source-in", source_in, "--duration", "3s", "--fit", "cover")
    batch("mute-video-audio", [op("item.set", {"property": "audio.enabled", "value": False}, name)
                              for name in ("intro", "demo")], "Use dedicated audio tracks without doubling")
    ac("apply", "project.agentcut.json", "--operations", "mute-video-audio.json")
    write("track", "add", "project.agentcut.json", "--id", "captions", "--type", "caption")
    write("subtitle", "import", "project.agentcut.json", "captions.srt", "--track", "captions")
    write("subtitle", "style", "project.agentcut.json", "--track", "captions", "--font-size", "22", "--font-asset", "font")
    report["agentcut_draft_revision"] = state()["revision"]
    draft = ac("render", "run", "project.agentcut.json", "--output", "draft.mp4")[1]
    report["agentcut_render"] = draft
    report["agentcut_draft"] = media_check("draft.mp4", 180)
    assert (report["agentcut_draft"]["width"], report["agentcut_draft"]["height"]) == (360, 640)
    report["agentcut_audio_mean_db"] = {"source": audio_mean("source.mp4"), "draft": audio_mean("draft.mp4")}
    assert abs(report["agentcut_audio_mean_db"]["source"] - report["agentcut_audio_mean_db"]["draft"]) < 1.0
    report["agentcut_decoded_pixels"] = {"intro": pixel("draft.mp4", 1), "demo": pixel("draft.mp4", 4)}
    red, green = report["agentcut_decoded_pixels"].values()
    assert red[0] > 200 and red[1] < 40 and green[1] > 200 and green[0] < 40
    cache = ac("render", "run", "project.agentcut.json", "--output", "draft-cached.mp4")[1]
    report["agentcut_cache"] = {"cache_hit": cache["data"]["cacheHit"], "same_bytes": sha("draft.mp4") == sha("draft-cached.mp4")}
    assert all(report["agentcut_cache"].values())
    ac("preview", "contact-sheet", "project.agentcut.json", "--count", "4", "--columns", "4",
       "--tile-width", "180", "--output", "draft-sheet.png")

    # Rejection, retry, and journal failure use actual separate CLI processes.
    before = sha("project.agentcut.json")
    batch("invalid", [op("project.rename", {"name": "Must not commit"}),
                      op("item.set", {"property": "enabled", "value": False}, "missing")], "Invalid batch")
    p, invalid = ac("apply", "project.agentcut.json", "--operations", "invalid.json", expected=None)
    report["agentcut_atomic_rejection"] = p.returncode != 0 and sha("project.agentcut.json") == before
    assert report["agentcut_atomic_rejection"]
    batch("retry", [op("project.rename", {"name": "Retry test"})], "Retry test")
    ac("apply", "project.agentcut.json", "--operations", "retry.json")
    revision = state()["revision"]
    ac("apply", "project.agentcut.json", "--operations", "retry.json")
    report["agentcut_idempotent_retry"] = state()["revision"] == revision
    stale = json.loads((output / "retry.json").read_text())
    stale["idempotencyKey"] = "stale-key"
    (output / "stale.json").write_text(json.dumps(stale))
    p, d = ac("apply", "project.agentcut.json", "--operations", "stale.json", expected=None)
    report["agentcut_stale_revision"] = {"exit": p.returncode, "code": d["error"]["code"]}
    assert d["error"]["code"] == "E_REVISION_CONFLICT"

    shutil.copyfile(output / "project.agentcut.json", output / "journal-failure.agentcut.json")
    (output / "journal-failure.agentcut.json.journal.json").mkdir()
    failed_journal = batch("journal-failure", [op("project.rename", {"name": "History lost"})], "Journal failure")
    p, d = ac("apply", "journal-failure.agentcut.json", "--operations", "journal-failure.json")
    p2, d2 = ac("undo", "journal-failure.agentcut.json", "--if-revision", str(failed_journal["baseRevision"] + 1), expected=None)
    report["agentcut_journal_failure"] = {"edit_reported_success": d["ok"], "warnings": d.get("warnings"),
                                          "undo_exit": p2.returncode, "undo_error": d2.get("error")}

    # Restore an omitted one-second source interval, shrink captions, change opening.
    # The grey interval represents restoration mechanics, not a transcribed sentence.
    operations = [op("item.move", {"to": rt(120)}, name) for name in ("demo", "demo_audio", "sub_0002")]
    for track, name in (("track_v1", "restored"), ("track_a1", "restored_audio")):
        operations.append(op("clip.add", {"id": name, "asset": "phone", "track": track,
                                           "at": rt(90), "sourceIn": rt(90), "duration": rt(30), "fit": "cover"}))
    operations.append(op("item.set", {"property": "audio.enabled", "value": False}, "restored"))
    for name in ("sub_0001", "sub_0002"):
        operations.append(op("item.set", {"property": "text.style.fontSize", "value": 18}, name))
    operations.append(op("item.set", {"property": "text.text", "value": "New opening"}, "sub_0001"))
    batch("restore", operations, "Restore source 3..4s; smaller captions; new opening")
    ac("apply", "project.agentcut.json", "--operations", "restore.json", "--dry-run")
    ac("apply", "project.agentcut.json", "--operations", "restore.json")
    report["agentcut_revision"] = ac("render", "run", "project.agentcut.json", "--output", "revision.mp4")[1]
    report["agentcut_revised"] = media_check("revision.mp4", 210)
    ac("undo", "project.agentcut.json", "--if-revision", str(state()["revision"]))
    ac("render", "run", "project.agentcut.json", "--output", "undone.mp4")
    report["agentcut_undo_restores_render"] = sha("undone.mp4") == sha("draft.mp4")
    assert report["agentcut_undo_restores_render"]
    ac("redo", "project.agentcut.json", "--if-revision", str(state()["revision"]))

    # ave produces a pipeline of intermediate media, not a durable editable project.
    report["ave_silence"] = av("detect", "source.mp4", "--kind", "silence")[1]
    av("keep", "source.mp4", "--ranges", "0-3,7-10", "--accurate", "-o", "ave-cut.mp4")
    av("resize", "ave-cut.mp4", "--width", "360", "--height", "640", "--fit", "crop", "-o", "ave-vertical.mp4")
    av("captions", "ave-vertical.mp4", "--srt", "captions.srt", "-o", "ave-draft.mp4")
    report["ave_draft"] = media_check("ave-draft.mp4", 180)
    p, d = av("keep", "source.mp4", "--ranges", "0-3", "--accurate", "-o", "source.mp4", expected=None)
    report["ave_rejects_in_place"] = p.returncode != 0 and d["error"] == "in_place"

    # OpenReelio must trim video and its linked audio explicitly.
    project = "openreelio-project"
    ore("project", "create", "--path", project, "--name", "Representative edit", "--width", "360", "--height", "640", "--fps", "30")
    asset = ore("asset", "import", "--path", project, "--file", str(output / "source.mp4"))[1]["assetId"]
    tracks = ore("timeline", "tracks", "--path", project)[1]["tracks"]
    track = next(t["id"] for t in tracks if t["kind"] == "Video")
    for at, si, so in (("0", "0", "3"), ("3", "7", "10")):
        d = ore("timeline", "insert", "--path", project, "--asset", asset, "--track", track, "--at", at)[1]
        for tid, cid in ((track, d["createdIds"][0]), (d["linkedAudio"]["trackId"], d["linkedAudio"]["clipId"])):
            ore("timeline", "trim", "--path", project, "--clip", cid, "--track", tid, "--source-in", si, "--source-out", so)
    ore("caption", "add", "--path", project, "--text", "Product demonstration", "--start", "3", "--end", "6")
    ore("render", "start", "--path", project, "--output", str(output / "openreelio-draft.mp4"), "--preset", "mp4_h264_1080p")
    report["openreelio_draft"] = media_check("openreelio-draft.mp4")
    p, d = ore("project", "info", "--path", "missing-project", expected=None)
    report["openreelio_error"] = {"exit": p.returncode, "stdout": p.stdout, "stderr": clean(p.stderr)}
    for name in ("draft", "revision", "ave-draft", "openreelio-draft"):
        frame(name + ".mp4", 4.5, name + "-frame.png")

    # Display-matrix fixture catches double rotation; no claim of real phone validation.
    run([bins["ffmpeg"], "-v", "error", "-display_rotation", "90", "-i", "source.mp4", "-c", "copy", "rotated.mov"])
    ac("init", "rotation.agentcut.json", "--name", "Rotation", "--width", "360", "--height", "640", "--fps", "30")
    ac("asset", "add", "rotation.agentcut.json", "rotated.mov", "--id", "rotated", "--if-revision", "0")
    ac("clip", "add", "rotation.agentcut.json", "--asset", "rotated", "--track", "track_v1", "--at", "0f",
       "--duration", "90f", "--fit", "cover", "--if-revision", "1")
    p, d = ac("render", "run", "rotation.agentcut.json", "--output", "rotation-agentcut.mp4", expected=None)
    report["agentcut_rotation"] = {"exit": p.returncode, "result": d}
    frame("rotated.mov", 1, "rotation-reference.png")
    report["source_hash_after"] = sha("source.mp4")
    assert report["source_hash_before"] == report["source_hash_after"]
    (output / "summary.json").write_text(clean(json.dumps(report, indent=2)) + "\n")
    print(json.dumps({"output": str(output), "commands": len(logs), "completed": True}))


if __name__ == "__main__":
    main()
