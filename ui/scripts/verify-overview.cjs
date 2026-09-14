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
  const pointer = async(selector, during) => {
    const point=await evaluate(`(()=>{const node=document.querySelector(${JSON.stringify(selector)});node.scrollIntoView({block:'center'});const r=node.getBoundingClientRect();return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}})()`);
    web.sendInputEvent({type:'mouseDown',button:'left',clickCount:1,...point});
    await during();
    web.sendInputEvent({type:'mouseUp',button:'left',clickCount:1,...point});
    await evaluate('new Promise(resolve=>setTimeout(resolve,30))');
  };
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
    await check('disclosure is an SVG with centered stable geometry','document.querySelector(".focus-run.expanded .disclosure-indicator.is-expanded svg path") && document.querySelector(".disclosure-indicator").textContent===""');
    await capture('setup-dark');
    await click('#optimization-start');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('launch replaces draft in-place and opens one Status panel','!qa.state.draft && document.querySelectorAll(".optimization-progress").length===1 && document.querySelector("[data-run-id=new-root] .focus-run-body")');
    await evaluate('window.__stableRunSpinner=document.querySelector(".optimization-progress .spinner")');
    await evaluate(`qa.setup.activity={startedAt:'2026-09-14T12:00:00Z',updatedAt:'2026-09-14T12:01:00Z',progress:{phase:'training',completed:40,total:100},events:[{at:'2026-09-14T12:01:00Z',progress:{phase:'training',completed:40,total:100}}]};qa.render()`);
    await check('live status exposes exact work and an unlabeled meter','document.querySelector("progress").value===40 && document.querySelector("progress").max===100 && !document.querySelector(".focus-status").textContent.includes("40 / 100") && document.querySelector(".optimization-progress .spinner")');
    await evaluate('window.__stableRunSpinner=document.querySelector(".optimization-progress .spinner")');
    await check('spinner lives in active stage, not a duplicated heading','document.querySelector(".optimization-progress-steps li.active .spinner") && !document.querySelector(".optimization-current-work") && !document.querySelector(".optimization-progress-title")');
    await check('elapsed and Stop share the activity heading','document.querySelector(".activity-heading [data-elapsed-start]") && [...document.querySelectorAll(".activity-heading button")].some(button=>button.textContent==="Stop")');
    await evaluate('window.__spinTime=window.__stableRunSpinner.getAnimations()[0].currentTime');
    await evaluate('new Promise(resolve=>setTimeout(resolve,120))');
    await evaluate('qa.render()');
    await check('spinner stays connected and advances across progress updates','window.__stableRunSpinner===document.querySelector(".optimization-progress .spinner") && window.__stableRunSpinner.getAnimations()[0].currentTime>window.__spinTime+80');
    await check('stage tracker follows the native task','document.querySelector(".optimization-progress-steps li.active").textContent==="Training"');
    await capture('status-dark');
    await evaluate(`qa.setup.liveProgress={phase:'verifying_file',subject:'model.safetensors',completed:8388608,total:16777216,unit:'bytes'};qa.setup.liveProgressAt=Date.parse('2026-09-14T12:02:00Z');qa.render()`);
    await check('native filename and meter live in Activity only','document.querySelector(".focus-events li .focus-event-copy").textContent.includes("model.safetensors") && !document.querySelector(".optimization-progress-detail") && document.querySelector(".optimization-live-progress").textContent.trim()===""');
    await check('checksum step does not move the stage backwards','document.querySelector(".optimization-progress-steps li.active").textContent==="Training"');
    await check('run stages precede and outweigh the current-step meter','document.querySelector(".optimization-progress-steps").compareDocumentPosition(document.querySelector(".optimization-live-progress")) & Node.DOCUMENT_POSITION_FOLLOWING && parseFloat(getComputedStyle(document.querySelector(".optimization-progress-steps li.active")).borderTopWidth) > parseFloat(getComputedStyle(document.querySelector(".optimization-live-progress progress")).height)');
    await capture('file-progress-dark');
    await evaluate(`window.__priorActivity=qa.setup.activity;qa.setup.liveProgress=undefined;qa.setup.liveProgressAt=undefined;
      qa.setup.activity={...qa.setup.activity,events:['checking_inputs','preparing_data','starting','training','saving_candidate','evaluating'].flatMap((stage,index)=>Array.from({length:140},(_,row)=>({at:new Date(Date.parse('2026-09-14T12:00:00Z')+(index*140+row)*1000).toISOString(),stage,progress:{phase:'checking_dataset',subject:stage+'-'+row+'.json'}})))};qa.render()`);
    for(const stage of ['checking_inputs','preparing_data','starting','training','saving_candidate','evaluating']) {
      await click('#overview-new-root-stage-'+stage);
      await check('all 140 activities remain selectable in '+stage,`document.querySelectorAll('.focus-events li').length===140 && [...document.querySelectorAll('.focus-events li')].every(row=>row.dataset.activityStage==='${stage}') && document.querySelector('.focus-events li:last-child').textContent.includes('${stage}-0.json')`);
      await check('ordinary activity is attributed to System in '+stage,`[...document.querySelectorAll('.focus-event-kind')].every(label=>label.childNodes[0].textContent==='System') && !document.querySelector('.focus-events').textContent.includes('Work') && !document.querySelector('.focus-events').textContent.includes('Now')`);
    }
    await click('#overview-new-root-stage-checking_inputs');
    await pointer('#overview-new-root-stage-preparing_data', async()=>{
      await evaluate(`window.__pressedStage=document.querySelector('#overview-new-root-stage-preparing_data');window.__liveList=document.querySelector('.focus-events');for(let i=0;i<3;i++)qa.render()`);
      await check('redraw does not detach a pressed stage or native scroll container',`window.__pressedStage===document.querySelector('#overview-new-root-stage-preparing_data') && window.__liveList===document.querySelector('.focus-events')`);
    });
    await check('one physical click switches stages despite intervening updates',`document.querySelector('#overview-new-root-stage-preparing_data').getAttribute('aria-pressed')==='true'`);
    await pointer('#overview-new-root-stage-evaluating', async()=>{
      await evaluate(`window.__pressedStage=document.querySelector('#overview-new-root-stage-evaluating');qa.setup.activity.events.push({at:'2026-09-14T13:00:00Z',stage:'saving_candidate',progress:{phase:'registering_candidate'}});qa.render()`);
      await check('stage transition also keeps the pressed button connected',`window.__pressedStage===document.querySelector('#overview-new-root-stage-evaluating')`);
    });
    await check('a physical click survives a change of active stage',`document.querySelector('#overview-new-root-stage-evaluating').getAttribute('aria-pressed')==='true'`);
    await evaluate(`qa.setup.activity.events.pop();qa.render()`);
    await click('#overview-new-root-stage-checking_inputs');
    await evaluate(`document.querySelector('.focus-events').scrollTop=400;document.querySelector('.focus-events').dispatchEvent(new Event('scroll'));qa.render()`);
    await check('polling retains the selected historical stage and its scroll','document.querySelector("#overview-new-root-stage-checking_inputs").getAttribute("aria-pressed")==="true" && document.querySelector(".focus-events").scrollTop===400 && !document.querySelector(".focus-event.current")');
    await click('#overview-new-root-stage-preparing_data');
    await click('#overview-new-root-stage-checking_inputs');
    await check('returning to a stage restores its own scroll','document.querySelector(".focus-events").scrollTop===400');
    await evaluate(`document.querySelector('.focus-events').scrollTop=0`);
    await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
    const thumb=await evaluate(`(()=>{const list=document.querySelector('.focus-events');const r=list.getBoundingClientRect();return {x:Math.round(r.right-4),y:Math.round(r.top+14)}})()`);
    web.sendInputEvent({type:'mouseDown',button:'left',clickCount:1,...thumb});
    await check('drag begins on the scrollbar thumb, not a page-down track click','document.querySelector(".focus-events").scrollTop===0');
    await evaluate('qa.render()');
    web.sendInputEvent({type:'mouseMove',x:thumb.x,y:thumb.y+120,button:'left'});
    await evaluate('new Promise(resolve=>setTimeout(resolve,30))');
    await evaluate('qa.render()');
    web.sendInputEvent({type:'mouseUp',button:'left',clickCount:1,x:thumb.x,y:thumb.y+120});
    await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
    await check('native scrollbar remains draggable through live redraws','document.querySelector(".focus-events").scrollTop>500');
    await evaluate(`document.querySelector('#overview-new-root-stage-checking_inputs').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}))`);
    await check('stage selection supports keyboard navigation','document.activeElement.id==="overview-new-root-stage-preparing_data" && document.querySelectorAll("[data-activity-stage=preparing_data]").length===140');
    await capture('stage-history-dark');
    await evaluate(`qa.state.activityViews.get('new-root').stage='preparing_data';qa.setup.activity={...qa.setup.activity,events:[
      {at:'2026-09-14T12:00:01Z',stage:'preparing_data',progress:{phase:'agent_analysis'},narrative:{origin:'agent',kind:'reasoning',summary:'Replace the ambiguous search row using the recorded failure.'}},
      {at:'2026-09-14T12:00:02Z',stage:'preparing_data',progress:{phase:'data_generation'},narrative:{origin:'generation',kind:'intent',summary:'Generating the requested search example.'}}
    ]};qa.render()`);
    await check('Agent and generator retain their actual origins inline',`[...document.querySelectorAll('.focus-event-kind')].map(node=>node.childNodes[0].textContent).join(',')==='Generation,Agent' && document.querySelector('.focus-events').textContent.includes('recorded failure')`);
    await capture('agent-generation-dark');
    await evaluate(`qa.setup.activity=window.__priorActivity;qa.state.activityViews.get('new-root').stage=undefined;qa.render()`);
    await evaluate(`qa.setup.liveProgress=undefined;qa.setup.liveProgressAt=undefined;qa.render()`);
    await evaluate('[...document.querySelectorAll(".optimization-status-controls button")].find(button=>button.textContent==="Stop").click()');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('stop preserves a resumable run','!qa.setup.running && [...document.querySelectorAll(".optimization-status-controls button")].some(button=>button.textContent==="Resume")');
    await evaluate(`qa.setup.run.state='materialization_failed';qa.setup.error=new Error('encoder task adapter failed: invalid training row');qa.setup.activity={startedAt:'2026-09-14T12:00:00Z',updatedAt:'2026-09-14T12:01:00Z',progress:{phase:'verifying_file',subject:'encoder-gym.json',completed:2400,total:2400,unit:'bytes'},failure:{code:'adapter',message:'encoder task adapter failed: invalid training row'},events:[{at:'2026-09-14T12:01:00Z',progress:{phase:'verifying_file',subject:'encoder-gym.json',completed:2400,total:2400,unit:'bytes'}}]};qa.render()`);
    await check('a run failure appears once and names the failed stage','document.querySelectorAll(".operation-failure").length===1 && document.querySelector(".optimization-progress-title strong").textContent==="Preparing data failed"');
    for(const width of [1040,760,390]){
      window.setContentSize(width,900);await capture('status-'+width);
      await check('no page overflow at '+width,'document.documentElement.scrollWidth<=innerWidth && document.getElementById("page").scrollWidth<=innerWidth');
    }
    window.setContentSize(1440,1000);
    await evaluate('document.documentElement.dataset.theme="light";qa.state.expanded="root";qa.state.tabs.set("root","report");qa.render()');await capture('report-light');
    await check('report green remains green under a rejected decision','document.querySelector(".focus-report h2").textContent==="REJECT" && document.querySelector("td.success")');
    await evaluate(`qa.runs.runs.find(run=>run.id==='root').outcome={kind:'baseline_retained'};qa.workspace.runs[0].candidates[0].model={key:'candidate'};qa.state.tabs.set('root','status');qa.render()`);
    await check('stopped candidate registration remains resumable after a decision','document.querySelector("[data-run-id=root] .optimization-progress").textContent.includes("Paused · Saving candidate") && [...document.querySelectorAll("[data-run-id=root] .optimization-status-controls button")].some(button=>button.textContent==="Resume")');
    await evaluate(`startFixture(${JSON.stringify({workspace,datasets:f.datasets,benchmark:f.benchmark,saved,roots:[root],holdPreparation:true})})`);
    await click('#overview-new-run'); await click('#optimization-start');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('Stop is visible while checking files before a run exists','!qa.setup.run && !!qa.setup.preparationId && [...document.querySelectorAll(".optimization-progress button")].some(button=>button.textContent==="Stop") && document.querySelector(".focus-events").textContent.includes("model.safetensors")');
    await evaluate('[...document.querySelectorAll(".optimization-progress button")].find(button=>button.textContent==="Stop").click()');
    await evaluate('new Promise(resolve=>setTimeout(resolve,50))');
    await check('early Stop returns to editable setup without a run or error','!qa.setup.run && !qa.setup.preparationId && !qa.setup.error && !document.querySelector(".focus-run .spinner") && !document.getElementById("optimization-start").disabled');
    console.log('Overview renderer verification complete.');
  }finally{window.destroy();app.quit()}
}).catch(error=>{console.error(error);app.exit(1)});
