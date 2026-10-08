import test from 'node:test';
import assert from 'node:assert/strict';
import {Keypair,Transaction,TransactionInstruction,SystemProgram} from '@solana/web3.js';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve,sep} from 'node:path';
import {connection,NETWORK,PROGRAM,MINT,ORACLE,RECIPIENT,digest,identity,validateSigned,verifyState,reconcile,prepare} from '../client.mjs';
import {TOKEN_PROGRAM_ID,getAssociatedTokenAddressSync} from '../spl.mjs';

const owner=Keypair.fromSeed(new Uint8Array(32).fill(17));
const other=Keypair.fromSeed(new Uint8Array(32).fill(18));
const c={id:'12345678-1234-4234-8234-123456789abc',owner:owner.publicKey.toBase58(),amount_units:5e6,starts_at:'2026-09-01T00:00:00Z',ends_at:'2026-09-01T00:10:00Z',upload_deadline:'2026-09-01T00:10:00Z',refund_after:'2026-09-01T00:15:00Z',terms_hash:'a'.repeat(64)};
function transaction(){return new Transaction({feePayer:owner.publicKey,recentBlockhash:other.publicKey.toBase58()}).add(SystemProgram.transfer({fromPubkey:owner.publicKey,toPubkey:other.publicKey,lamports:123}));}
function account(status=0){const b=Buffer.alloc(130);digest('account:Commitment').copy(b,0,0,8);identity(c).id.copy(b,8);owner.publicKey.toBuffer().copy(b,24);b.writeBigUInt64LE(5_000_000n,56);['starts_at','ends_at','upload_deadline','refund_after'].forEach((k,i)=>b.writeBigInt64LE(BigInt(Date.parse(c[k])/1000),64+8*i));Buffer.from(c.terms_hash,'hex').copy(b,96);b[129]=status;return {owner:PROGRAM,data:b};}
function vault(){const b=Buffer.alloc(165);MINT.toBuffer().copy(b);identity(c).commitment.toBuffer().copy(b,32);b.writeBigUInt64LE(5_000_000n,64);b[108]=1;return {owner:TOKEN_PROGRAM_ID,data:b};}
function chain(t,state){t.mock.method(connection,'getGenesisHash',async()=>NETWORK);t.mock.method(connection,'getAccountInfo',async address=>address.equals(identity(c).commitment)?state:vault());}
async function privateKeys(t){
  const root=await mkdtemp(join(tmpdir(),'truhabit-client-key-test-')),previous=process.env.TRUHABIT_KEY_DIR;
  process.env.TRUHABIT_KEY_DIR=root;
  t.after(async()=>{
    if(previous===undefined)delete process.env.TRUHABIT_KEY_DIR;else process.env.TRUHABIT_KEY_DIR=previous;
    assert.ok(resolve(root).startsWith(resolve(tmpdir())+sep));await rm(root,{recursive:true,force:true});
  });
  return root;
}
function settlement(status=4,signature='external-settlement'){
  const {commitment,vault:vaultKey}=identity(c),action=status-1;
  const recipient=action===1?RECIPIENT:owner.publicKey;
  const actor=action<2?ORACLE:action===2?owner.publicKey:other.publicKey;
  const destination=getAssociatedTokenAddressSync(MINT,recipient);
  const outer=new Transaction({feePayer:actor,recentBlockhash:other.publicKey.toBase58()}).add(new TransactionInstruction({programId:PROGRAM,keys:[
    {pubkey:actor,isSigner:true,isWritable:false},{pubkey:MINT,isSigner:false,isWritable:false},
    {pubkey:commitment,isSigner:false,isWritable:true},{pubkey:vaultKey,isSigner:false,isWritable:true},
    {pubkey:destination,isSigner:false,isWritable:true},{pubkey:TOKEN_PROGRAM_ID,isSigner:false,isWritable:false}
  ],data:Buffer.concat([digest('global:settle').subarray(0,8),Buffer.from([action])])}));
  const message=outer.compileMessage(),index=key=>message.accountKeys.findIndex(k=>k.equals(key));
  const data=Buffer.alloc(10);data[0]=12;data.writeBigUInt64LE(BigInt(c.amount_units),1);data[9]=6;
  const inner=new Transaction({feePayer:other.publicKey,recentBlockhash:other.publicKey.toBase58()}).add(new TransactionInstruction({programId:TOKEN_PROGRAM_ID,keys:[],data})).compileMessage().instructions[0].data;
  const balance=(address,authority,amount)=>({accountIndex:index(address),mint:MINT.toBase58(),owner:authority.toBase58(),uiTokenAmount:{amount:String(amount),decimals:6}});
  return {transaction:{message,signatures:[signature]},slot:124,meta:{err:null,
    preTokenBalances:[balance(vaultKey,commitment,5_000_000),balance(destination,recipient,100_000_000)],
    postTokenBalances:[balance(vaultKey,commitment,0),balance(destination,recipient,105_000_000)],
    innerInstructions:[{index:0,instructions:[{programIdIndex:index(TOKEN_PROGRAM_ID),accounts:[index(vaultKey),index(MINT),index(destination),index(commitment)],data:inner}]}]}};
}

test('signed bytes bind the exact message, all signatures and a stable transaction ID',()=>{
  const tx=transaction();const payload={message:tx.serializeMessage().toString('base64')};tx.sign(owner);
  const bytes=tx.serialize().toString('base64');const verified=validateSigned(payload,bytes);assert.equal(verified.transaction,bytes);assert.equal(validateSigned(payload,bytes).signature,verified.signature);
  const changed=transaction();changed.instructions[0].data[4]^=1;changed.sign(owner);assert.throws(()=>validateSigned(payload,changed.serialize().toString('base64')),/TRANSACTION_MISMATCH/);
  const corrupted=Buffer.from(bytes,'base64');corrupted[2]^=1;assert.throws(()=>validateSigned(payload,corrupted.toString('base64')),/TRANSACTION_MISMATCH/);
  const missing=transaction();assert.throws(()=>validateSigned(payload,missing.serialize({requireAllSignatures:false}).toString('base64')),/TRANSACTION_MISMATCH/);
});
test('chain verification binds owner, program, mint, amount, terms, status and vault',async t=>{
  const good=account();chain(t,good);await verifyState(c,0);
  for(const index of [0,8,24,56,64,96,129]) {good.data[index]^=1;await assert.rejects(()=>verifyState(c,0),/CHAIN_TERMS_MISMATCH/);good.data[index]^=1;}
  good.owner=other.publicKey;await assert.rejects(()=>verifyState(c,0),/CHAIN_ACCOUNT_MISMATCH/);
  good.owner=PROGRAM;const wrong=vault();wrong.data[64]=0;
  t.mock.method(connection,'getAccountInfo',async address=>address.equals(identity(c).commitment)?good:wrong);
  await assert.rejects(()=>verifyState(c,0),/CHAIN_VAULT_MISMATCH/);
});
test('an expired unsigned deposit with no account can safely be retried',async t=>{
  chain(t,null);t.mock.method(connection,'getSignaturesForAddress',async()=>[]);t.mock.method(connection,'getBlockHeight',async()=>200);
  assert.equal((await reconcile(c,'DEPOSIT',{last_valid_block_height:100},null)).state,'EXPIRED');
});
test('expiry with a terminal account or an unfinalized failure retains the pending command',async t=>{
  chain(t,account(1));t.mock.method(connection,'getSignaturesForAddress',async()=>[]);t.mock.method(connection,'getBlockHeight',async()=>200);
  assert.equal((await reconcile(c,'SUCCESS',{last_valid_block_height:100},null)).state,'PENDING');
  t.mock.method(connection,'getTransaction',async()=>null);t.mock.method(connection,'getSignatureStatuses',async()=>({value:[{err:{InstructionError:[0,'error']},confirmationStatus:'processed'}]}));
  assert.equal((await reconcile(c,'SUCCESS',{last_valid_block_height:100},'signature')).state,'PENDING');
});
test('an expired settlement can be retried only with an unchanged active account',async t=>{
  chain(t,account());t.mock.method(connection,'getSignaturesForAddress',async()=>[]);t.mock.method(connection,'getBlockHeight',async()=>200);
  assert.equal((await reconcile(c,'SUCCESS',{last_valid_block_height:100},null)).state,'EXPIRED');
});
test('externally submitted transaction is recovered by exact finalized message',async t=>{
  chain(t,account());const tx=transaction();tx.sign(owner);const message=tx.compileMessage();
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'recovered',err:null}]);
  t.mock.method(connection,'getTransaction',async()=>({transaction:{message},meta:{err:null},slot:123}));
  const r=await reconcile(c,'DEPOSIT',{message:message.serialize().toString('base64')},null);assert.equal(r.state,'CONFIRMED');assert.equal(r.signature,'recovered');
});
test('a different network refuses reconciliation before transaction inspection',async t=>{
  t.mock.method(connection,'getGenesisHash',async()=>'mainnet');await assert.rejects(()=>reconcile(c,'DEPOSIT',{},null),/WRONG_NETWORK/);
});

test('owner cancellation and hard timeout require only the owner signature',async t=>{
  await privateKeys(t); // Both exits work even when this installation has no oracle.
  t.mock.method(connection,'getGenesisHash',async()=>NETWORK);
  const mint=Buffer.alloc(82);mint[44]=6;mint[45]=1;
  t.mock.method(connection,'getAccountInfo',async address=>{
    if(address.equals(PROGRAM))return {executable:true};
    if(address.equals(MINT))return {owner:TOKEN_PROGRAM_ID,data:mint};
    return address.equals(identity(c).commitment)?account():vault();
  });
  t.mock.method(connection,'getLatestBlockhash',async()=>({blockhash:other.publicKey.toBase58(),lastValidBlockHeight:100}));
  for(const action of ['CANCEL','TIMEOUT']){
    const payload=await prepare(c,action);
    assert.equal(payload.user_signs,true);
    const tx=Transaction.from(Buffer.from(payload.transaction,'base64'));
    assert.equal(tx.feePayer.toBase58(),owner.publicKey.toBase58());
    assert.deepEqual(tx.signatures.map(s=>s.publicKey.toBase58()),[owner.publicKey.toBase58()]);
    tx.sign(owner);
    assert.ok(validateSigned(payload,tx.serialize().toString('base64')).signature);
  }
});

test('a deposit with missing tokens or rent SOL refuses before wallet signing',async t=>{
  const root=await privateKeys(t);
  await writeFile(join(root,'prototype-oracle.json'),JSON.stringify(Array.from(owner.secretKey)));
  // Pre-signing fund checks need the public oracle identity, never its real secret.
  t.mock.method(Keypair,'fromSecretKey',()=>({publicKey:ORACLE}));
  t.mock.method(connection,'getGenesisHash',async()=>NETWORK);
  const mint=Buffer.alloc(82);mint[44]=6;mint[45]=1;
  let source=null;
  t.mock.method(connection,'getAccountInfo',async address=>address.equals(PROGRAM)?{executable:true}:address.equals(MINT)?{owner:TOKEN_PROGRAM_ID,data:mint}:source);
  t.mock.method(connection,'getLatestBlockhash',async()=>({blockhash:other.publicKey.toBase58(),lastValidBlockHeight:100}));
  await assert.rejects(()=>prepare(c,'DEPOSIT'),/INSUFFICIENT_TEST_TOKENS/);
  source=vault();owner.publicKey.toBuffer().copy(source.data,32);
  t.mock.method(connection,'getBalance',async()=>0);
  t.mock.method(connection,'getMinimumBalanceForRentExemption',async()=>123);
  t.mock.method(connection,'getFeeForMessage',async()=>({value:10_000}));
  await assert.rejects(()=>prepare(c,'DEPOSIT'),/INSUFFICIENT_TEST_SOL/);
});

test('a confirmed deposit recovers a later finalized timeout without losing the deposit identity',async t=>{
  chain(t,account(4));const tx=transaction();tx.sign(owner);const message=tx.compileMessage();
  const deposit={transaction:{message,signatures:['deposit-original']},meta:{err:null},slot:120};
  const timeout=settlement(4,'timeout-external');
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'timeout-external',err:null},{signature:'deposit-original',err:null}]);
  t.mock.method(connection,'getTransaction',async signature=>signature==='deposit-original'?deposit:timeout);
  const result=await reconcile(c,'DEPOSIT',{message:message.serialize().toString('base64')},'deposit-original');
  assert.equal(result.state,'CONFIRMED');assert.equal(result.signature,'deposit-original');assert.equal(result.slot,120);
  assert.equal(result.settlement.action,'TIMEOUT');assert.equal(result.settlement.signature,'timeout-external');assert.equal(result.settlement.slot,124);
  assert.equal(result.settlement.recipient,c.owner);assert.equal(result.settlement.amount_units,5_000_000);
});

test('state refresh proves each external terminal action and its actual recipient',async t=>{
  let status=1;chain(t,account());
  t.mock.method(connection,'getAccountInfo',async address=>address.equals(identity(c).commitment)?account(status):vault());
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'external-settlement',err:null}]);
  t.mock.method(connection,'getTransaction',async()=>settlement(status));
  for(status=1;status<=4;status++){
    const result=await reconcile(c,'STATE',{},null);
    assert.equal(result.state,'RECOVERED');assert.equal(result.settlement.chain_status,status);
    assert.equal(result.settlement.action,['SUCCESS','FAILURE','CANCEL','TIMEOUT'][status-1]);
    assert.equal(result.settlement.recipient,status===2?RECIPIENT.toBase58():c.owner);
  }
});

test('a terminal bit and missing history never invent a refund or forfeit',async t=>{
  chain(t,account(4));t.mock.method(connection,'getSignaturesForAddress',async()=>[]);
  assert.deepEqual(await reconcile(c,'STATE',{},null),{state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'});
  const tx=transaction();tx.sign(owner);const message=tx.compileMessage();
  t.mock.method(connection,'getTransaction',async()=>({transaction:{message,signatures:['deposit']},meta:{err:null},slot:120}));
  assert.equal((await reconcile(c,'DEPOSIT',{message:message.serialize().toString('base64')},'deposit')).state,'PENDING');
});

test('external recovery rejects a mismatched CPI, recipient, amount, program or failed transaction',async t=>{
  chain(t,account(4));t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'external-settlement',err:null}]);
  let candidate; t.mock.method(connection,'getTransaction',async()=>candidate);
  const attacks=[
    result=>{result.meta.err={InstructionError:[0,'failure']};},
    result=>{delete result.meta.err;},
    result=>{result.transaction.signatures[0]='another-signature';},
    result=>{result.transaction.message.instructions[0].data=transaction().compileMessage().instructions[0].data;},
    result=>{result.meta.postTokenBalances[1].owner=other.publicKey.toBase58();},
    result=>{result.meta.postTokenBalances[1].uiTokenAmount.amount='104999999';},
    result=>{result.meta.innerInstructions=[];},
    result=>{result.meta.innerInstructions[0].instructions[0].accounts.reverse();},
    result=>{result.meta.innerInstructions[0].instructions[0].data='1';},
    result=>{result.transaction.message.accountKeys[result.transaction.message.instructions[0].programIdIndex]=SystemProgram.programId;}
  ];
  for(const attack of attacks){candidate=settlement();attack(candidate);assert.equal((await reconcile(c,'STATE',{},null)).state,'PENDING');}
});

test('external recovery still binds immutable terms and the current vault',async t=>{
  const state=account(4);chain(t,state);
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'external-settlement',err:null}]);
  t.mock.method(connection,'getTransaction',async()=>settlement());
  state.data[96]^=1;await assert.rejects(()=>reconcile(c,'STATE',{},null),/CHAIN_TERMS_MISMATCH/);state.data[96]^=1;
  const wrong=vault();other.publicKey.toBuffer().copy(wrong.data,32);
  t.mock.method(connection,'getAccountInfo',async address=>address.equals(identity(c).commitment)?state:wrong);
  await assert.rejects(()=>reconcile(c,'STATE',{},null),/CHAIN_VAULT_MISMATCH/);
});

test('a different verified external settlement supersedes pending bytes without rebroadcast',async t=>{
  chain(t,account(4));const tx=transaction();tx.sign(owner);
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'external-settlement',err:null}]);
  t.mock.method(connection,'getTransaction',async signature=>signature==='pending-original'?null:settlement());
  t.mock.method(connection,'sendRawTransaction',async()=>assert.fail('Recovery must never broadcast replacement bytes'));
  const result=await reconcile(c,'SUCCESS',{message:tx.serializeMessage().toString('base64'),last_valid_block_height:100},'pending-original');
  assert.equal(result.state,'RECOVERED');assert.equal(result.command_status,'FAILED');assert.equal(result.reason,'SUPERSEDED_BY_VERIFIED_SETTLEMENT');
  assert.equal(result.settlement.action,'TIMEOUT');assert.equal(result.settlement.signature,'external-settlement');
});

test('intermittent history of the original settlement confirms it instead of marking it superseded',async t=>{
  chain(t,account(1));const result=settlement(1,'original');const payload={message:result.transaction.message.serialize().toString('base64')};
  let calls=0;t.mock.method(connection,'getTransaction',async()=>++calls===1?null:result);
  t.mock.method(connection,'getSignaturesForAddress',async()=>[{signature:'original',err:null}]);
  assert.deepEqual(await reconcile(c,'SUCCESS',payload,'original'),{state:'CONFIRMED',signature:'original',slot:124});
});

test('a failed deposit with no created account is failed, not an invented terminal transfer',async t=>{
  chain(t,null);const tx=transaction();tx.sign(owner);const message=tx.compileMessage();
  t.mock.method(connection,'getTransaction',async()=>({transaction:{message,signatures:['failed']},meta:{err:{InstructionError:[0,'failure']}},slot:120}));
  assert.deepEqual(await reconcile(c,'DEPOSIT',{message:message.serialize().toString('base64')},'failed'),{state:'FAILED'});
});

test('an active state refresh does not scan transaction history or initiate a transfer',async t=>{
  chain(t,account());t.mock.method(connection,'getSignaturesForAddress',async()=>assert.fail('Active state needs no terminal history'));
  t.mock.method(connection,'sendRawTransaction',async()=>assert.fail('State refresh is read only'));
  assert.deepEqual(await reconcile(c,'STATE',{},null),{state:'ACTIVE',chain_status:0});
});

test('external terminal recovery paginates past later address-only transactions',async t=>{
  chain(t,account(4));
  // Anyone may credit a commitment with a lamport. Such a finalized transfer
  // mentions its address, but cannot change the program's terminal state.
  const signatures=[...Array.from({length:25},(_,i)=>({signature:`later-${i}`,err:null})),{signature:'external-settlement',err:null}];
  const pages=[];
  t.mock.method(connection,'getSignaturesForAddress',async(_address,options)=>{
    pages.push(options.before??null);
    const start=options.before?signatures.findIndex(s=>s.signature===options.before)+1:0;
    return signatures.slice(start,start+options.limit);
  });
  t.mock.method(connection,'getTransaction',async signature=>{
    if(signature==='external-settlement')return settlement(4,signature);
    const noise=new Transaction({feePayer:other.publicKey,recentBlockhash:other.publicKey.toBase58()})
      .add(SystemProgram.transfer({fromPubkey:other.publicKey,toPubkey:identity(c).commitment,lamports:1}));
    return {transaction:{message:noise.compileMessage(),signatures:[signature]},slot:125,meta:{err:null}};
  });
  t.mock.method(connection,'sendRawTransaction',async()=>assert.fail('Terminal recovery must not broadcast'));
  const result=await reconcile(c,'STATE',{},null);
  assert.equal(result.state,'RECOVERED');
  assert.equal(result.settlement.signature,'external-settlement');
  assert.equal(result.settlement.action,'TIMEOUT');
  assert.ok(pages.length>1,'History must request the next signature page');
});

test('an unknown submitted message is recovered from an older finalized page',async t=>{
  chain(t,account());const tx=transaction();tx.sign(owner);const message=tx.compileMessage();
  let originalReads=0;
  const signatures=[...Array.from({length:22},(_,i)=>({signature:`later-${i}`,err:null})),{signature:'deposit-original',err:null}];
  t.mock.method(connection,'getSignaturesForAddress',async(_address,options)=>{
    const start=options.before?signatures.findIndex(s=>s.signature===options.before)+1:0;
    return signatures.slice(start,start+options.limit);
  });
  t.mock.method(connection,'getTransaction',async signature=>{
    if(signature!=='deposit-original')return null;
    originalReads++;assert.equal(originalReads,1,'A matched finalized transaction must not be fetched twice');
    return {transaction:{message,signatures:[signature]},meta:{err:null},slot:120};
  });
  const result=await reconcile(c,'DEPOSIT',{message:message.serialize().toString('base64')},null);
  assert.equal(result.state,'CONFIRMED');assert.equal(result.signature,'deposit-original');assert.equal(result.slot,120);
  assert.equal(originalReads,1);
});

test('history page exhaustion shares one finite budget and keeps terminal outcomes pending',async t=>{
  chain(t,account(4));let pages=0,reads=0;
  t.mock.method(connection,'getSignaturesForAddress',async(_address,options)=>{
    pages++;assert.equal(options.limit,20);
    return Array.from({length:20},(_,i)=>({signature:`page-${pages}-${i}`,err:null}));
  });
  t.mock.method(connection,'getTransaction',async()=>{reads++;return null;});
  t.mock.method(connection,'sendRawTransaction',async()=>assert.fail('An exhausted search cannot broadcast'));
  assert.equal((await reconcile(c,'SUCCESS',{message:'missing',last_valid_block_height:100},null)).state,'PENDING');
  assert.equal(pages,5);assert.equal(reads,100);
});

test('an already observed external proof survives the shared history page limit',async t=>{
  chain(t,account(4));let pages=0,reads=0;
  t.mock.method(connection,'getSignaturesForAddress',async()=>{
    pages++;return Array.from({length:20},(_,i)=>({signature:`page-${pages}-${i}`,err:null}));
  });
  t.mock.method(connection,'getTransaction',async signature=>{
    reads++;return signature==='page-5-19'?settlement(4,signature):null;
  });
  const result=await reconcile(c,'SUCCESS',{message:'original-unknown',last_valid_block_height:100},null);
  assert.equal(result.state,'RECOVERED');assert.equal(result.settlement.action,'TIMEOUT');
  assert.equal(result.settlement.signature,'page-5-19');assert.equal(result.command_status,'FAILED');
  assert.equal(pages,5);assert.equal(reads,100);
});

test('history time exhaustion aborts the actual RPC request and retains pending state',async t=>{
  chain(t,account(4));t.mock.timers.enable({apis:['setTimeout']});
  let signal,started;const requested=new Promise(resolve=>{started=resolve;});
  t.mock.method(globalThis,'fetch',async(_url,options)=>{
    signal=options.signal;started();
    return new Promise((_resolve,reject)=>signal.addEventListener('abort',()=>reject(signal.reason),{once:true}));
  });
  const pending=reconcile(c,'STATE',{},null);
  await requested;t.mock.timers.tick(8000);
  assert.deepEqual(await pending,{state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'});
  assert.equal(signal.aborted,true);
});
