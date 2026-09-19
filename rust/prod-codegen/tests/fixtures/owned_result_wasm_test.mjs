import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const pages = Number(process.argv[3]);
assert.ok(pages === 65 || pages === 66);
if (pages === 66) await import('./borrowed_operands_wasm_test.mjs');
const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const {exports} = new WebAssembly.Instance(module, {});
const maximum = 1_048_576;
const pointer = exports.holo_alloc(maximum);
new Uint8Array(exports.memory.buffer, pointer, maximum).fill(19);
if (pages === 65) {
  assert.throws(() => exports.holo_run(pointer, maximum), WebAssembly.RuntimeError);
  assert.ok(exports.memory.buffer.byteLength <= pages * 65536);
} else {
  const packed = BigInt.asUintN(64, exports.holo_run(pointer, maximum));
  const start = Number(packed >> 32n);
  assert.equal(Number(packed & 0xffffffffn), maximum);
  assert.equal(exports.memory.buffer.byteLength, pages * 65536);
  new Uint8Array(exports.memory.buffer, start, maximum).fill(23);
  assert.ok(new Uint8Array(exports.memory.buffer, pointer, maximum).every(byte => byte === 19));
}
console.log(`PASS exact owned-result ${pages}-page ${pages === 66 ? 'acceptance' : 'denial'}`);
