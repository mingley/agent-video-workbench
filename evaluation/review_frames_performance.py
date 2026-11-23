#!/usr/bin/env python3
"""Compare two release binaries on the same generated, frozen 720p edit."""
import argparse
import hashlib
import json
import platform
import shlex
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def run(args, data=None):
    result = subprocess.run(args, input=data, capture_output=True, timeout=300)
    if result.returncode:
        raise RuntimeError((result.stderr or result.stdout).decode(errors="replace")[-4000:])
    return result.stdout


def benchmark(args, root):
    def request(binary, command, **fields):
        payload = {"command": command, "project": "project", **fields}
        output = run([binary, "--workspace", root, "--ffmpeg", args.ffmpeg, "--ffprobe", args.ffprobe,
                      "request", "-"], json.dumps(payload).encode())
        return json.loads(output)["result"]

    run([args.ffmpeg, "-v", "error", "-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=12",
         "-c:v", "libx264", "-preset", "ultrafast", "-threads", "2", "-pix_fmt", "yuv420p",
         "-x264-params", "colorprim=bt709:transfer=bt709:colormatrix=bt709",
         "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
         "-color_range", "tv", root / "source.mp4"])
    request(args.before, "create", name="Frame benchmark")
    request(args.before, "import", source="source.mp4", id="source", expectedRevision=0, key="frame-import")
    request(args.before, "assemble", expectedRevision=1, key="frame-assembly", edit={
        "outputId": "edit", "name": "Two cuts", "cuts": [
            {"id": "one", "assetId": "source", "startMs": 1000, "endMs": 5000},
            {"id": "two", "assetId": "source", "startMs": 7000, "endMs": 11000}]})
    job = request(args.before, "render-start", sequence="edit", expectedRevision=2,
                  key="frame-render", noLaunch=True)["id"]
    run([args.before, "worker", root / "project"])
    status = request(args.before, "job-status", id=job)
    assert status["state"] == "succeeded", status

    # Count external processes in separate untimed runs so tracing does not affect medians.
    trace = root / "media-calls"
    def traced(name, tool):
        wrapper = root / name
        wrapper.write_text("#!/bin/sh\n" + f"echo {name} >> {shlex.quote(str(trace))}\n"
                           + f"exec {shlex.quote(str(tool))} \"$@\"\n")
        wrapper.chmod(0o700)
        return wrapper

    tracing = [traced("ffmpeg", args.ffmpeg), traced("ffprobe", args.ffprobe)]
    cases = {
        "single": [4010],
        "nearby": [4034, 3900, 3934, 3967, 4000, 4067, 4100, 4134],
        "spread": [7000, 1000, 5000, 0, 6000, 2000, 4000, 3000],
    }
    result = {}
    for name, times in cases.items():
        timings = {"before": [], "after": []}
        calls = {}
        reference = None
        # The first pair warms both binaries and establishes the exact reference output.
        for trial in range(args.repeats + 1):
            order = ("before", "after") if trial % 2 == 0 else ("after", "before")
            for label in order:
                binary = getattr(args, label)
                tools = tracing if trial == 0 else [args.ffmpeg, args.ffprobe]
                trace.write_text("")
                with tempfile.TemporaryDirectory(dir=root) as scratch:
                    destination = Path(scratch) / "frames"
                    payload = {"command": "thumbnail-candidates", "project": "project", "id": job,
                               "destination": str(destination), "selection": {"schemaVersion": 1, "atMs": times}}
                    argv = [binary, "--workspace", root, "--ffmpeg", tools[0], "--ffprobe", tools[1], "request", "-"]
                    body = json.dumps(payload).encode()
                    start = time.perf_counter()
                    output = run(argv, body)
                    elapsed = (time.perf_counter() - start) * 1000
                    manifest = json.loads(output)["result"]["manifest"]
                    if reference is None:
                        reference = manifest
                    assert manifest == reference, (name, label, trial)
                    for entry in manifest["candidates"]:
                        png = (destination / entry["path"]).read_bytes()
                        assert hashlib.sha256(png).hexdigest() == entry["image"]["sha256"]
                    if trial:
                        timings[label].append(elapsed)
                    else:
                        calls[label] = len(trace.read_text().splitlines())
        result[name] = {"medianMs": {k: round(statistics.median(v), 1) for k, v in timings.items()},
                        "mediaProcesses": calls}
    return {"size": "1280x720", "framesPerSecond": 30, "outputSeconds": 8, "pairs": args.repeats,
            "machine": platform.machine(), "ffmpeg": run([args.ffmpeg, "-version"]).decode().splitlines()[0],
            "manifestsAndPngHashesMatched": True, "cases": result}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("before", "after", "ffmpeg", "ffprobe"):
        parser.add_argument("--" + arg, type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    options = parser.parse_args()
    if options.repeats < 1:
        parser.error("repeats must be positive")
    for arg in ("before", "after", "ffmpeg", "ffprobe"):
        setattr(options, arg, getattr(options, arg).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="avw-review-") as directory:
        print(json.dumps(benchmark(options, Path(directory)), indent=2))
