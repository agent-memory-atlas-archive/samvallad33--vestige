'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');
const setup = require('../lib/vestige-memory-setup');
const { rpc } = require('../hooks/memory.cjs');
const BIN = process.env.VESTIGE_TEST_MCP_BIN;
test('native hooks: first-chat Maya, real Backfill receipt, all lifecycle timing, doctor', { skip: !BIN, timeout: 30000 }, async t => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'vestige-memory-native-'));
  for (const name of ['.claude','.codex','.gemini','.cursor']) fs.mkdirSync(path.join(home, name));
  const store = path.join(home, 'store');
  const env = { ...process.env, HOME: home, USERPROFILE: home, VESTIGE_DATA_DIR: store,
    OPERATOR_HOME: path.join(home,'.operator'),
    VESTIGE_HTTP_ENABLED: '0', VESTIGE_DASHBOARD_ENABLED: 'false', VESTIGE_AUTOPILOT_ENABLED: '0', VESTIGE_BACKFILL_AUTOFIRE: '1', VESTIGE_MCP_PATH: BIN, VESTIGE_HOOK_DIAGNOSTICS: '1' };
  const p = setup.plan({ home, platform: process.platform, env }); setup.apply(p);
  const config = path.join(p.base, 'config.json');
  const c = JSON.parse(fs.readFileSync(config)); c.binaryArgs = ['--no-http','--data-dir',store]; fs.writeFileSync(config, JSON.stringify(c));
  const mcp = rpc(BIN, ['--no-http','--data-dir',store], 20000, env); t.after(() => mcp.close()); await mcp.ready();
  await mcp.tool('session_start',{memory_hook:{limit:3}});
  t.diagnostic(`owner endpoint available: ${fs.existsSync(path.join(store,'.serve.endpoint'))}; diagnostics: ${mcp.diagnostics()}`);
  const times = [];
  const hook = (event, prompt = '') => {
    const before = performance.now();
    const r = spawnSync(process.execPath, [path.join(p.base,'hooks','memory.cjs'), '--config',config,'--client','claude','--event',event],
      { input: JSON.stringify({ session_id:'maya-birthday',prompt }), env, encoding:'utf8', timeout:2000 });
    const elapsed = performance.now()-before; times.push({ event,ms:Number(elapsed.toFixed(2)) });
    assert.equal(r.status,0,r.stderr); assert.equal(r.stderr,'');
    if (elapsed >= 300) t.diagnostic(JSON.stringify({home,event,stdout:r.stdout,errors:fs.existsSync(path.join(p.base,'errors.log')) ? fs.readFileSync(path.join(p.base,'errors.log'),'utf8') : null}));
    assert(elapsed < 300, `${event}: ${elapsed}ms`);
    return JSON.parse(r.stdout);
  };
  hook('session');
  hook('prompt','Maya felt off after dinner at Bangkok Garden'); hook('turn');
  let packet = await mcp.tool('session_start',{memory_hook:{prompt:'Maya',limit:8}});
  const reaction = packet.memories.find(n=>n.content.includes('Bangkok Garden'));
  assert(reaction,JSON.stringify(packet)); await mcp.tool('memory',{action:'demote',id:reaction.id});
  await mcp.tool('smart_ingest',{content:'Liam felt off after dinner at Bangkok Garden',entities:['person:Liam','place:Bangkok Garden']});
  await mcp.tool('smart_ingest',{content:'The bicycle tire was repaired',entities:['service:bicycle']});
  // Another connected agent may have saved the words without explicit metadata.
  await mcp.tool('smart_ingest',{content:'Maya has a peanut allergy'});
  hook('prompt','Maya has a peanut allergy'); hook('turn');
  const answer = hook('prompt','where should we take Maya for her birthday dinner?');
  const context = answer.hookSpecificOutput?.additionalContext;
  assert(context,`birthday recall returned no context; ${fs.existsSync(path.join(p.base,'errors.log'))?fs.readFileSync(path.join(p.base,'errors.log'),'utf8'):'no hook errors'}`);
  assert(context.includes('Bangkok Garden'),context); assert(!context.includes('Liam'),context);
  assert(context.includes('backfill_receipt'),context);
  assert.equal(JSON.parse(context.split('\n').find(line=>line.startsWith('{'))).id,reaction.id,'raised reaction ranks first, including when the trigger is an echo');
  const receipts = await mcp.tool('backfill',{action:'receipts'});
  const receipt = receipts.receipts.find(r=>r.memory_id===reaction.id);
  assert.equal(receipt.shared_entity,'person:Maya'); assert.match(receipt.reason,/new health fact about Maya/);
  const recorded = fs.readFileSync(path.join(p.base,'receipts.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
  const last = recorded.at(-1); assert(last.trigger.matches.some(m=>m.category==='questions'&&m.phrase==='where'));
  assert(last.trigger.matches.some(m=>m.category==='entity'&&m.phrase==='person:Maya'));
  assert(last.backfill_receipts.includes(receipt.receipt_id));
  hook('compact'); hook('end');
  const doctor = spawnSync(process.execPath,[path.join(__dirname,'../bin/init.js'),'doctor','--json'],{env,encoding:'utf8',timeout:5000});
  assert.equal(doctor.status,0,doctor.stdout+doctor.stderr); const health=JSON.parse(doctor.stdout); assert(health.checks.every(c=>c.ok),doctor.stdout);
  t.diagnostic(JSON.stringify({hook_wall_times:times,memory_count:health.memory_count,receipt:receipt.receipt_id,doctor:health.checks}));
});

test('free Lite: real guarded STOP becomes Strata memory through the bundled reflex', { skip: !BIN, timeout:30000 }, async t => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(),'vestige-lite-native-'));
  const operatorHome = path.join(home,'.operator'); const gate = path.join(operatorHome,'gate','operator-gate.py');
  fs.mkdirSync(path.dirname(gate),{recursive:true}); fs.mkdirSync(path.join(home,'.claude'));
  fs.copyFileSync(path.join(__dirname,'../hooks/operator-lite.py'),gate); fs.writeFileSync(path.join(operatorHome,'mode'),'enforce\n');
  const store = path.join(home,'store'); const env = {...process.env,HOME:home,USERPROFILE:home,OPERATOR_HOME:operatorHome,VESTIGE_DATA_DIR:store,VESTIGE_MCP_PATH:BIN,VESTIGE_HTTP_ENABLED:'0',VESTIGE_DASHBOARD_ENABLED:'false',VESTIGE_AUTOPILOT_ENABLED:'0',OPERATOR_STOP_MEMORY_BUDGET:'3'};
  const p = setup.plan({home,platform:process.platform,env,guard:true}); setup.apply(p);
  const mcp=rpc(BIN,['--no-http','--data-dir',store],20000,env); t.after(()=>mcp.close()); await mcp.ready(); await mcp.tool('session_start',{memory_hook:{limit:3}});
  const guardBase=path.join(operatorHome,'state','vestige-memory');
  const memoryConfig=JSON.parse(fs.readFileSync(path.join(p.base,'config.json')));
  memoryConfig.python='/does/not/exist'; memoryConfig.gate='/fake/guard';
  fs.writeFileSync(path.join(p.base,'config.json'),JSON.stringify(memoryConfig));
  for (const file of [path.join(guardBase,'config.json'),path.join(guardBase,'guard.cjs')]) {
    const write=spawnSync(process.execPath,[path.join(guardBase,'guard.cjs'),'--config',path.join(guardBase,'config.json'),'--client','claude'],{input:JSON.stringify({hook_event_name:'PreToolUse',session_id:'free-loop-fixture',cwd:home,tool_name:'Write',tool_input:{file_path:file,content:'bypass the guard'}}),env,encoding:'utf8',timeout:15000});
    assert.equal(write.status,2,`Guard runtime must protect ${path.basename(file)}: ${write.stdout} ${write.stderr}`);
  }
  const payload={hook_event_name:'PreToolUse',session_id:'free-loop-fixture',cwd:home,tool_name:'Bash',tool_input:{command:'git push --force origin main'}};
  const stopped=spawnSync(process.execPath,[path.join(guardBase,'guard.cjs'),'--config',path.join(guardBase,'config.json'),'--client','claude'],{input:JSON.stringify(payload),env,encoding:'utf8',timeout:15000});
  assert.equal(stopped.status,2,stopped.stdout+stopped.stderr);
  const files=fs.readdirSync(path.join(operatorHome,'receipts')).filter(f=>f.endsWith('.jsonl'));
  const rows=files.flatMap(f=>fs.readFileSync(path.join(operatorHome,'receipts',f),'utf8').trim().split('\n').map(JSON.parse));
  const receipt=rows.find(r=>r.decision==='STOP' && r.commitment==='OP-004'); assert(receipt,JSON.stringify(rows));
  const config=JSON.parse(fs.readFileSync(path.join(guardBase,'config.json')));
  const registration=JSON.parse(fs.readFileSync(path.join(home,'.claude','settings.json'))).hooks.PostToolUse.find(g=>g.hooks[0].command.includes('stop-memory.cjs')).hooks[0];
  assert.equal(registration.timeout,1);
  const reflexStart=performance.now();
  const reflex=spawnSync(registration.command,{shell:true,input:JSON.stringify({...payload,hook_event_name:'PostToolUse'}),env,encoding:'utf8',timeout:2000});
  assert(performance.now()-reflexStart<300,'registered memory relay stays below 300ms');
  assert.equal(reflex.status,0,reflex.stderr);
  const recalled=await mcp.tool('recall',{handle:`op-receipt-${receipt.digest.slice(0,16)}`});
  assert.equal(recalled.nodes.length,1,JSON.stringify(recalled));
  const rule=await mcp.tool('recall',{handle:`op-rule-${receipt.commitment}`}); assert.equal(rule.nodes.length,1);
  const walk=await mcp.tool('causal_walk',{start_points:[{kind:'logged_write',node_id:recalled.nodes[0].id}],promote:false});
  assert(walk.causes.some(c=>c.id===rule.nodes[0].id),JSON.stringify(walk));
  // Bounded passes may leave other distinct stops pending. Drain them before
  // asserting replay is stable; every receipt must resolve to exactly one node.
  const stoppedRows=rows.filter(r=>r.decision==='STOP');
  for (let attempt=0;attempt<8;attempt++) {
    const copies=await Promise.all(stoppedRows.map(row=>mcp.tool('recall',{handle:`op-receipt-${row.digest.slice(0,16)}`})));
    if (copies.every(value=>value.nodes?.length===1)) break;
    const start=performance.now();
    spawnSync(registration.command,{shell:true,input:JSON.stringify({...payload,hook_event_name:'PostToolUse'}),env,encoding:'utf8',timeout:2000});
    assert(performance.now()-start<300,`continuation relay: ${(performance.now()-start).toFixed(2)}ms must be below 300ms`);
  }
  for (const row of stoppedRows) assert.equal((await mcp.tool('recall',{handle:`op-receipt-${row.digest.slice(0,16)}`})).nodes.length,1,'all stopped calls eventually enter Strata exactly once');
  const count=(await mcp.tool('session_start',{memory_hook:{limit:3}})).memory_count;
  spawnSync(registration.command,{shell:true,input:JSON.stringify({...payload,hook_event_name:'PostToolUse'}),env,encoding:'utf8',timeout:2000});
  assert.equal((await mcp.tool('session_start',{memory_hook:{limit:3}})).memory_count,count,'replaying the reflex is idempotent');
  t.diagnostic(JSON.stringify({rule:receipt.commitment,gate_receipt:receipt.digest.slice(0,16),stop_memory:recalled.nodes[0].id,rule_memory:rule.nodes[0].id}));
});
