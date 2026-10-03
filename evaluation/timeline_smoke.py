#!/usr/bin/env python3
"""Measure independent visual/dialogue routes for J/L cuts and logo replacement."""
import argparse
from array import array
import json
import math
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('avw','ffmpeg','ffprobe','output'):
        parser.add_argument('--'+name, required=True)
    args = parser.parse_args(); root = Path(args.output).resolve(); root.mkdir(parents=True,exist_ok=False)
    project = root/'project'; ledger=[]
    def run(argv):
        p=subprocess.run(list(map(str,argv)),capture_output=True,timeout=120)
        assert p.returncode==0,(argv,p.stdout,p.stderr); return p.stdout
    def call(command, **fields):
        request={'command':command,**fields}
        p=subprocess.run([args.avw,'--ffmpeg',args.ffmpeg,'--ffprobe',args.ffprobe,'request','-'],
                         input=json.dumps(request).encode(),capture_output=True,timeout=120)
        response=json.loads(p.stdout); ledger.append({'request':request,'response':response})
        (root/'commands.json').write_text(json.dumps(ledger,indent=2))
        assert p.returncode==0 and response['ok'],response;return response['result']
    call('create',project=str(project),name='Narrative routes')
    for revision,(name,color,hz) in enumerate([('a','red',440),('b','green',880)]):
        source=root/f'{name}.mp4'
        run([args.ffmpeg,'-v','error','-f','lavfi','-i',f'color=c={color}:s=360x640:r=30:d=4',
             '-f','lavfi','-i',f'sine=frequency={hz}:sample_rate=48000:duration=4',
             '-c:v','libx264','-preset','ultrafast','-c:a','aac',source])
        call('import',project=str(project),source=str(source),id=name,expectedRevision=revision,key='import-'+name)
    t=lambda value:{'value':value,'rate':{'numerator':30,'denominator':1}}
    ops=[{'op':'track.add','params':{'id':'speech_b_track','sequence':'seq_main','type':'audio'}}]
    def clip(id,asset,track,at,source,duration,audio=True):
        ops.append({'op':'clip.add','params':{'id':id,'asset':asset,'track':track,'at':t(at),
                    'sourceIn':t(source),'duration':t(duration)}})
        if not audio:ops.append({'op':'item.set','target':id,'params':{'property':'audio.enabled','value':False}})
    clip('visual_a','a','track_v1',0,0,60,False);clip('visual_b','b','track_v1',60,15,60,False)
    clip('speech_a','a','track_a1',0,0,75);clip('speech_b','b','speech_b_track',45,0,75)
    def apply(operations,key):
        p=call('status',project=str(project))
        for i,op in enumerate(operations):op['id']=str(i)
        return call('apply',project=str(project),batch={'schemaVersion':'1.0.0','projectId':p['projectId'],
          'baseRevision':p['revision'],'idempotencyKey':key,'operations':operations})
    apply(ops,'narrative-routes'); before=call('render',project=str(project))
    def amplitude(at,hz):
        data=array('f');data.frombytes(run([args.ffmpeg,'-v','error','-ss',str(at),'-i',before['path'],
                        '-t','0.2','-vn','-ac','1','-ar','16000','-f','f32le','-']))
        return 2*abs(complex(sum(v*math.cos(2*math.pi*hz*i/16000) for i,v in enumerate(data)),
                            sum(v*math.sin(2*math.pi*hz*i/16000) for i,v in enumerate(data))))/len(data)
    assert amplitude(1.6,880)>.04 and amplitude(2.1,440)>.04
    assert amplitude(.1,880)<.001 and amplitude(3,440)<.001
    for name,color in [('logo','white'),('logo_b','yellow')]:
        path=root/f'{name}.png';run([args.ffmpeg,'-v','error','-f','lavfi','-i',f'color=c={color}:s=64x64','-frames:v','1',path])
        p=call('status',project=str(project));call('import',project=str(project),source=str(path),id=name,
            expectedRevision=p['revision'],key='import-'+name)
    def studio(action,**fields):
        revision=call('status',project=str(project))['revision']
        return call('studio',project=str(project),edit={'action':action,**fields},expectedRevision=revision,key=f'{action}-{revision}')
    studio('layer',sequence='seq_main',id='brand_mark',assetId='logo',atMs=0,sourceStartMs=0,durationMs=4000,fit='contain')
    apply([{'op':'item.set','target':'brand_mark','params':{'property':'transform.scale','value':{'x':.1,'y':.1}}}],'scale-logo')
    studio('replace-layer',itemId='brand_mark',assetId='logo_b',sourceStartMs=0)
    after=call('render',project=str(project))
    def audio(path):return run([args.ffmpeg,'-v','error','-i',path,'-vn','-f','s16le','-'])
    assert audio(before['path'])==audio(after['path'])
    # Transition handles must be present in the immutable sources.
    apply([{'op':'sequence.add','params':{'id':'dissolve','name':'Dissolve','width':360,'height':640,'frameRate':{'numerator':30,'denominator':1}}},
           {'op':'track.add','params':{'id':'dissolve_v','sequence':'dissolve','type':'video'}},
           {'op':'clip.add','params':{'id':'left','asset':'a','track':'dissolve_v','at':t(0),'sourceIn':t(30),'duration':t(60)}},
           {'op':'clip.add','params':{'id':'right','asset':'b','track':'dissolve_v','at':t(60),'sourceIn':t(30),'duration':t(60)}},
           {'op':'transition.add','params':{'id':'xfade','sequence':'dissolve','left':'left','right':'right',
            'type':'video.transition.crossfade','duration':t(15),'handlePolicy':'reject'}}],'transition-handles')
    transition=call('render',project=str(project),sequence='dissolve')
    assert json.loads(Path(transition['manifest']).read_text())['verification']['expectedFrames']==120
    # Halfway through the dissolve both source colors must contribute.
    pixels=run([args.ffmpeg,'-v','error','-ss','2','-i',transition['path'],'-frames:v','1','-f','rawvideo','-pix_fmt','rgb24','-'])
    center=(320*360+180)*3;assert pixels[center]>30 and pixels[center+1]>25,pixels[center:center+3]
    (root/'summary.json').write_text(json.dumps({'passed':True,'checks':['measured J-cut incoming dialogue',
        'measured L-cut outgoing dialogue','no duplicate embedded audio','logo replacement preserves dialogue PCM',
        'crossfade handles and blended pixels/full decode']},indent=2))
    print('Timeline passed:',root/'summary.json')


if __name__=='__main__': main()
