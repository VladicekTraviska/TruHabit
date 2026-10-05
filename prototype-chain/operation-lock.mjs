// Process-owned locks for manual distribution. A verified dead owner is recovered
// under a separate recovery guard; a live/unknown/foreign owner is never evicted.
import {mkdir,readFile,writeFile,rename,unlink,rmdir,access} from 'node:fs/promises';
import {hostname} from 'node:os';
import {randomUUID} from 'node:crypto';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath} from 'node:url';

export class OperationLocked extends Error {
  constructor(message='Another operator is using this operation lock. Retry the same request later.') {super(message);this.code='OPERATION_LOCKED';}
}
const ownerPath=directory=>join(directory,'owner.json');
async function exists(path){try{await access(path);return true;}catch(error){if(error.code==='ENOENT')return false;throw error;}}
async function owner(directory){
  try{
    const value=JSON.parse(await readFile(ownerPath(directory),'utf8'));
    if(value.hostname!==hostname()||!Number.isSafeInteger(value.pid)||value.pid<1||!/^[-0-9a-f]{36}$/.test(value.nonce))return null;
    return value;
  }catch(error){if(error.code==='ENOENT'||error instanceof SyntaxError)return null;throw error;}
}
function isDead(value){
  if(!value)return false;
  try{process.kill(value.pid,0);return false;}catch(error){return error.code==='ESRCH';}
}
async function release(directory,nonce){
  const current=await owner(directory);
  if(!current||current.nonce!==nonce)throw new OperationLocked('Operation lock ownership changed; manual inspection is required.');
  await unlink(ownerPath(directory));
  await rmdir(directory);
}
async function initialize(directory){
  const value={pid:process.pid,hostname:hostname(),nonce:randomUUID(),created_utc:new Date().toISOString()};
  await mkdir(directory,{mode:0o700});
  await writeFile(ownerPath(directory),JSON.stringify(value)+'\n',{flag:'wx',mode:0o600});
  return value;
}

export async function acquireOperationLock(input){
  const directory=resolve(input instanceof URL?fileURLToPath(input):input);
  const recovery=directory+'.recovery';
  await mkdir(dirname(directory),{recursive:true,mode:0o700});
  for(let attempt=0;attempt<3;attempt++){
    if(await exists(recovery))throw new OperationLocked('Lock recovery is in progress or was interrupted; inspect the recovery owner before retrying.');
    try{
      const own=await initialize(directory);
      // An acquisition started just before recovery must not enter the operation.
      if(await exists(recovery)){await release(directory,own.nonce);throw new OperationLocked();}
      let released=false;
      return async()=>{if(!released){await release(directory,own.nonce);released=true;}};
    }catch(error){
      if(error.code!=='EEXIST')throw error;
    }
    const previous=await owner(directory);
    if(!isDead(previous))throw new OperationLocked('Operation lock has a live, foreign or unknown owner. It was not removed.');
    let guard;
    try{guard=await initialize(recovery);}catch(error){if(error.code==='EEXIST')throw new OperationLocked();throw error;}
    try{
      const current=await owner(directory);
      if(!current||current.nonce!==previous.nonce||!isDead(current))throw new OperationLocked();
      // Preserve stale metadata as evidence, never rename a fresh operation.
      await rename(directory,directory+'.stale-'+guard.nonce);
    }finally{await release(recovery,guard.nonce);}
  }
  throw new OperationLocked();
}
