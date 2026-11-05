#!/usr/bin/env python3
"""Measure source browsing and bounded native-frame-rate previews on generated media."""
import argparse,hashlib,json,statistics,subprocess,time,platform
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
for n in ('avw','ffmpeg','ffprobe','output'):p.add_argument('--'+n,required=True)
a=p.parse_args();root=Path(a.output).resolve();root.mkdir(parents=True,exist_ok=False);project=root/'project';ledger=[]
def run(args,input_data=None):
 p=subprocess.run(list(map(str,args)),input=input_data,capture_output=True,timeout=300)
 assert p.returncode==0,(args,p.stdout[-1500:],p.stderr[-1500:]);return p.stdout
def call(command,**fields):
 req={'command':command,'project':str(project),**fields}
 out=json.loads(run([a.avw,'--ffmpeg',a.ffmpeg,'--ffprobe',a.ffprobe,'request','-'],json.dumps(req).encode()));assert out['ok'],out
 ledger.append({'request':req,'response':out});(root/'commands.json').write_text(json.dumps(ledger,indent=2));return out['result']
def view(command,request,key):
 t=time.perf_counter();j=call(command,request=request,key=key,expectedRevision=1,noLaunch=True)
 run([a.avw,'worker',project]);state=call('job-status',id=j['id']);assert state['state']=='succeeded',state
 result=call('artifact',id=j['id']);elapsed=time.perf_counter()-t
 return elapsed,json.loads(Path(result['path']).read_text())['data'],state['result']['cacheHit']
source=root/'source.mp4'
run([a.ffmpeg,'-v','error','-f','lavfi','-i','testsrc2=s=1280x720:r=30:d=120','-f','lavfi','-i','sine=frequency=440:duration=120','-c:v','libx264','-preset','ultrafast','-threads','2','-g','240','-bf','2','-sc_threshold','0','-x264-params','colorprim=bt709:transfer=bt709:colormatrix=bt709','-color_range','tv','-c:a','aac','-shortest',source])
source_sha=hashlib.sha256(source.read_bytes()).hexdigest();call('create',name='Source views benchmark');call('import',source=str(source),id='source',expectedRevision=0,key='benchmark-source')
measure={k:[] for k in ['storyboardCold','storyboardWarm','previewCold','previewWarm']};details=[]
for n in range(3):
 for kind in ['storyboard','preview']:
  if kind=='storyboard':command='source-storyboard';q={'assetId':'source','sampleEveryMs':10000,'limit':12,'maxBytes':1048576+n}
  else:command='budget-preview';q={'assetId':'source','startMs':15300,'endMs':25300,'maxDurationMs':10000,'maxBytes':1048576+n,'width':640,'height':360}
  for phase in ['Cold','Warm']:
   elapsed,data,hit=view(command,q,f'{kind}-{phase}-{n}');assert hit==(phase=='Warm')
   measure[kind+phase].append(elapsed);details.append({'kind':kind,'phase':phase,'seconds':elapsed,'bytes':data['bytes'],'cacheHit':hit,'encodingAttempts':data.get('attempts')})
   print(kind,phase,n,round(elapsed,3),flush=True)
   if kind=='storyboard':assert [t['sourceTimeMs'] for t in data['tiles']]==list(range(0,120000,10000))
   else:assert data['bytes']<=q['maxBytes'] and data['verification']['videoFrames']==300
assert hashlib.sha256(source.read_bytes()).hexdigest()==source_sha
summary={'passed':True,'sourceSeconds':120,'sourceBytes':source.stat().st_size,'sourceSha256':source_sha,'previewSeconds':10,'previewFps':30,'previewFrames':300,'storyboardSamples':12,'sampleEveryMs':10000,
 'repeatedSeconds':measure,'medianSeconds':{k:statistics.median(v) for k,v in measure.items()},'measurements':details,
 'platform':platform.platform(),'cpuModel':next(s.split(':',1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines() if s.startswith('model name')),
 'cpuQuota':Path('/sys/fs/cgroup/cpu.max').read_text().strip(),'memoryLimitBytes':Path('/sys/fs/cgroup/memory.max').read_text().strip(),
 'binarySha256':hashlib.sha256(Path(a.avw).read_bytes()).hexdigest(),
 'limits':['Generated SDR; warm OS filesystem cache; no concurrent build/render','Cold means uncached analysis recipe, not cold disk. Source import/generation excluded; queue, worker, source/attachment integrity and artifact lookup included.','No semantic model/ASR or host-specific media acceptance tested; camera/4K performance is not inferred','Three budget variants force uncached work; quality/dimensions and all source frames are preserved within each recipe']}
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))
