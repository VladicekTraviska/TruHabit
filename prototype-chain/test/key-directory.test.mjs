import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {tmpdir} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {Keypair} from '@solana/web3.js';
import {localKey,oracleReadiness,prepare,balance,connection,NETWORK,PROGRAM,ORACLE} from '../client.mjs';

async function privateDirectory(t){
  const root=await mkdtemp(join(tmpdir(),'truhabit-oracle-readiness-'));
  const previous=process.env.TRUHABIT_KEY_DIR;
  process.env.TRUHABIT_KEY_DIR=root;
  t.after(async()=>{
    if(previous===undefined)delete process.env.TRUHABIT_KEY_DIR;else process.env.TRUHABIT_KEY_DIR=previous;
    assert.ok(resolve(root).startsWith(resolve(tmpdir())+sep));
    await rm(root,{recursive:true,force:true});
  });
  return root;
}

test('operator keys use explicit private directories; invalid/missing configuration is precise',async t=>{
  const root=await mkdtemp(join(tmpdir(),'truhabit-key-test-'));
  const previous=process.env.TRUHABIT_KEY_DIR;
  t.after(async()=>{
    if(previous===undefined)delete process.env.TRUHABIT_KEY_DIR;else process.env.TRUHABIT_KEY_DIR=previous;
    assert.ok(resolve(root).startsWith(resolve(tmpdir())+sep));
    await rm(root,{recursive:true,force:true});
  });
  process.env.TRUHABIT_KEY_DIR='relative-directory';
  await assert.rejects(()=>localKey('oracle'),/INVALID_KEY_DIRECTORY/);
  process.env.TRUHABIT_KEY_DIR=root;
  await assert.rejects(()=>localKey('oracle'),/CHAIN_ORACLE_NOT_CONFIGURED/);
  await assert.rejects(()=>localKey('../oracle'),/INVALID_KEY_ROLE/);
  const key=Keypair.fromSeed(new Uint8Array(32).fill(41));
  await writeFile(join(root,'prototype-oracle.json'),JSON.stringify(Array.from(key.secretKey)),{mode:0o600});
  assert.equal((await localKey('oracle')).publicKey.toBase58(),key.publicKey.toBase58());
});

test('oracle readiness distinguishes missing, wrong and malformed keys without secret diagnostics',async t=>{
  const root=await privateDirectory(t),file=join(root,'prototype-oracle.json');
  assert.deepEqual(await oracleReadiness(),{oracle_configured:false,oracle_reason:'CHAIN_ORACLE_NOT_CONFIGURED'});
  const key=Keypair.fromSeed(new Uint8Array(32).fill(42));
  await writeFile(file,JSON.stringify(Array.from(key.secretKey)));
  assert.deepEqual(await oracleReadiness(),{oracle_configured:false,oracle_reason:'WRONG_ORACLE_KEY'});
  for(const invalid of ['not json','{}','[]',JSON.stringify(Array(64).fill(256)),JSON.stringify(Array(64).fill(1.5)),JSON.stringify(Array(64).fill(null)),JSON.stringify(Array(64).fill(0)),' '.repeat(4097)]){
    await writeFile(file,invalid);
    const status=await oracleReadiness();
    assert.deepEqual(status,{oracle_configured:false,oracle_reason:'CHAIN_ORACLE_INVALID_KEY'});
    assert.equal(JSON.stringify(status).includes(root),false);
    await assert.rejects(()=>localKey('oracle'),/^Error: CHAIN_ORACLE_INVALID_KEY$/);
  }
  process.env.TRUHABIT_KEY_DIR='relative-private-path';
  assert.deepEqual(await oracleReadiness(),{oracle_configured:false,oracle_reason:'INVALID_KEY_DIRECTORY'});
});

test('an unreadable oracle is a fixed configuration code rather than a filesystem error',async t=>{
  const root=await privateDirectory(t);
  await mkdir(join(root,'prototype-oracle.json'));
  assert.deepEqual(await oracleReadiness(),{oracle_configured:false,oracle_reason:'CHAIN_ORACLE_UNREADABLE'});
});

test('oracle identity check returns only readiness for the authorized public identity',async t=>{
  const root=await privateDirectory(t),key=Keypair.fromSeed(new Uint8Array(32).fill(43));
  await writeFile(join(root,'prototype-oracle.json'),JSON.stringify(Array.from(key.secretKey)));
  // Exercise the identity gate without accessing an actual deployed signing key.
  t.mock.method(Keypair,'fromSecretKey',()=>({publicKey:ORACLE}));
  assert.deepEqual(await oracleReadiness(),{oracle_configured:true,oracle_reason:null});
});

test('missing or malformed oracle refuses deposit and verdict preparation before any RPC',async t=>{
  const root=await privateDirectory(t);
  let calls=0;t.mock.method(connection,'getGenesisHash',async()=>{calls++;throw Error('RPC_SHOULD_NOT_BE_CALLED');});
  for(const action of ['DEPOSIT','SUCCESS','FAILURE'])await assert.rejects(()=>prepare({},action),/CHAIN_ORACLE_NOT_CONFIGURED/);
  await writeFile(join(root,'prototype-oracle.json'),'invalid');
  for(const action of ['DEPOSIT','SUCCESS','FAILURE'])await assert.rejects(()=>prepare({},action),/CHAIN_ORACLE_INVALID_KEY/);
  assert.equal(calls,0);
});

test('wallet balance exposes local oracle readiness even while a deployed program is present',async t=>{
  await privateDirectory(t);
  t.mock.method(connection,'getGenesisHash',async()=>NETWORK);
  t.mock.method(connection,'getBalance',async()=>123);
  t.mock.method(connection,'getAccountInfo',async address=>address.equals(PROGRAM)?{executable:true}:null);
  const value=await balance(ORACLE.toBase58());
  assert.equal(value.oracle_configured,false);
  assert.equal(value.oracle_reason,'CHAIN_ORACLE_NOT_CONFIGURED');
  assert.equal(value.sol_lamports,123);
  assert.equal(value.amount_units,'0');
  assert.equal(Object.hasOwn(value,'key_directory'),false);
});

test('worker provides a local-only oracle probe with no key material or raw paths',async t=>{
  const root=await privateDirectory(t),file=join(root,'prototype-oracle.json');
  for(const contents of [null,'PRIVATE_TEST_INVALID_CONTENT']){
    if(contents!==null)await writeFile(file,contents);
    const child=spawnSync(process.execPath,[fileURLToPath(new URL('../worker.mjs',import.meta.url))],{input:JSON.stringify({operation:'oracle_readiness'}),encoding:'utf8',env:process.env});
    assert.equal(child.status,0,child.stderr);
    const output=JSON.parse(child.stdout);
    assert.deepEqual(output,{ok:true,result:{oracle_configured:false,oracle_reason:contents===null?'CHAIN_ORACLE_NOT_CONFIGURED':'CHAIN_ORACLE_INVALID_KEY'}});
    assert.equal(child.stdout.includes(root),false);
    assert.equal(child.stdout.includes('PRIVATE_TEST_INVALID_CONTENT'),false);
  }
});
