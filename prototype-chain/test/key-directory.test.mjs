import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {Keypair} from '@solana/web3.js';
import {localKey} from '../client.mjs';

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
