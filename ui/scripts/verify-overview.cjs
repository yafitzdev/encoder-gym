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
    const iteration = number => ({id:'iteration-'+number,number,createdAt:'2026-09-16T12:00:00Z',startingModelId:'original-model',inputDatasetVersionId:'input-'+number,benchmarkVersionId:'benchmark',experimentRunId:'experiment-'+number,qualifiedDatasetVersionId:'qualified-'+number,trainingDatasetVersionId:'training-'+number,modelId:'model-'+number,completed:number===1,noChange:false,selected:number===1,developmentPassed:number===1?true:null,
      checks:[{reportId:'report-'+number,suite:'development',metric:'loss',direction:'lower_is_better',baseline:.5,candidate:number===1?.3:.7,passed:number===1}]});
    const agentRoot={...root,id:'agent-root',experimentRunId:undefined,state:'agent_running',agentExecution:{lastSequence:1},iterations:[iteration(1),iteration(2)]};
    const agentWorkspace={...workspace,runs:[{...experiment,id:'experiment-1'},{...experiment,id:'experiment-2'}]};
    await evaluate(`startFixture(${JSON.stringify({workspace:agentWorkspace,datasets:f.datasets,benchmark:f.benchmark,saved,roots:[agentRoot]})});
      qa.runs.runningId='agent-root';qa.runs.activities.set('agent-root',{state:'progress',startedAt:'2026-09-16T12:00:00Z',updatedAt:'2026-09-16T12:05:00Z',stages:[],events:[
        ...Array.from({length:80},(_,i)=>({at:'2026-09-16T12:00:00Z',stage:'training',progress:{phase:'training',iteration:1,subject:'first-'+i}})),
        {at:'2026-09-16T12:05:00Z',stage:'training',progress:{phase:'training',iteration:2,subject:'second'}}]});qa.render()`);
    await check('Agent experiments stay inside one root with iteration selector above stages',`document.querySelectorAll('.focus-run').length===1 && document.querySelector('select[id$="-iteration"]').value==='2' && document.querySelector('.iteration-navigation').compareDocumentPosition(document.querySelector('.optimization-progress-steps')) & Node.DOCUMENT_POSITION_FOLLOWING`);
    const selectIteration=async number=>evaluate(`(()=>{const select=document.getElementById('overview-agent-root-iteration');select.value='${number}';select.dispatchEvent(new Event('change',{bubbles:true}))})()`);
    await selectIteration(1);
    await click('[id="overview-agent-root:1-stage-training"]');
    await check('historical iteration isolates activity and stops its spinner',`document.querySelectorAll('.focus-events li').length===80 && !document.querySelector('.focus-events').textContent.includes('second') && !document.querySelector('.optimization-progress .spinner') && [...document.querySelectorAll('.focus-event-kind')].every(node=>node.textContent==='Training')`);
    await evaluate(`document.querySelector('.focus-events').scrollTop=400;document.querySelector('.focus-events').dispatchEvent(new Event('scroll'));
      qa.runs.runs[0].iterations[1].completed=true;qa.runs.runs[0].iterations[1].developmentPassed=false;
      qa.runs.runs[0].iterations.push(${JSON.stringify(iteration(3))});
      qa.runs.activities.get('agent-root').events.push({at:'2026-09-16T12:06:00Z',stage:'training',progress:{phase:'training',iteration:3,subject:'third'}});qa.render()`);
    await check('new iterations do not steal selected stage or scroll',`document.getElementById('overview-agent-root-iteration').value==='1' && document.querySelector('.focus-events').scrollTop===400 && document.querySelector('[id="overview-agent-root:1-stage-training"]').getAttribute('aria-pressed')==='true'`);
    await selectIteration(2);await click('[id="overview-agent-root:2-stage-training"]');await selectIteration(1);
    await check('returning to an iteration restores its own stage and scroll',`document.querySelector('.focus-events').scrollTop===400 && document.querySelectorAll('.focus-events li').length===80`);
    await evaluate(`[...document.querySelectorAll('.iteration-navigation button')].find(node=>node.textContent==='Live iteration').click()`);
    await check('explicit Live iteration returns to current work',`document.getElementById('overview-agent-root-iteration').value==='3' && document.querySelector('.focus-events').textContent.includes('third') && !!document.querySelector('.optimization-progress .spinner')`);
    await click('#overview-agent-root-report');await selectIteration(2);
    await check('rejected iteration retains original baseline, model, dataset and report navigation',`document.querySelector('.focus-report h2').textContent==='Development · REJECT' && document.querySelectorAll('.focus-report td.danger').length===2 && document.querySelector('.focus-report').textContent.includes('original baseline')`);
    for(const [label,page,id,tab] of [['View model','model','model-2',undefined],['Dataset changes','dataset','qualified-2','changes'],['Training dataset','dataset','training-2','rows'],['Evaluation reports','benchmarks','benchmark','results']]) {
      await evaluate(`[...document.querySelectorAll('.focus-report button')].find(node=>node.textContent==='${label}').click()`);
      await check('ordinary viewer link: '+label,`lastNavigation.refreshProject===true && lastNavigation.page==='${page}' && lastNavigation.id==='${id}'${tab?` && lastNavigation.tab==='${tab}'`:''}`);
    }
    await capture('iteration-report-light');
    await selectIteration(1);
    await check('development selection is distinct from final approval',`document.querySelector('.focus-report h2').textContent==='Development · KEEP' && document.querySelector('.focus-report').textContent.includes('Final approval and promotion are separate') && document.querySelectorAll('.focus-report td.success').length===2`);
    await evaluate(`Object.assign(qa.runs.runs[0].iterations[2],{completed:true,noChange:true,developmentPassed:null,modelId:null,trainingDatasetVersionId:null,qualifiedDatasetVersionId:null,experimentRunId:null,checks:[]});qa.runs.runs[0].state='agent_completed';qa.runs.runningId=undefined;qa.render()`);
    await selectIteration(3);
    await check('no-change iteration has no fabricated verdict or model',`document.querySelector('.focus-report h2').textContent==='No change proposed' && ![...document.querySelectorAll('.focus-report button')].some(node=>node.textContent==='View model')`);
    await selectIteration(2);await click('#overview-run-agent-root');await click('#overview-run-agent-root');
    await check('iteration and Report selection survive collapsing the run',`document.getElementById('overview-agent-root-iteration').value==='2' && document.querySelector('.focus-report h2').textContent==='Development · REJECT'`);
    for(const width of [760,390]) {window.setContentSize(width,900);await capture('iteration-report-'+width);await check('iteration report fits '+width,`document.documentElement.scrollWidth<=innerWidth && [...document.querySelectorAll('.iteration-navigation > *, .focus-report .focus-controls > *')].every(node=>{const r=node.getBoundingClientRect();return r.left>=0 && r.right<=innerWidth})`)}
    await evaluate(`Object.assign(qa.runs.runs[0].iterations[2],{completed:false,noChange:false});qa.runs.runs[0].state='agent_failed';qa.render()`);
    await selectIteration(3);await click('#overview-agent-root-status');
    await check('failed iteration names its recorded native stage',`document.querySelector('.optimization-progress-title').textContent==='Training failed'`);
    await evaluate(`qa.runs.runs[0].state='agent_budget_exhausted';qa.render()`);
    await check('budget-stop iteration is terminal, not falsely paused',`document.querySelector('.optimization-progress-title').textContent==='Training budget exhausted' && ![...document.querySelectorAll('.optimization-status-controls button')].some(node=>node.textContent==='Resume')`);
    await evaluate(`qa.runs.runs[0].state='agent_running';qa.render()`);
    await check('reopened running state is unverified, not falsely paused or live',`document.querySelector('.focus-run-state').textContent==='Status unverified' && document.querySelector('.optimization-progress-title').textContent==='Execution status unverified · Training' && !document.querySelector('.focus-run .spinner')`);
    await check('reopened Agent runs expose durable Stop and explicit status refresh',`['Stop','Refresh status','Resume'].every(label=>[...document.querySelectorAll('.optimization-status-controls button')].some(node=>node.textContent===label && !node.disabled))`);
    await check('recovery controls fit a narrow window',`[...document.querySelectorAll('.optimization-status-controls button')].every(node=>{const r=node.getBoundingClientRect();return r.left>=0 && r.right<=innerWidth})`);
    await capture('recovery-390');
    await evaluate(`qa.runs.runs[0].state='agent_stopping';qa.runs.runningId='agent-root';qa.render()`);
    await check('Stop intent remains visible while owned work unwinds',`document.querySelector('.focus-run-state').textContent==='Stop requested' && document.querySelector('.optimization-progress-title').textContent==='Stop requested · Training' && !!document.querySelector('.optimization-progress .spinner')`);
    await evaluate(`qa.runs.runs[0].state='agent_paused';qa.runs.runs[0].agentExecution.lastSequence=3;qa.setup.run={...qa.runs.runs[0],state:'agent_running',agentExecution:{lastSequence:1}};qa.setup.running=true;qa.render()`);
    await check('durable pause beats stale setup state and an unsettled drive promise',`document.querySelector('[data-run-id="agent-root"]').dataset.runState==='agent_paused' && document.querySelector('.focus-run-state').textContent==='Paused' && !document.querySelector('.focus-run .spinner') && document.querySelector('.optimization-progress-title').textContent==='Paused · Training'`);
    await evaluate(`qa.setup.running=false;qa.setup.run=undefined;qa.runs.runningId=undefined;qa.runs.runs[0].state='agent_interrupted';qa.render()`);
    await check('interruption is not relabeled as a user pause',`document.querySelector('.focus-run-state').textContent==='Interrupted' && document.querySelector('.optimization-progress-title').textContent==='Interrupted · Training'`);
    console.log('Overview renderer verification complete.');
  }finally{window.destroy();app.quit()}
}).catch(error=>{console.error(error);app.exit(1)});
