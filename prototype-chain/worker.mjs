// Private stdin protocol. No HTTP listener, no client-supplied key paths or RPC endpoints.
import {balance,prepare,validateSigned,broadcast,reconcile} from './client.mjs';
let text='';
try {
  for await(const chunk of process.stdin){text+=chunk;if(text.length>20000)throw Error('INPUT_LIMIT');}
  const i=JSON.parse(text);let result;
  switch(i.operation){
    case 'balance':result=await balance(i.owner);break;
    case 'prepare':result=await prepare(i.challenge,i.action);break;
    case 'validate':result=validateSigned(i.payload,i.transaction);break;
    case 'broadcast':result=await broadcast(i.payload,i.transaction);break;
    case 'reconcile':result=await reconcile(i.challenge,i.action,i.payload,i.signature);break;
    default:throw Error('INVALID_OPERATION');
  }
  process.stdout.write(JSON.stringify({ok:true,result}));
}catch(e){
  // RPC diagnostics can contain user data. Expose only a fixed token, never raw RPC text.
  const code=/^[A-Z_]{3,60}$/.test(e.message)?e.message:'CHAIN_UNAVAILABLE';
  process.stdout.write(JSON.stringify({ok:false,code}));process.exitCode=1;
}
