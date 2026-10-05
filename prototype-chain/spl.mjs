// Fixed legacy SPL Token layouts only. BigInt decoding avoids native bigint-buffer bindings.
import {PublicKey,SystemProgram,TransactionInstruction} from '@solana/web3.js';
export const TOKEN_PROGRAM_ID=new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
export const ASSOCIATED_TOKEN_PROGRAM_ID=new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
export function getAssociatedTokenAddressSync(mint,owner){return PublicKey.findProgramAddressSync([owner.toBuffer(),TOKEN_PROGRAM_ID.toBuffer(),mint.toBuffer()],ASSOCIATED_TOKEN_PROGRAM_ID)[0];}
export function createAssociatedTokenAccountIdempotentInstruction(payer,address,owner,mint){
  return new TransactionInstruction({programId:ASSOCIATED_TOKEN_PROGRAM_ID,keys:[{pubkey:payer,isSigner:true,isWritable:true},{pubkey:address,isSigner:false,isWritable:true},{pubkey:owner,isSigner:false,isWritable:false},{pubkey:mint,isSigner:false,isWritable:false},{pubkey:SystemProgram.programId,isSigner:false,isWritable:false},{pubkey:TOKEN_PROGRAM_ID,isSigner:false,isWritable:false}],data:Buffer.from([1])});
}
export async function getAccount(connection,address,commitment='finalized'){
  const a=await connection.getAccountInfo(address,commitment);
  if(!a||!a.owner.equals(TOKEN_PROGRAM_ID)||a.data.length!==165||a.data[108]!==1)throw Error('INVALID_TOKEN_ACCOUNT');
  return {mint:new PublicKey(a.data.subarray(0,32)),owner:new PublicKey(a.data.subarray(32,64)),amount:a.data.readBigUInt64LE(64)};
}
export async function getMint(connection,address){
  const a=await connection.getAccountInfo(address,'finalized');
  if(!a||!a.owner.equals(TOKEN_PROGRAM_ID)||a.data.length!==82||a.data[45]!==1)throw Error('INVALID_MINT');
  return {decimals:a.data[44],supply:a.data.readBigUInt64LE(36)};
}
