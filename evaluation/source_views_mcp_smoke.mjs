// Qualify new source views through the official MCP SDK on the Python fixture.
import { pathToFileURL } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
const [avw, ffmpeg, ffprobe, root, sdkRoot] = process.argv.slice(2);
if (!sdkRoot) throw new Error('Usage: source_views_mcp_smoke.mjs AVW FFMPEG FFPROBE EXISTING_ROOT SDK_ROOT');
const { Client } = await import(pathToFileURL(path.join(sdkRoot,'dist/esm/client/index.js')));
const { StdioClientTransport } = await import(pathToFileURL(path.join(sdkRoot,'dist/esm/client/stdio.js')));
const summaryFile=path.join(root,'summary.json');
const summary=JSON.parse(fs.readFileSync(summaryFile));
if (!summary.passed) throw new Error('Python source-view checks must pass first');
const project=path.join(root,'project');
const client=new Client({name:'source-views-conformance',version:'1.0.0'});
const transport=new StdioClientTransport({command:avw,args:['--ffmpeg',ffmpeg,'--ffprobe',ffprobe,'mcp','--root',root],stderr:'inherit'});
const ledger=[];
async function call(name,args) {
  const response=await client.callTool({name,arguments:args});
  const envelope=response.structuredContent;
  ledger.push({name,args,ok:envelope.ok,contentTypes:response.content.map(c=>c.type)});
  fs.writeFileSync(path.join(root,'mcp-commands.json'),JSON.stringify(ledger,null,2));
  if (!envelope.ok || response.isError) throw new Error(JSON.stringify(envelope));
  return {value:envelope.result,response};
}
try {
  await client.connect(transport);
  const tools=await client.listTools();
  if (!tools.tools.some(t=>t.name==='avw-media')) throw new Error('media tool missing');
  const image=await call('avw-media',{kind:'storyboard',project,jobId:summary.storyboardJobId});
  const png=image.response.content.find(c=>c.type==='image');
  if (!png || png.mimeType!=='image/png' || !Buffer.from(png.data,'base64').subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) throw new Error('invalid image content');
  if (image.value.revision!==1) throw new Error('frozen revision missing');
  if (!image.value.storyboard.tiles.every(t=>Number.isInteger(t.observedPts) && t.timeBase)) throw new Error('observed clocks missing');
  const request={command:'budget-preview',project,expectedRevision:summary.revision,key:'official-mcp-budget-preview',noLaunch:true,request:{assetId:'source',startMs:300,endMs:10300,maxDurationMs:10000,maxBytes:2000000,width:640,height:360}};
  const queued=(await call('avw',request)).value;
  const replay=spawnSync(avw,['--ffmpeg',ffmpeg,'--ffprobe',ffprobe,'request','-'],{input:JSON.stringify(request),encoding:'utf8',timeout:30000});
  if (replay.status!==0 || JSON.parse(replay.stdout).result.id!==queued.id) throw new Error('CLI/MCP replay mismatch');
  const worker=spawnSync(avw,['worker',project],{encoding:'utf8',timeout:180000});
  if (worker.status!==0) throw new Error(worker.stderr);
  const state=(await call('avw',{command:'job-status',project,id:queued.id})).value;
  if (state.state!=='succeeded') throw new Error(JSON.stringify(state));
  const artifact=(await call('avw',{command:'artifact',project,id:queued.id})).value;
  const report=JSON.parse(fs.readFileSync(artifact.path));
  if (!report.data.verification.durationWithinBudget || !report.data.verification.bytesWithinBudget) throw new Error('unverified caps');
  const audio=await call('avw-media',{kind:'audio-proxy',project,jobId:queued.id,startMs:0,durationMs:1000});
  if (!audio.response.content.some(c=>c.type==='audio')) throw new Error('audio missing');
  summary.officialMcpSdk=true;summary.cliMcpReplayEquivalent=true;summary.boundedPreviewAudioTransport=true;
  fs.writeFileSync(summaryFile,JSON.stringify(summary,null,2)+'\n');
  console.log('Source views official MCP checks passed');
} finally { await client.close(); }
