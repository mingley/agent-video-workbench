#!/usr/bin/env python3
"""FFmpeg-only investigation of audio-clock preservation on generated media."""
import argparse
from array import array
from fractions import Fraction
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import wave
import zlib

p = argparse.ArgumentParser()
p.add_argument("--ffmpeg", required=True)
p.add_argument("--ffprobe", required=True)
args = p.parse_args()

def run(argv, stderr=False):
    result = subprocess.run(list(map(str, argv)), capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f"{argv}: {result.stderr.decode(errors='replace')[-3000:]}")
    return result.stderr if stderr else result.stdout

def make_fixture(path, rate=16000, origin=0, delay=0, gap=False):
    audio = (r"aevalsrc=0.5*sin(2*PI*1000*t)*(between(t\,0.25\,0.4)+between(t\,1.5\,1.65))"
             f":s={rate}:d=3")
    shift = f"PTS+{origin + delay}/TB"
    if gap:
        shift += "+gte(T\\,1)/TB"
    graph = f"[0:v]setpts=PTS+{origin}/TB[v];[1:a]asetpts={shift}[a]"
    run([args.ffmpeg,"-nostdin","-v","error","-copyts",
         "-f","lavfi","-i","testsrc2=s=64x64:r=10:d=4",
         "-f","lavfi","-i",audio,"-filter_complex",graph,
         "-map","[v]","-map","[a]","-fps_mode","passthrough",
         "-c:v","libx264","-preset","ultrafast","-threads:v","1",
         "-c:a","pcm_s16le","-avoid_negative_ts","disabled","-y",path])

def wave_data(path):
    with wave.open(str(path), "rb") as wav:
        spec = (wav.getnchannels(), wav.getframerate(), wav.getsampwidth())
        if spec != (1, 16000, 2):
            raise ValueError(f"wrong WAV format {spec}")
        count = wav.getnframes()
        data = wav.readframes(count)
    pcm = array("h")
    pcm.frombytes(data)
    if sys.byteorder != "little":
        pcm.byteswap()
    loud = []
    size = 160
    for i in range(0, len(pcm) - size + 1, size):
        chunk = pcm[i:i+size]
        if sum(int(x)*int(x) for x in chunk) / size > 200**2:
            if not loud or i - loud[-1][-1] > size:
                loud.append([i])
            else:
                loud[-1].append(i)
    return count, data, [round(group[0]/16, 3) for group in loud]

def inspect(name, path, root):
    probe = json.loads(run([args.ffprobe,"-v","error","-select_streams","a:0",
        "-show_frames","-show_streams","-show_format","-show_entries",
        "format=start_time,duration:stream=index,time_base,start_time,sample_rate,channels:frame=best_effort_timestamp,best_effort_timestamp_time,nb_samples",
        "-of","json",path]))
    stream = probe["streams"][0]
    frames = probe["frames"]
    base = Fraction(stream["time_base"])
    source_pts = [int(f["best_effort_timestamp"]) for f in frames]
    source_rate = int(stream["sample_rate"])
    source_gaps = [(Fraction(b-a)*base - Fraction(int(frame["nb_samples"]),source_rate))
                   for a,b,frame in zip(source_pts,source_pts[1:],frames)]
    raw_origin = probe.get("format",{}).get("start_time")
    origin = Fraction(raw_origin) if raw_origin not in (None, "N/A") else None

    wav_path = root / (name + "-new.wav")
    audit_path = root / (name + "-audit.txt")
    graph = ("[0:a:0]asplit=2[src][first];"
             "[first]asettb=1/16000,aselect='eq(n,0)',ashowinfo@asr_clock,anullsink;"
             "[src]aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono,asplit=2[w][audit]")
    log = run([args.ffmpeg,"-nostdin","-hide_banner","-nostats","-v","info","-copyts","-i",path,
         "-filter_complex",graph,
         "-map","[w]","-c:a","pcm_s16le","-f","wav","-y",wav_path,
         "-map","[audit]","-c:a","pcm_s16le","-avoid_negative_ts","disabled",
         "-fs",str(8*1024*1024),"-f","framecrc","-y",audit_path],stderr=True).decode(errors="replace")[-65536:]
    matches = [x for x in log.splitlines() if "ashowinfo@asr_clock" in x and re.search(r"\bn:0\b",x)]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one first-frame clock marker: {matches}")
    match = re.search(r"\bpts:(-?\d+)\b",matches[0])
    if not match:
        raise ValueError(f"invalid first-frame marker: {matches[0]}")
    first_marker = int(match.group(1))
    count, data, regions = wave_data(wav_path)
    wave_probe = json.loads(run([args.ffprobe,"-v","error","-select_streams","a:0",
        "-show_entries","stream=duration_ts,time_base,duration","-of","json",wav_path]))
    legacy = root / (name+"-legacy.wav")
    run([args.ffmpeg,"-nostdin","-v","error","-i",path,"-vn","-ar","16000","-ac","1",
         "-c:a","pcm_s16le","-y",legacy])
    _,_,old_regions = wave_data(legacy)

    audit = audit_path.read_text().splitlines()
    tb = Fraction(next(x for x in audit if x.startswith("#tb 0:")).split(":",1)[1].strip())
    packets = []
    for line in audit:
        if not line or line.startswith("#"):
            continue
        fields = [x.strip() for x in line.split(",")]
        if len(fields)<6 or fields[0]!="0":
            raise ValueError(f"unexpected framecrc packet {line}")
        packets.append((int(fields[2]),int(fields[3]),int(fields[4]),int(fields[5],16)))
    if not packets:
        raise ValueError("empty framecrc")
    audit_gaps = [Fraction(b[0]-a[0]-a[1])*tb for a,b in zip(packets,packets[1:])]
    cursor = 0
    checksums_match = True
    for _,_,size,checksum in packets:
        chunk = data[cursor:cursor+size]
        checksums_match &= len(chunk)==size and (zlib.adler32(chunk,0)&0xffffffff)==checksum
        cursor += size
    source_first = source_pts[0]*base
    audit_first = packets[0][0]*tb
    result = {
        "case":name,"formatStartSeconds":float(origin) if origin is not None else None,
        "sourceTimeBase":str(base),"sourceSampleRate":source_rate,
        "firstSourceAudioPtsSeconds":float(source_first),
        "firstAudioRelativeToFormatSeconds":float(source_first-origin) if origin is not None else None,
        "auditTimeBase":str(tb),"firstAuditPtsSeconds":float(audit_first),
        "sourceFirstAs16kPts":first_marker,"auditFirstAs16kPts":float(audit_first*16000),
        "auditMinusIndependentSourceSamples":float(audit_first*16000-first_marker),
        "decodedWavSamples":count,"wavProbe":wave_probe,
        "auditDurationSamples":float(sum(p[1] for p in packets)*tb*16000),
        "auditPacketBytes":cursor,"wavPcmBytes":len(data),
        "auditPacketDataMatchesWav":bool(checksums_match and cursor==len(data)),
        "wavLoudRegionStartsMs":regions,"legacyWavLoudRegionStartsMs":old_regions,
        "maxSourceGapMs":float(max(source_gaps,default=Fraction(0))*1000),
        "minSourceGapMs":float(min(source_gaps,default=Fraction(0))*1000),
        "maxAuditGapMs":float(max(audit_gaps,default=Fraction(0))*1000),
        "minAuditGapMs":float(min(audit_gaps,default=Fraction(0))*1000),
        "auditPacketCount":len(packets),"auditBytes":audit_path.stat().st_size,
    }
    return result

with tempfile.TemporaryDirectory(prefix="avw-audio-clock-") as tmp:
    root = Path(tmp)
    wav = root/"audio-only.wav"
    run([args.ffmpeg,"-nostdin","-v","error","-f","lavfi","-i",
         "sine=frequency=1000:sample_rate=16000:duration=2","-c:a","pcm_s16le","-y",wav])
    cases={"audio_only_wav":wav}
    errors=[]
    for name, opts in [("zero_origin",{}),("delayed_audio",{"delay":1}),
                       ("nonzero_origin",{"origin":5,"delay":1}),
                       ("rate_44100",{"rate":44100,"delay":0.375}),
                       ("internal_gap",{"gap":True}),
                       ("negative_origin",{"origin":-1,"delay":0.5})]:
        try:
            path=root/(name+".mkv")
            make_fixture(path,**opts)
            cases[name]=path
        except Exception as error:
            print(json.dumps({"case":name,"fixtureError":str(error)}),flush=True)
            errors.append(name)
    if "rate_44100" in cases:
        try:
            aac=root/"aac-44100.mkv"
            run([args.ffmpeg,"-nostdin","-v","error","-copyts","-i",cases["rate_44100"],
                 "-map","0:v:0","-map","0:a:0","-c:v","copy","-c:a","aac","-b:a","96k",
                 "-avoid_negative_ts","disabled","-y",aac])
            cases["aac_44100"]=aac
        except Exception as error:
            print(json.dumps({"case":"aac_44100","fixtureError":str(error)}),flush=True)
            errors.append("aac_44100")
    for name,path in cases.items():
        try:
            result=inspect(name,path,root)
            print(json.dumps(result,sort_keys=True),flush=True)
            if name=="internal_gap" and result["maxSourceGapMs"]<900:
                raise ValueError("fixture did not contain an independently observed audio PTS gap")
        except Exception as error:
            print(json.dumps({"case":name,"inspectionError":str(error)}),flush=True)
            errors.append(name)
    if errors:
        raise SystemExit("Incomplete diagnostic cases: "+", ".join(errors))
