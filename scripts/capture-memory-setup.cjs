#!/usr/bin/env node
'use strict';
// Reproduce the actual first-run transcript in a disposable HOME and Strata store.
// No host application is launched and no user configuration is changed.
const fs=require('fs'); const path=require('path'); const os=require('os');
const {spawnSync}=require('child_process');
const {rpc}=require('../packages/vestige-init/hooks/memory.cjs');
async function main() {
  const bin=process.env.VESTIGE_TEST_MCP_BIN; if(!bin) throw new Error('Set VESTIGE_TEST_MCP_BIN to the built v5.0.1 vestige-mcp');
  const home=fs.mkdtempSync(path.join(os.tmpdir(),'vestige-first-run-')); const store=path.join(home,'store');
  for(const name of ['.claude','.codex','.gemini','.cursor']) fs.mkdirSync(path.join(home,name));
  fs.writeFileSync(path.join(home,'.claude','settings.json'),'{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-existing-hook"}]}]}}\n');
  const env={...process.env,HOME:home,USERPROFILE:home,OPERATOR_HOME:path.join(home,'.operator'),VESTIGE_DATA_DIR:store,VESTIGE_MCP_PATH:bin,VESTIGE_HTTP_ENABLED:'0',VESTIGE_DASHBOARD_ENABLED:'false',VESTIGE_AUTOPILOT_ENABLED:'0',VESTIGE_BACKFILL_AUTOFIRE:'1'};
  const init=path.resolve('packages/vestige-init/bin/init.js');
  const started=performance.now(); const first=spawnSync(process.execPath,[init],{input:'\ny\n',env,encoding:'utf8',timeout:15000});
  if(first.status!==0) throw new Error(first.stdout+first.stderr);
  const firstMs=performance.now()-started; const lines=['ACTUAL FIXTURE CAPTURE — source candidate v5.0.1; paths redacted, not a published install.','The commands below run the local package entrypoint intended for npx after publication.','', '$ node packages/vestige-init/bin/init.js',first.stdout];
  const mcp=rpc(bin,['--no-http','--data-dir',store],20000,env);
  try {
    await mcp.ready(); await mcp.tool('session_start',{memory_hook:{limit:3}});
    const times=[]; let last;
    for(const [event,prompt] of [['session',''],['prompt','Maya felt off after dinner at Bangkok Garden'],['turn',''],['prompt','Maya has a peanut allergy'],['turn',''],['prompt','where should we take Maya for her birthday dinner?'],['compact',''],['end','']]) {
      const start=performance.now(); const run=spawnSync(process.execPath,[path.join(home,'.vestige','memory','hooks','memory.cjs'),'--config',path.join(home,'.vestige','memory','config.json'),'--client','claude','--event',event],{input:JSON.stringify({session_id:'first-chat-fixture',prompt}),env,encoding:'utf8',timeout:2000});
      const ms=performance.now()-start; times.push({event,ms:Number(ms.toFixed(2))});
      if(run.status!==0||ms>=300) throw new Error(`${event} failed or exceeded300ms: ${ms} ${run.stderr}`);
      if(event==='prompt'&&prompt.startsWith('where')) last=JSON.parse(run.stdout);
    }
    const context=last?.hookSpecificOutput?.additionalContext;
    if(!context?.includes('Bangkok Garden')||!context.includes('backfill_receipt')) throw new Error('Maya did not surface with her Backfill receipt');
    lines.push('FIRST CHAT — exact fixture utterances through the installed hooks:', 'Maya felt off after dinner at Bangkok Garden','Maya has a peanut allergy','where should we take Maya for her birthday dinner?','', 'Injected context:',context,'');
    const doctor=spawnSync(process.execPath,[init,'doctor'],{env,encoding:'utf8',timeout:5000});
    if(doctor.status!==0) throw new Error(doctor.stdout+doctor.stderr);
    lines.push('$ node packages/vestige-init/bin/init.js doctor',doctor.stdout,`First-run setup wall time: ${firstMs.toFixed(2)} ms (binary already installed; excludes human reading time).`,`Hook process wall times: ${JSON.stringify(times)}`,'','Fixture only: native client trust, reload and execution in each GUI/CLI remain separate checks.');
  } finally {mcp.close();}
  const out=lines.join('\n').split(home).join('<fixture-home>').split(bin).join('<built-v5.0.1/vestige-mcp>');
  fs.writeFileSync('docs/releases/v5.0.1-first-run.txt',out+'\n');
  console.log('Captured actual first run, native Maya receipts, doctor and wall timing to docs/releases/v5.0.1-first-run.txt');
}
main().catch(error=>{console.error(error.message);process.exitCode=1;});
