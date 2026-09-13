const { app, BrowserWindow } = require('electron');
const { mkdirSync, mkdtempSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const assert = require('node:assert/strict');
app.setPath('userData', mkdtempSync(join(tmpdir(), 'encoder-gym-overview-renderer-')));
app.whenReady().then(async()=>{
  const output=join(__dirname,'../artifacts/overview');mkdirSync(output,{recursive:true});
  const window=new BrowserWindow({show:false,width:1440,height:1000,webPreferences:{contextIsolation:true,nodeIntegration:false}});
  const web=window.webContents, evaluate=code=>web.executeJavaScript(code);
  const check=async(name,code)=>{assert.ok(await evaluate(code),name);console.log('PASS '+name)};
  const click=selector=>evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const capture=async name=>{await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');writeFileSync(join(output,name+'.png'),(await web.capturePage()).toPNG())};
  try{
    await window.loadFile(join(__dirname,'../tests/overview-renderer.html'));
    await check('renderer module loaded','window.fixtureReady');
    const {setupFixture}=await import('../tests/optimization-setup-fixture.mjs');
    const f=setupFixture(); f.benchmark.definition={suites:[{key:'generic_holdout',role:'development'}]};
    f.workspace.providerCatalog={id:'providers',providers:[{role:'advisor',endpoint:'https://api.deepseek.com',model:'deepseek-chat'},{role:'generation',endpoint:'https://yan.tail85512d.ts.net',model:'generator'}]};
    const saved=f.saved({id:'setup',inputs:f.inputs}).setup;
    const result={report:{suite:'generic_holdout'},checks:[{metric:'mrr',baseline:.895654,candidate:.905654},{metric:'recall_at_2',baseline:.906,candidate:.901},{metric:'loss',baseline:.32,candidate:.25}]};
    const experiment={id:'experiment',createdAt:'2026-09-12T12:00:00Z',baseline:{key:'Nomos baseline'},candidates:[{sequence:1,development:[result]}],directions:{mrr:'higher_is_better',recall_at_2:'higher_is_better',loss:'lower_is_better'},acceptance:{state:'unused'},decision:'retain_baseline',activity:[]};
    const root={id:'root',projectId:f.projectId,setupId:'setup',experimentRunId:'experiment',createdAt:'2026-09-12T12:00:00Z',state:'baseline_retained',lastSequence:5};
    const workspace={managed:f.workspace,runs:[experiment]};
    await evaluate(`startFixture(${JSON.stringify({workspace,datasets:f.datasets,benchmark:f.benchmark,saved,roots:[root]})})`);
    await check('one row per run and report is the default','document.querySelectorAll(".focus-run").length===1 && document.querySelector(".focus-report h2").textContent==="REJECT"');
    await check('positive and negative numbers honor metric direction','document.querySelectorAll(".focus-comparison td.success").length===4 && document.querySelectorAll(".focus-comparison td.danger").length===2');
    await capture('report-dark');
    await click('#overview-new-run');
    await check('setup has exactly five fields and no metadata','document.querySelectorAll(".focus-field").length===5 && !document.querySelector(".focus-field small") && !document.querySelector(".focus-field p")');
    await check('unavailable stages disabled without prose','document.getElementById("overview-draft-status").disabled && document.getElementById("overview-draft-report").disabled && !document.body.textContent.includes("Ready when you are")');
    await click('#overview-run-draft');await check('all runs can be collapsed','document.querySelectorAll(".focus-run-body").length===0');
    await click('#overview-run-draft');
    await check('selection survives collapsing','document.getElementById("optimization-dataset").value===qa.setup.datasetId');
    await capture('setup-dark');
    await click('#optimization-start');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('launch replaces draft in-place and opens one Status panel','!qa.state.draft && document.querySelectorAll(".optimization-progress").length===1 && document.querySelector("[data-run-id=new-root] .focus-run-body")');
    await evaluate(`qa.setup.activity={startedAt:'2026-09-14T12:00:00Z',updatedAt:'2026-09-14T12:01:00Z',progress:{phase:'training',completed:40,total:100},events:[{at:'2026-09-14T12:01:00Z',progress:{phase:'training',completed:40,total:100}}]};qa.render()`);
    await check('live status exposes exact work, counters and activity','document.querySelector("progress").value===40 && document.querySelector("progress").max===100 && document.querySelector(".focus-events").textContent.includes("40 / 100 steps") && document.querySelector(".optimization-progress .spinner")');
    await check('stage tracker follows the native task','document.querySelector(".optimization-progress-steps li.active").textContent==="Training"');
    await capture('status-dark');
    await evaluate('[...document.querySelectorAll(".focus-controls button")].find(button=>button.textContent==="Stop").click()');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('stop preserves a resumable run','!qa.setup.running && [...document.querySelectorAll(".focus-controls button")].some(button=>button.textContent==="Resume")');
    for(const width of [1040,760,390]){
      window.setContentSize(width,900);await capture('status-'+width);
      await check('no page overflow at '+width,'document.documentElement.scrollWidth<=innerWidth && document.getElementById("page").scrollWidth<=innerWidth');
    }
    window.setContentSize(1440,1000);
    await evaluate('document.documentElement.dataset.theme="light";qa.state.expanded="root";qa.state.tabs.set("root","report");qa.render()');await capture('report-light');
    await check('report green remains green under a rejected decision','document.querySelector(".focus-report h2").textContent==="REJECT" && document.querySelector("td.success")');
    console.log('Overview renderer verification complete.');
  }finally{window.destroy();app.quit()}
}).catch(error=>{console.error(error);app.exit(1)});
