// Compiler transport oracle over deliberately structural Wasm/SDK fixtures.
// These fixtures provide no application, identity authority or kernel proofs.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {createServer} from 'node:http';
import {createRequire} from 'node:module';
import {readFileSync,writeFileSync,mkdtempSync,mkdirSync,rmSync,existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import test,{after} from 'node:test';

const script=fileURLToPath(import.meta.url),repository=resolve(dirname(script),'../../../..');
const require=createRequire('/opt/lean4-prod/browser-tests/package.json');
assert.equal(require('playwright/package.json').version,'1.62.1');
const{chromium}=require('playwright');
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const work=mkdtempSync(join(tmpdir(),'lean4-prod-workspace-view-'));
after(()=>rmSync(work,{recursive:true,force:true}));
const canonical=value=>JSON.stringify(value);
function execute(program,args){const result=spawnSync(program,args,{cwd:repository,encoding:'utf8',timeout:60000,maxBuffer:8*1024*1024});assert.ifError(result.error);assert.equal(result.status,0,result.stdout+'\n'+result.stderr);return result.stdout;}
const metadata=JSON.parse(execute('cargo',['metadata','--manifest-path','rust/Cargo.toml','--no-deps','--locked','--offline','--format-version','1']));
const generator=join(metadata.target_directory,'debug/examples/workspace_view_fixture');
assert.ok(existsSync(generator),'run the owning workspace-view gate to build its compiler example');
const digest='11'.repeat(32); // Deliberately synthetic binding metadata, not a proof.
function leb(value){const bytes=[];do{const byte=value&127;value>>>=7;bytes.push(byte|(value?128:0));}while(value);return bytes;}
function section(tag,bytes){return[tag,...leb(bytes.length),...bytes];}
function wasm(pages){
  const exports=[3];for(const[name,kind,index]of[['memory',2,0],['holo_alloc',0,0],['holo_run',0,1]])exports.push(name.length,...Buffer.from(name),kind,index);
  return Buffer.from([0,97,115,109,1,0,0,0,
    ...section(1,[2,0x60,1,0x7f,1,0x7f,0x60,2,0x7f,0x7f,1,0x7e]),
    ...section(3,[2,0,1]),...section(5,[1,1,1,...leb(pages)]),...section(7,exports),
    ...section(10,[2,4,0,0x20,0,0x0b,4,0,0x42,0,0x0b])]);
}
function write(name,bytes){const file=join(work,name);mkdirSync(dirname(file),{recursive:true});writeFileSync(file,bytes,{flag:'wx'});return file;}
const marker=name=>name.replaceAll('-','_')+'Marker';
const sdk=[];
for(const[name,dependencies]of[
  ['identity',[]],['view-error',[]],['store',['identity']],['journal',['identity']],
  ['commands',['identity','journal']],['queries',['identity','journal']],
  ['view-dom',['identity','view-error']],['view-host',['identity','commands','queries','view-dom','view-error']],
]){
  const imports=dependencies.map(name=>`import {${marker(name)}} from './${name}.mjs';\n`).join('');
  const implementation=name==='view-host'?`
export async function openWorkspaceView(binding) {
  if (await binding.store.loadIdentity() === null) throw Error('structural fixture missing identity');
  globalThis.fixtureBindings = {headName: binding.headName, labels: [...binding.labels],
    roles: ['viewModule','commandModule','queryModule','journalModule'].map(name => binding[name] instanceof WebAssembly.Module)};
  let closed = false;
  return Object.freeze({dispatch(bytes) { if (closed) throw Error('fixture closed'); globalThis.fixtureIntent = [...bytes]; },
    close() { closed = true; globalThis.fixtureClosed = true; }});
}
`:'';
  // Initial BOM is legal JavaScript whitespace. Verify the loader preserves it
  // rather than silently normalizing the digest-verified source before import.
  const bytes=Buffer.from((name==='identity'?'\uFEFF':'')+`// ${name}: structural transport fixture only\n${imports}export const ${marker(name)} = true;\n${implementation}`);
  sdk.push({path:`sdk/browser/${name}.mjs`,file:write(`input/${name}.mjs`,bytes),sha256:hash(bytes),size:bytes.length});
}
sdk.sort((left,right)=>left.path<right.path?-1:1);
const guests=[];
for(const[role,module,entry,request_maximum,response_maximum,maximum_pages]of[
  ['view','View.Workspace.V1.Interaction','workspaceInteractionBytes',133728,71055,128],
  ['command','Browser.V1.WorkspaceCommand','workspaceCommandBytes',139873,74243,64],
  ['query','Browser.V1.WorkspaceQuery','workspaceQueryBytes',1166279,66803,512],
  ['journal','Browser.V1.WorkspaceJournal','workspaceJournalBytes',1235980,1166008,640],
]){const bytes=wasm(maximum_pages);guests.push({role,root:`PrismPM.Foundation.${module}.${entry}`,source_snapshot_sha256:digest,ir_sha256:digest,proof_sha256:digest,
  wasm_sha256:hash(bytes),file:write(`input/${role}.wasm`,bytes),request_maximum,response_maximum,maximum_pages});}
const labels=Object.fromEntries('action action0 action1 action2 action3 action4 asOf author body close closed conflict contributor event inputError members message messages next none offset owner pending principal reader ready refresh rejected replay result role select submit title total unavailable unknown workspace'.split(' ').map(key=>[key,key]));
labels.spec='prismpm/workspace-view-labels/1';labels.title='\uFEFFStructural fixture <script>not executable</script>';
const labelBytes=Buffer.from(canonical(Object.fromEntries(Object.entries(labels).sort(([a],[b])=>a<b?-1:1))));
const fixture={view:{model_id:digest,view_model_id:digest,source_snapshot_sha256:digest,labels_root:'Fixture.labels',labels_proof_sha256:digest,labels_sha256:hash(labelBytes),file:write('input/labels.json',labelBytes)},
  sdk_inventory_sha256:hash(Buffer.from(canonical(sdk.map(({path,sha256,size})=>({path,sha256,size}))))),sdk,guests};
const input=write('fixture.json',canonical(fixture)),output=join(work,'first'),second=join(work,'second');
execute(generator,[input,output]);execute(generator,[input,second]);
const manifestBytes=readFileSync(join(output,'workspace-assets.json')),manifest=JSON.parse(manifestBytes);
assert.deepEqual(manifestBytes,readFileSync(join(second,'workspace-assets.json')));
assert.equal(manifest.kind,'component');assert.equal(manifest.files.length,15);assert.equal(existsSync(join(output,'index.html')),false);
const assets=new Map(manifest.files.map(record=>{const bytes=readFileSync(join(output,record.path));assert.equal(hash(bytes),record.sha256);assert.equal(bytes.length,record.size);assert.deepEqual(bytes,readFileSync(join(second,record.path)));return[record.path,bytes];}));

async function browserCase(action,mutation){
  const requests=[];
  const server=createServer((request,response)=>{
    requests.push(request.url);const path=request.url.slice(1);
    if(!path){response.writeHead(200,{'content-type':'text/html','content-security-policy':"default-src 'none'; script-src 'self' blob: 'wasm-unsafe-eval'; connect-src 'self'; base-uri 'none'; form-action 'none'"}).end('<!doctype html><title>Structural component transport test</title><main id="root"></main>');return;}
    let bytes=assets.get(path);if(!bytes){response.writeHead(404).end();return;}
    if(path==='workspace.mjs'&&process.env.WORKSPACE_COMPONENT_MUTANT==='skip-digest'){
      const guard="hexadecimal(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))) !== record.sha256";
      assert.equal(bytes.toString().split(guard).length,2);bytes=Buffer.from(bytes.toString().replace(guard,'false'));
    }
    if(path==='workspace.mjs'&&process.env.WORKSPACE_COMPONENT_MUTANT==='strip-bom')bytes=Buffer.from(bytes.toString().replace(', ignoreBOM: true',''));
    if(path===mutation?.path){
      if(mutation.kind==='redirect'){response.writeHead(302,{location:'/labels.json'}).end();return;}
      if(mutation.kind==='stall'){response.writeHead(200,{'content-type':'text/javascript'});response.write(bytes.subarray(0,1));request.socket.once('close',()=>requests.push('ABORTED'));return;}
      if(mutation.kind==='short')bytes=bytes.subarray(0,bytes.length-1);
      else if(mutation.kind==='long')bytes=Buffer.concat([bytes,Buffer.of(0)]);
      else {bytes=Buffer.from(bytes);bytes[bytes.length-1]^=1;}
    }
    response.writeHead(200,{'content-type':path.endsWith('.mjs')?'text/javascript':path.endsWith('.wasm')?'application/wasm':'application/json','cache-control':'no-store'}).end(bytes);
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  let browser,timer;
  try{
    browser=await chromium.launch({headless:true});assert.equal(browser.version(),'151.0.7922.34');
    const page=await browser.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await Promise.race([action(page,requests),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('bounded browser fixture timeout')),25000);})]);assert.deepEqual(errors,[]);
  }finally{clearTimeout(timer);await browser?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
}
const closedError={name:'WorkspaceBootstrapError',code:'workspace-bootstrap-unavailable',message:'workspace-bootstrap-unavailable',keys:['code','name'],noCause:true};
test('deterministic component transport preserves BOM source and closed mount surface',async()=>{
  await browserCase(async(page,requests)=>{
    const result=await page.evaluate(async()=>{
      const NativeBlob=Blob,initial=[];globalThis.Blob=class extends NativeBlob{constructor(parts,options){super(parts,options);initial.push(parts[0].charCodeAt(0));}};
      const{mount}=await import('./workspace.mjs'),root=document.querySelector('#root');let loads=0;
      const api=await mount(root,{async loadIdentity(){loads++;return{structuralFixtureOnly:true};}},'fixture');
      await api.dispatch(Uint8Array.of(1,2,3));api.close();
      return{initial,loads,frozen:Object.isFrozen(api),keys:Object.keys(api).sort(),bindings:globalThis.fixtureBindings,intent:globalThis.fixtureIntent,closed:globalThis.fixtureClosed};
    });
    assert.equal(result.loads,1);assert.equal(result.initial.filter(value=>value===0xfeff).length,1);assert.equal(result.frozen,true);assert.deepEqual(result.keys,['close','dispatch']);
    assert.deepEqual(result.bindings,{headName:'fixture',labels:[...labelBytes],roles:[true,true,true,true]});assert.deepEqual(result.intent,[1,2,3]);assert.equal(result.closed,true);
    for(const asset of sdk)assert.equal(requests.filter(path=>path==='/'+asset.path).length,1,'verified bytes must not be refetched by imports');
  });
});
for(const path of [...sdk.map(asset=>asset.path),...guests.map(guest=>`guests/${guest.role}.wasm`),'labels.json'])test('altered asset '+path,async()=>{
  await browserCase(async page=>{
    const result=await page.evaluate(async()=>{const{mount}=await import('./workspace.mjs');let reads=0,error;try{await mount(document.querySelector('#root'),{loadIdentity(){reads++;throw Error('private');}},'fixture');}catch(value){error={name:value.name,code:value.code,message:value.message,keys:Object.keys(value).sort(),noCause:value.cause===undefined};}return{reads,error};});
    assert.deepEqual(result,{reads:0,error:closedError});
  },{path});
});
for(const kind of ['redirect','short','long'])test('bounded response '+kind,async()=>{
  await browserCase(async page=>{const code=await page.evaluate(async()=>{try{await(await import('./workspace.mjs')).mount(document.querySelector('#root'),{},'fixture');}catch(error){return error.code;}});assert.equal(code,closedError.code);},{path:'sdk/browser/identity.mjs',kind});
});
test('stalled real native response is cancelled at the fixed deadline',async()=>{
  await browserCase(async(page,requests)=>{
    const result=await page.evaluate(async()=>{const start=performance.now();let code;try{await(await import('./workspace.mjs')).mount(document.querySelector('#root'),{},'fixture');}catch(error){code=error.code;}return{code,elapsed:performance.now()-start};});
    assert.equal(result.code,closedError.code);assert.ok(result.elapsed>=14000&&result.elapsed<22000);await new Promise(resolve=>setTimeout(resolve,30));assert.ok(requests.includes('ABORTED'));
  },{path:'sdk/browser/identity.mjs',kind:'stall'});
});
test('invalid mount/lifecycle calls are closed typed failures without extra authority',async()=>{
  await browserCase(async page=>{
    const result=await page.evaluate(async()=>{
      const{mount}=await import('./workspace.mjs'),root=document.querySelector('#root'),store={async loadIdentity(){return{structuralFixtureOnly:true};}},errors=[];
      const reject=async call=>{try{await call();return null;}catch(value){return{name:value.name,code:value.code,message:value.message,keys:Object.keys(value).sort(),noCause:value.cause===undefined};}};
      errors.push(await reject(()=>mount(new Proxy({}, {getPrototypeOf(){throw{secret:'root'};}}),store,'fixture')));
      errors.push(await reject(()=>mount(document.createElement('main'),store,'fixture')));
      errors.push(await reject(()=>mount(root,store,'../outside')));
      errors.push(await reject(()=>mount(root,store,'fixture','extra')));
      const api=await mount(root,store,'fixture');errors.push(await reject(()=>mount(root,store,'fixture')));
      errors.push(await reject(()=>api.dispatch(Uint8Array.of(1),'extra')));errors.push(await reject(()=>api.close('extra')));
      api.close();errors.push(await reject(()=>api.dispatch(Uint8Array.of(1))));
      const reopened=await mount(root,store,'fixture');reopened.close();return errors;
    });
    assert.equal(result.length,8);for(const error of result)assert.deepEqual(error,closedError);
  });
});
test('store/crypto/compiler/Blob failures cannot expose asynchronous exception payloads',async()=>{
  await browserCase(async page=>{
    const result=await page.evaluate(async()=>{
      const{mount}=await import('./workspace.mjs'),root=document.querySelector('#root'),errors=[];
      const reject=async call=>{try{await call();return null;}catch(value){return{name:value.name,code:value.code,message:value.message,keys:Object.keys(value).sort(),noCause:value.cause===undefined};}};
      errors.push(await reject(()=>mount(root,{async loadIdentity(){throw{privateMailbox:'secret'};}},'fixture')));
      for(const[owner,key]of[[crypto.subtle,'digest'],[WebAssembly,'compile'],[URL,'createObjectURL']]){
        const original=owner[key];owner[key]=()=>{throw{privatePayload:'secret'};};
        try{errors.push(await reject(()=>mount(root,{},'fixture')));}finally{owner[key]=original;}
      }
      return errors;
    });
    assert.equal(result.length,4);for(const error of result)assert.deepEqual(error,closedError);
  });
});
test('implementation mutants fail their owning loader regressions',()=>{
  for(const[mutant,pattern]of[['skip-digest','altered asset sdk/browser/identity'],['strip-bom','preserves BOM source']]){
    const environment=Object.fromEntries(Object.entries(process.env).filter(([key])=>key!=='NODE_TEST_CONTEXT'));
    const result=spawnSync(process.execPath,['--test','--test-name-pattern='+pattern,script],{cwd:repository,encoding:'utf8',timeout:30000,maxBuffer:1024*1024,env:{...environment,WORKSPACE_COMPONENT_MUTANT:mutant}});
    assert.ifError(result.error);assert.notEqual(result.status,0,'actual implementation mutant survived '+mutant);assert.match(result.stdout,/not ok 1/);assert.match(result.stdout,/# fail 1/);assert.match(result.stdout,/ERR_ASSERTION/);
  }
});
