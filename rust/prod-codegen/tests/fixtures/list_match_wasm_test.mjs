import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
assert.equal(process.argv.length,3);
const module=new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module),[]);
let count=0;
for(const length of [0,1,31,256,4096,65536])for(let repeat=0;repeat<2;repeat++){
  const input=Uint8Array.from({length},(_,i)=>i%251);
  const instance=new WebAssembly.Instance(module,{});
  const at=instance.exports.holo_alloc(input.length);
  new Uint8Array(instance.exports.memory.buffer,at,input.length).set(input);
  const result=BigInt.asUintN(64,instance.exports.holo_run(at,input.length));
  const pointer=Number(result>>32n),size=Number(result&0xffffffffn);
  assert.equal(size,input.length);
  assert.ok(instance.exports.memory.buffer.byteLength<=256*65536);
  assert.deepEqual(new Uint8Array(instance.exports.memory.buffer,pointer,size),input);
  count++;
}
assert.equal(count,12);
for(const byte of [255,254]) {
  const instance=new WebAssembly.Instance(module,{});
  const at=instance.exports.holo_alloc(1);
  new Uint8Array(instance.exports.memory.buffer,at,1).set([byte]);
  assert.throws(()=>instance.exports.holo_run(at,1),WebAssembly.RuntimeError);
  // Failure cannot manufacture a success result or poison the later valid call.
  const valid=instance.exports.holo_alloc(1);
  new Uint8Array(instance.exports.memory.buffer,valid,1).set([42]);
  const result=BigInt.asUintN(64,instance.exports.holo_run(valid,1));
  assert.deepEqual(new Uint8Array(instance.exports.memory.buffer,Number(result>>32n),Number(result&0xffffffffn)),Uint8Array.of(42));
}
console.log('list scrutinees: 12 values and 2 eager-error Wasm executions passed');
