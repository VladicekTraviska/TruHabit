import {createHash} from 'node:crypto';
import {AsyncLocalStorage} from 'node:async_hooks';
import {readFile} from 'node:fs/promises';
import {isAbsolute,join} from 'node:path';
import {Connection,PublicKey,Keypair,Transaction,TransactionInstruction,SystemProgram} from '@solana/web3.js';
import {TOKEN_PROGRAM_ID,getAssociatedTokenAddressSync,createAssociatedTokenAccountIdempotentInstruction,getAccount,getMint} from './spl.mjs';

export const NETWORK='EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG';
export const PROGRAM=new PublicKey('24nGy1KAx5GSFR2C6xaHnRgKMqYyoNq3LNfu3fFtvNRa');
export const MINT=new PublicKey('BYa72dJf9S1sw4a4cmESd9DhinH8pQ6fy7gjbPm6VNp3');
export const ORACLE=new PublicKey('8m8eyZih5Mjakb8CH5Bxsi7BLp2gbderP6bKuJPm96n4');
export const RECIPIENT=new PublicKey('6dgtm5XMrG6VTSWZGtrpZMWgZHq6mr4JuzBbjBBJTD8e');
const historySignal=new AsyncLocalStorage();
export const connection=new Connection('https://api.devnet.solana.com',{commitment:'finalized',disableRetryOnRateLimit:true,fetch:async(url,options)=>{
  const signals=[AbortSignal.timeout(15000),options?.signal,historySignal.getStore()].filter(Boolean);
  return fetch(url,{...options,signal:AbortSignal.any(signals)});
}});
export const digest=s=>createHash('sha256').update(s).digest();
const disc=name=>digest('global:'+name).subarray(0,8);
const key=(pubkey,isSigner=false,isWritable=false)=>({pubkey,isSigner,isWritable});
const i64=n=>{const b=Buffer.alloc(8);b.writeBigInt64LE(BigInt(n));return b;};
export async function network(){if(await connection.getGenesisHash()!==NETWORK)throw Error('WRONG_NETWORK');}
export async function localKey(name){
  if(!['oracle','program','mint','recipient','tester'].includes(name))throw Error('INVALID_KEY_ROLE');
  const configured=process.env.TRUHABIT_KEY_DIR;
  if(configured&&!isAbsolute(configured))throw Error('INVALID_KEY_DIRECTORY');
  const path=configured?join(configured,`prototype-${name}.json`):new URL(`../.local/prototype-${name}.json`,import.meta.url);
  let source;
  try{source=await readFile(path,'utf8');}catch(error){
    if(name==='oracle')throw Error(error.code==='ENOENT'?'CHAIN_ORACLE_NOT_CONFIGURED':'CHAIN_ORACLE_UNREADABLE');
    throw error;
  }
  try{
    if(source.length>4096)throw Error();
    const bytes=JSON.parse(source);
    if(!Array.isArray(bytes)||bytes.length!==64||bytes.some(byte=>!Number.isInteger(byte)||byte<0||byte>255))throw Error();
    return Keypair.fromSecretKey(Uint8Array.from(bytes));
  }catch{throw Error(name==='oracle'?'CHAIN_ORACLE_INVALID_KEY':'INVALID_KEY_FILE');}
}
async function configuredOracle(){
  const oracle=await localKey('oracle');
  if(!oracle.publicKey.equals(ORACLE))throw Error('WRONG_ORACLE_KEY');
  return oracle;
}
export async function oracleReadiness(){
  try{await configuredOracle();return {oracle_configured:true,oracle_reason:null};}
  catch(error){
    const codes=['CHAIN_ORACLE_NOT_CONFIGURED','CHAIN_ORACLE_UNREADABLE','CHAIN_ORACLE_INVALID_KEY','WRONG_ORACLE_KEY','INVALID_KEY_DIRECTORY'];
    return {oracle_configured:false,oracle_reason:codes.includes(error.message)?error.message:'CHAIN_ORACLE_INVALID_KEY'};
  }
}
export function identity(c){
  if(!/^[0-9a-f-]{36}$/i.test(c.id))throw Error('INVALID_CHALLENGE');
  const owner=new PublicKey(c.owner),id=Buffer.from(c.id.replaceAll('-',''),'hex');
  if(id.length!==16)throw Error('INVALID_CHALLENGE');
  const [commitment]=PublicKey.findProgramAddressSync([Buffer.from('prototype'),owner.toBuffer(),id],PROGRAM);
  const [vault]=PublicKey.findProgramAddressSync([Buffer.from('vault'),commitment.toBuffer()],PROGRAM);
  return {owner,id,commitment,vault};
}
function terms(c){
  if(!Number.isSafeInteger(c.amount_units)||c.amount_units<1e6||c.amount_units>50e6)throw Error('INVALID_AMOUNT');
  const dates=['starts_at','ends_at','upload_deadline','refund_after'].map(k=>Math.floor(Date.parse(c[k])/1000));
  if(dates.some(x=>!Number.isSafeInteger(x)))throw Error('INVALID_TIME');
  if(!/^[0-9a-f]{64}$/.test(c.terms_hash))throw Error('INVALID_TERMS_HASH');
  return {dates,hash:Buffer.from(c.terms_hash,'hex')};
}
export async function balance(owner){
  const oracle=await oracleReadiness();
  await network();
  const key=new PublicKey(owner);
  const [sol,program,mintAccount]=await Promise.all([connection.getBalance(key),connection.getAccountInfo(PROGRAM),connection.getAccountInfo(MINT)]);
  let amount='0';
  if(mintAccount){const mint=await getMint(connection,MINT);if(mint.decimals!==6)throw Error('INVALID_MINT');
    const ata=getAssociatedTokenAddressSync(MINT,key);if(await connection.getAccountInfo(ata)) amount=(await getAccount(connection,ata)).amount.toString();}
  return {network:'solana-devnet',genesis:NETWORK,program:PROGRAM.toBase58(),mint:MINT.toBase58(),recipient:RECIPIENT.toBase58(),token_label:'TruHabit Test Token',decimals:6,amount_units:amount,sol_lamports:sol,deployed:!!program?.executable&&!!mintAccount,...oracle,checked_at:new Date().toISOString()};
}
export async function prepare(c,action){
  // Refuse unavailable oracle signing before any RPC or wallet request. Owner
  // cancellation and emergency timeout remain independent of this private key.
  const oracle=['DEPOSIT','SUCCESS','FAILURE'].includes(action)?await configuredOracle():null;
  await network();
  const program=await connection.getAccountInfo(PROGRAM);
  if(!program?.executable)throw Error('PROTOTYPE_NOT_DEPLOYED');
  const mint=await getMint(connection,MINT);if(mint.decimals!==6)throw Error('INVALID_MINT');
  const {owner,id,commitment,vault}=identity(c),{dates,hash}=terms(c);
  const userSigns=['DEPOSIT','CANCEL','TIMEOUT'].includes(action);
  const block=await connection.getLatestBlockhash('finalized');
  const tx=new Transaction({...block,feePayer:userSigns?owner:ORACLE});
  if(action==='DEPOSIT'){
    const source=getAssociatedTokenAddressSync(MINT,owner);
    if(!await connection.getAccountInfo(source,'finalized'))throw Error('INSUFFICIENT_TEST_TOKENS');
    const account=await getAccount(connection,source);
    if(account.amount<BigInt(c.amount_units)||!account.owner.equals(owner))throw Error('INSUFFICIENT_TEST_TOKENS');
    tx.add(new TransactionInstruction({programId:PROGRAM,keys:[key(owner,true,true),key(ORACLE,true),key(MINT),key(commitment,false,true),key(vault,false,true),key(source,false,true),key(TOKEN_PROGRAM_ID),key(SystemProgram.programId)],data:Buffer.concat([disc('deposit'),id,i64(c.amount_units),...dates.map(i64),hash])}));
    const [sol,rentCommitment,rentVault,fee]=await Promise.all([
      connection.getBalance(owner,'finalized'),
      connection.getMinimumBalanceForRentExemption(130),
      connection.getMinimumBalanceForRentExemption(165),
      connection.getFeeForMessage(tx.compileMessage(),'finalized')
    ]);
    if(fee.value===null)throw Error('CHAIN_UNAVAILABLE');
    if(sol<rentCommitment+rentVault+fee.value)throw Error('INSUFFICIENT_TEST_SOL');
  }else{
    const actions={SUCCESS:0,FAILURE:1,CANCEL:2,TIMEOUT:3};
    if(!Object.hasOwn(actions,action))throw Error('INVALID_ACTION');
    await verifyState(c,0);
    const recipient=action==='FAILURE'?RECIPIENT:owner;
    const destination=getAssociatedTokenAddressSync(MINT,recipient);
    tx.add(createAssociatedTokenAccountIdempotentInstruction(userSigns?owner:ORACLE,destination,recipient,MINT));
    tx.add(new TransactionInstruction({programId:PROGRAM,keys:[key(userSigns?owner:ORACLE,true),key(MINT),key(commitment,false,true),key(vault,false,true),key(destination,false,true),key(TOKEN_PROGRAM_ID)],data:Buffer.concat([disc('settle'),Buffer.from([actions[action]])])}));
  }
  // Owner exits need neither the oracle's secret nor its signature.
  if(oracle){
    if(action==='DEPOSIT')tx.partialSign(oracle);else tx.sign(oracle);
  }
  return {transaction:tx.serialize({requireAllSignatures:!userSigns}).toString('base64'),message:tx.serializeMessage().toString('base64'),last_valid_block_height:block.lastValidBlockHeight,user_signs:userSigns,chain:{network:'solana-devnet',genesis:NETWORK,program:PROGRAM.toBase58(),mint:MINT.toBase58(),recipient:RECIPIENT.toBase58(),owner:owner.toBase58(),commitment:commitment.toBase58(),vault:vault.toBase58(),terms_hash:c.terms_hash}};
}
export function validateSigned(payload,encoded){
  if(typeof encoded!=='string'||encoded.length>2000)throw Error('INVALID_TRANSACTION');
  const tx=Transaction.from(Buffer.from(encoded,'base64'));
  if(tx.serializeMessage().toString('base64')!==payload.message||!tx.verifySignatures(true))throw Error('TRANSACTION_MISMATCH');
  // The primary signature can be obtained from the compiled transaction without another dependency.
  const signature=tx.signatures[0].signature;
  if(!signature)throw Error('MISSING_SIGNATURE');
  return {transaction:tx.serialize().toString('base64'),signature:base58(signature)};
}
function base58(bytes){
  const alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';let n=BigInt('0x'+Buffer.from(bytes).toString('hex')),s='';while(n){s=alphabet[Number(n%58n)]+s;n/=58n;}for(const b of bytes){if(b!==0)break;s='1'+s;}return s;
}
export async function broadcast(payload,encoded){
  await network();const verified=validateSigned(payload,encoded);
  // Sending identical signed bytes is safe; never fabricate a fresh signature on timeout.
  await connection.sendRawTransaction(Buffer.from(verified.transaction,'base64'),{skipPreflight:false,maxRetries:2});
  return verified;
}
export async function verifyState(c,expected){
  const {owner,id,commitment,vault}=identity(c),{dates,hash}=terms(c);
  const info=await connection.getAccountInfo(commitment,'finalized');
  if(!info||!info.owner.equals(PROGRAM)||info.data.length!==130)throw Error('CHAIN_ACCOUNT_MISMATCH');
  const b=info.data;
  const status=b[129];
  if(!b.subarray(0,8).equals(digest('account:Commitment').subarray(0,8))||!b.subarray(8,24).equals(id)||!b.subarray(24,56).equals(owner.toBuffer())||b.readBigUInt64LE(56)!==BigInt(c.amount_units)||!dates.every((v,i)=>b.readBigInt64LE(64+i*8)===BigInt(v))||!b.subarray(96,128).equals(hash)||status>4||(expected!==undefined&&status!==expected))throw Error('CHAIN_TERMS_MISMATCH');
  const tokens=await getAccount(connection,vault,'finalized');
  if(!tokens.owner.equals(commitment)||!tokens.mint.equals(MINT)||tokens.amount<(status===0?BigInt(c.amount_units):0n))throw Error('CHAIN_VAULT_MISMATCH');
  return {status,commitment:commitment.toBase58()};
}
const settlementActions=['SUCCESS','FAILURE','CANCEL','TIMEOUT'];
function decode58(encoded){
  if(typeof encoded!=='string'||encoded.length>200)throw Error('INVALID_INSTRUCTION');
  const alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';let n=0n;
  for(const character of encoded){const digit=alphabet.indexOf(character);if(digit<0)throw Error('INVALID_INSTRUCTION');n=n*58n+BigInt(digit);}
  let bytes=n?Buffer.from(n.toString(16).padStart(Math.ceil(n.toString(16).length/2)*2,'0'),'hex'):Buffer.alloc(0);
  let zeroes=0;while(encoded[zeroes]==='1')zeroes++;
  return Buffer.concat([Buffer.alloc(zeroes),bytes]);
}
// A terminal status alone cannot establish who received the tokens or which TX
// moved them. Recovery requires both the actual settle invocation and its CPI.
function settlementProof(c,status,signature,result,payload){
  if(!result?.meta||result.meta.err!==null||result.transaction?.signatures?.[0]!==signature||!Number.isSafeInteger(result.slot))return null;
  const {owner,commitment,vault}=identity(c),action=status-1;
  if(action<0||action>3)return null;
  const recipient=action===1?RECIPIENT:owner;
  const message=result.transaction.message;
  try{
    const keys=message.getAccountKeys({accountKeysFromLookups:result.meta.loadedAddresses});
    const matches=(index,address)=>keys.get(index)?.equals(address)===true;
    const amount=BigInt(c.amount_units);
    for(const [index,ix] of message.compiledInstructions.entries()){
      const data=Buffer.from(ix.data),accounts=ix.accountKeyIndexes;
      if(!matches(ix.programIdIndex,PROGRAM)||data.length!==9||!data.subarray(0,8).equals(disc('settle'))||data[8]!==action||accounts.length!==6)continue;
      const [actor,mint,account,vaultIndex,destination,tokenProgram]=accounts;
      if(!message.isAccountSigner(actor)||!matches(mint,MINT)||!matches(account,commitment)||!matches(vaultIndex,vault)||!matches(tokenProgram,TOKEN_PROGRAM_ID))continue;
      if((action<2&&!matches(actor,ORACLE))||(action===2&&!matches(actor,owner)))continue;
      const tokenBalance=(kind,accountIndex)=>result.meta[kind]?.find(b=>b.accountIndex===accountIndex&&b.mint===MINT.toBase58());
      const beforeVault=tokenBalance('preTokenBalances',vaultIndex),afterVault=tokenBalance('postTokenBalances',vaultIndex);
      const beforeDestination=tokenBalance('preTokenBalances',destination),afterDestination=tokenBalance('postTokenBalances',destination);
      if(beforeVault?.owner!==commitment.toBase58()||afterVault?.owner!==commitment.toBase58()||afterDestination?.owner!==recipient.toBase58()||(beforeDestination&&beforeDestination.owner!==recipient.toBase58()))continue;
      if(BigInt(beforeVault.uiTokenAmount.amount)-BigInt(afterVault.uiTokenAmount.amount)!==amount||BigInt(afterDestination.uiTokenAmount.amount)-BigInt(beforeDestination?.uiTokenAmount.amount??'0')!==amount)continue;
      const inner=result.meta.innerInstructions?.find(group=>group.index===index)?.instructions??[];
      const transferred=inner.some(instruction=>{
        if(!matches(instruction.programIdIndex,TOKEN_PROGRAM_ID)||instruction.accounts?.length!==4)return false;
        const innerData=typeof instruction.data==='string'?decode58(instruction.data):Buffer.from(instruction.data);
        return innerData.length===10&&innerData[0]===12&&innerData.readBigUInt64LE(1)===amount&&innerData[9]===6&&instruction.accounts.every((value,i)=>matches(value,[vault,MINT,keys.get(destination),commitment][i]));
      });
      if(transferred)return {action:settlementActions[action],signature,slot:result.slot,recipient:recipient.toBase58(),destination:keys.get(destination).toBase58(),amount_units:c.amount_units,chain_status:status,matches_command:!!payload&&message.serialize().toString('base64')===payload.message};
    }
  }catch{return null;}
  return null;
}
const HISTORY_PAGE_SIZE=20,HISTORY_MAX_PAGES=5,HISTORY_TIMEOUT_MS=8000;
const HISTORY_EXHAUSTED=Symbol('history-exhausted');
function historyBudget(){
  const controller=new AbortController();let timer=null,pages=HISTORY_MAX_PAGES;
  return {
    observed:[],
    page(){if(pages===0||controller.signal.aborted)return false;pages--;return true;},
    async read(operation){
      if(controller.signal.aborted)return HISTORY_EXHAUSTED;
      if(timer===null)timer=setTimeout(()=>controller.abort(),HISTORY_TIMEOUT_MS);
      let onAbort;
      const aborted=new Promise(resolve=>{onAbort=()=>resolve(HISTORY_EXHAUSTED);controller.signal.addEventListener('abort',onAbort,{once:true});});
      try{return await Promise.race([historySignal.run(controller.signal,operation),aborted]);}
      catch(error){if(controller.signal.aborted)return HISTORY_EXHAUSTED;throw error;}
      finally{controller.signal.removeEventListener('abort',onAbort);}
    },
    close(){if(timer!==null)clearTimeout(timer);controller.abort();}
  };
}
async function finalizedHistory(c,budget,visit,known=[]){
  const seen=new Set(known);let before;
  // Later transfers can mention a settled commitment without changing it.
  // Search older pages, with one shared page/time budget per reconciliation.
  // Exhaustion retains PENDING; it never invents a transfer or permits a retry.
  while(budget.page()){
    const candidates=await budget.read(()=>connection.getSignaturesForAddress(identity(c).commitment,{limit:HISTORY_PAGE_SIZE,...(before?{before}:{})},'finalized'));
    if(candidates===HISTORY_EXHAUSTED||candidates.length===0)return null;
    for(const candidate of candidates.slice(0,HISTORY_PAGE_SIZE)){
      if(candidate.err||seen.has(candidate.signature))continue;
      seen.add(candidate.signature);
      const result=await budget.read(()=>connection.getTransaction(candidate.signature,{commitment:'finalized',maxSupportedTransactionVersion:0}));
      if(result===HISTORY_EXHAUSTED)return null;
      budget.observed.push({signature:candidate.signature,result});
      const match=visit(candidate.signature,result);
      if(match)return match;
    }
    const next=candidates[Math.min(candidates.length,HISTORY_PAGE_SIZE)-1]?.signature;
    if(!next||next===before||candidates.length<HISTORY_PAGE_SIZE)return null;
    before=next;
  }
  return null;
}
async function recoverSettlement(c,status,known=[],payload,budget){
  const observed=[...known,...budget.observed];
  for(const entry of observed){const proof=settlementProof(c,status,entry.signature,entry.result,payload);if(proof)return proof;}
  return finalizedHistory(c,budget,(signature,result)=>settlementProof(c,status,signature,result,payload),observed.map(entry=>entry.signature));
}
export async function reconcile(c,action,payload,signature){
  const budget=historyBudget();
  try{return await reconcileWithHistory(c,action,payload,signature,budget);}
  finally{budget.close();}
}
async function reconcileWithHistory(c,action,payload,signature,budget){
  await network();
  if(action==='STATE'){
    const current=await verifyState(c);
    if(current.status===0)return {state:'ACTIVE',chain_status:0};
    const settlement=await recoverSettlement(c,current.status,[],undefined,budget);
    return settlement?{state:'RECOVERED',settlement}:{state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'};
  }
  const expected={DEPOSIT:0,SUCCESS:1,FAILURE:2,CANCEL:3,TIMEOUT:4}[action];
  if(expected===undefined)throw Error('INVALID_ACTION');
  // Expiry proves only that these bytes cannot land now. A terminal account
  // without available transaction history remains ambiguous and blocks retries.
  async function expiredSafely(){
    if(await connection.getBlockHeight('finalized')<=payload.last_valid_block_height)return false;
    if(action==='DEPOSIT')return !await connection.getAccountInfo(identity(c).commitment,'finalized');
    try{await verifyState(c,0);return true;}catch{return false;}
  }
  if(!signature){
    signature=await finalizedHistory(c,budget,(candidate,found)=>found?.transaction.message.serialize().toString('base64')===payload.message?candidate:null);
  }
  // The history lookup already fetched a matching finalized transaction.
  // Reuse it so an unnecessary second RPC cannot obscure an available result.
  const observed=signature?budget.observed.find(entry=>entry.signature===signature&&entry.result)?.result:null;
  const result=observed??(signature?await connection.getTransaction(signature,{commitment:'finalized',maxSupportedTransactionVersion:0}):null);
  if(!result){
    const info=await connection.getAccountInfo(identity(c).commitment,'finalized');
    if(info){
      const current=await verifyState(c);
      if(current.status>0){
        const settlement=await recoverSettlement(c,current.status,[],payload,budget);
        if(settlement?.matches_command&&settlement.action===action)return {state:'CONFIRMED',signature:settlement.signature,slot:settlement.slot};
        if(settlement&&action!=='DEPOSIT')return {state:'RECOVERED',settlement,command_status:'FAILED',reason:'SUPERSEDED_BY_VERIFIED_SETTLEMENT'};
        return {state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'};
      }
    }else if(action!=='DEPOSIT')throw Error('CHAIN_ACCOUNT_MISMATCH');
    const status=signature?(await connection.getSignatureStatuses([signature],{searchTransactionHistory:true})).value[0]:null;
    if(status?.err&&status.confirmationStatus==='finalized')return {state:'FAILED'};
    // Even after expiry a nonfinal status remains ambiguous; retain the original signed command.
    return {state:!status&&await expiredSafely()?'EXPIRED':'PENDING'};}
  if(result.transaction.message.serialize().toString('base64')!==payload.message)throw Error('CHAIN_MESSAGE_MISMATCH');
  if(!result.meta||!Object.hasOwn(result.meta,'err'))return {state:'PENDING',reason:'TRANSACTION_METADATA_UNAVAILABLE'};
  if(result.meta?.err){
    const info=await connection.getAccountInfo(identity(c).commitment,'finalized');
    const current=info?await verifyState(c):null;
    if(current?.status>0){
      const settlement=await recoverSettlement(c,current.status,[{signature,result}],payload,budget);
      if(settlement)return {state:'RECOVERED',settlement,command_status:'FAILED',reason:'SUPERSEDED_BY_VERIFIED_SETTLEMENT'};
      return {state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'};
    }
    return {state:'FAILED'};
  }
  const current=await verifyState(c);
  if(action==='DEPOSIT'&&current.status>0){
    const settlement=await recoverSettlement(c,current.status,[{signature,result}],undefined,budget);
    if(!settlement)return {state:'PENDING',reason:'TERMINAL_HISTORY_UNAVAILABLE'};
    return {state:'CONFIRMED',signature,slot:result.slot,settlement};
  }
  if(current.status!==expected)throw Error('CHAIN_TERMS_MISMATCH');
  return {state:'CONFIRMED',signature,slot:result.slot};
}
