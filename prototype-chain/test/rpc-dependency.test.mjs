import test from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const BrowserClient=require('jayson/lib/client/browser');

test('pinned Jayson browser client preserves web3 JSON-RPC request, result and error callbacks',async()=>{
  const messages=[];
  const client=new BrowserClient((text,callback)=>{
    const request=JSON.parse(text);messages.push(request);
    callback(null,JSON.stringify({jsonrpc:'2.0',id:request.id,result:{value:123}}));
  },{generator:()=> 'stable-id'});
  const result=await new Promise((resolve,reject)=>client.request('getBalance',['public-key'],(error,response)=>error?reject(error):resolve(response)));
  assert.deepEqual(messages,[{method:'getBalance',jsonrpc:'2.0',params:['public-key'],id:'stable-id'}]);
  assert.deepEqual(result,{jsonrpc:'2.0',id:'stable-id',result:{value:123}});
  const errorClient=new BrowserClient((text,callback)=>callback(null,JSON.stringify({jsonrpc:'2.0',id:JSON.parse(text).id,error:{code:-32000,message:'Test error'}})));
  const rpcError=await new Promise((resolve,reject)=>errorClient.request('getBalance',[],(transport,error,result)=>transport?reject(transport):resolve({error,result})));
  assert.equal(rpcError.error.code,-32000);assert.equal(rpcError.result,undefined);
  assert.throws(()=>require.resolve('stream-json'),{code:'MODULE_NOT_FOUND'});
});
