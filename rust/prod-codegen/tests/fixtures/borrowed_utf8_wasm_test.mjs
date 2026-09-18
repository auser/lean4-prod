import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const encoder = new TextEncoder();
function invoke(input) {
  const {exports} = new WebAssembly.Instance(module, {});
  const pointer = exports.holo_alloc(input.length);
  new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
  const packed = BigInt.asUintN(64, exports.holo_run(pointer, input.length));
  const start = Number(packed >> 32n), length = Number(packed & 0xffffffffn);
  assert.ok(length <= 128 && start + length <= exports.memory.buffer.byteLength);
  assert.ok(exports.memory.buffer.byteLength <= 4 * 65536);
  return new Uint8Array(exports.memory.buffer, start, length).slice();
}
for (const text of ['', 'ascii', 'é\0𐐷', '\ufeffedge', 'a\0b', 'x'.repeat(128)]) {
  const input = encoder.encode(text);
  for (let repeat = 0; repeat < 2; repeat++) assert.deepEqual(invoke(input), input);
}
assert.deepEqual(invoke(Uint8Array.of(0xff)), Uint8Array.of(0xff));
assert.throws(() => invoke(new Uint8Array(129)), WebAssembly.RuntimeError);
console.log('PASS borrowed/aliased/repeated/record/owned UTF-8 in actual import-free Wasm');
