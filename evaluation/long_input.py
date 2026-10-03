#!/usr/bin/env python3
"""Measure bounded late-source editing on a generated one-hour CPU fixture."""
import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('avw','ffmpeg','ffprobe','font','output'):
        p.add_argument('--'+name,required=True)
    a=p.parse_args();root=Path(a.output).resolve();root.mkdir(parents=True,exist_ok=False)
    measurements=[]
    def run(argv,json_result=False):
        start=time.monotonic();process=subprocess.Popen(list(map(str,argv)),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        peak=0
        while process.poll() is None:
            rows={}
            for stat in Path('/proc').glob('[0-9]*/stat'):
                try:
                    text=stat.read_text().rsplit(') ',1)[1].split()
                    rows[int(stat.parent.name)]=(int(text[1]),int(text[21])*4096)
                except (OSError,ValueError,IndexError):
                    pass
            owned={process.pid};changed=True
            while changed:
                new={pid for pid,(parent,_) in rows.items() if parent in owned}
                changed=not new.issubset(owned);owned.update(new)
            peak=max(peak,sum(rows.get(pid,(0,0))[1] for pid in owned))
            time.sleep(0.1)
        stdout,stderr=process.communicate(timeout=10)
        assert process.returncode==0,(argv,stderr.decode(errors='replace'),stdout.decode(errors='replace'))
        measurements.append({'command':str(argv[-1]),'elapsedSeconds':round(time.monotonic()-start,3),'peakProcessTreeRssBytes':peak})
        (root/'measurements.json').write_text(json.dumps(measurements,indent=2))
        if json_result:
            data=json.loads(stdout);assert data['ok'],data;return data['result']
    source=root/'one-hour.mp4'
    run([a.ffmpeg,'-v','error','-f','lavfi','-i','color=c=lime:s=256x144:r=10:d=3600',
         '-f','lavfi','-i','sine=frequency=440:duration=3600','-c:v','libx264','-preset','ultrafast',
         '-threads','2','-pix_fmt','yuv420p','-c:a','aac','-b:a','32k','-shortest',source])
    project=root/'project'
    def avw(*args):return run([a.avw,'--ffmpeg',a.ffmpeg,'--ffprobe',a.ffprobe,*args],True)
    avw('create',project,'--name','One-hour fixture')
    avw('import',project,source,'--id','phone','--expected-revision',0,'--key','long-source')
    avw('import',project,a.font,'--id','font','--expected-revision',1,'--key','long-font')
    compose={'outputId':'late_short','name':'Late-source preview','fontAssetId':'font','width':360,'height':640,'fontSize':24,
             'cuts':[{'id':'late','assetId':'phone','startMs':3540000,'endMs':3545000}]}
    edit=root/'compose.json';edit.write_text(json.dumps(compose))
    avw('compose',project,'--request',edit,'--expected-revision',2,'--key','late-compose')
    artifact=avw('render',project,'--sequence','late_short')
    manifest=json.loads(Path(artifact['manifest']).read_text())
    assert manifest['verification']['expectedFrames']==150
    assert manifest['inputSeeksSeconds']['phone']==3540,'late render did not seek before decode'
    assert max(m['peakProcessTreeRssBytes'] for m in measurements)<1024*1024*1024,'memory exceeded 1 GiB fixture budget'
    summary={'passed':True,'sourceSeconds':3600,'sourceBytes':source.stat().st_size,
        'outputFrames':150,'inputSeekSeconds':3540,'measurements':measurements,
        'limitations':['256x144 10fps synthetic SDR, 360x640 output; not a 4K/iPhone benchmark','Linux /proc sampled RSS, not a hard memory reservation']}
    (root/'summary.json').write_text(json.dumps(summary,indent=2))
    print('Long-input fixture passed:',root/'summary.json')


if __name__=='__main__':main()
