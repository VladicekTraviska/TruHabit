import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import {tmpdir,hostname} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {acquireOperationLock} from '../operation-lock.mjs';

async function fixture(t){const root=await mkdtemp(join(tmpdir(),'truhabit-lock-'));t.after(()=>{assert.ok(resolve(root).startsWith(resolve(tmpdir())+sep));return rm(root,{recursive:true,force:true});});return join(root,'grant.lock');}
test('two concurrent acquisitions admit exactly one operator and release is idempotent',async t=>{
  const path=await fixture(t);
  const attempts=await Promise.allSettled([acquireOperationLock(path),acquireOperationLock(path)]);
  assert.equal(attempts.filter(v=>v.status==='fulfilled').length,1);
  assert.equal(attempts.filter(v=>v.status==='rejected'&&v.reason.code==='OPERATION_LOCKED').length,1);
  const unlock=attempts.find(v=>v.status==='fulfilled').value;
  await unlock();await unlock();
  const next=await acquireOperationLock(path);await next();
});
test('a SIGKILL owner is recovered without replacing a recorded operation',async t=>{
  const path=await fixture(t),record=join(path,'..','signed-operation.json');
  await writeFile(record,'{"request":"same-request","signed_bytes":"unchanged"}');
  const module=new URL('../operation-lock.mjs',import.meta.url).href;
  const child=spawn(process.execPath,['--input-type=module','-e',`import {acquireOperationLock} from ${JSON.stringify(module)};await acquireOperationLock(process.argv[1]);console.log('HELD');setInterval(()=>{},1000);`,path],{stdio:['ignore','pipe','pipe']});
  t.after(()=>{if(child.exitCode===null)child.kill('SIGKILL');});
  await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>reject(Error('Lock child did not start')),5000);
    child.stdout.once('data',value=>{clearTimeout(timer);value.toString().includes('HELD')?resolve():reject(Error('Unexpected child output'));});
    child.once('error',error=>{clearTimeout(timer);reject(error);});
  });
  await assert.rejects(()=>acquireOperationLock(path),{code:'OPERATION_LOCKED'});
  const exited=once(child,'exit');child.kill('SIGKILL');await exited;
  const unlock=await acquireOperationLock(path);
  assert.equal(await readFile(record,'utf8'),'{"request":"same-request","signed_bytes":"unchanged"}');
  await unlock();
});
test('foreign, malformed, and incomplete locks are never evicted',async t=>{
  const path=await fixture(t);await mkdir(path);
  await assert.rejects(()=>acquireOperationLock(path),{code:'OPERATION_LOCKED'});
  await writeFile(join(path,'owner.json'),'invalid');
  await assert.rejects(()=>acquireOperationLock(path),{code:'OPERATION_LOCKED'});
  await writeFile(join(path,'owner.json'),JSON.stringify({pid:2147483647,hostname:hostname()+'-foreign',nonce:'a'.repeat(36)}));
  await assert.rejects(()=>acquireOperationLock(path),{code:'OPERATION_LOCKED'});
});
test('an interrupted recovery guard prevents a fresh operation',async t=>{
  const path=await fixture(t);await mkdir(path+'.recovery');
  await assert.rejects(()=>acquireOperationLock(path),{code:'OPERATION_LOCKED'});
});
