import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
assert.equal(Number(process.argv[3]), 98);
for (const size of [0, 1, 8192, 65536, 1_048_575, 1_048_576]) {
  const input = Uint8Array.from({length: size}, (_, index) => index % 251);
  const {exports: e} = new WebAssembly.Instance(module, {});
  const pointer = e.holo_alloc(size);
  new Uint8Array(e.memory.buffer, pointer, size).set(input);
  const result = BigInt.asUintN(64, e.holo_run(pointer, size));
  const offset = Number(result >> 32n), length = Number(result & 0xffffffffn);
  assert.equal(length, size * 2);
  assert.ok(offset + length <= e.memory.buffer.byteLength);
  assert.ok(e.memory.buffer.byteLength <= 98 * 65536);
  assert.deepEqual(new Uint8Array(e.memory.buffer, offset, size), input);
  assert.deepEqual(new Uint8Array(e.memory.buffer, offset + size, size), input);
}
assert.throws(() => new WebAssembly.Instance(module, {}).exports.holo_alloc(1_048_577), WebAssembly.RuntimeError);
console.log('PASS actual self append, doubled bytes and unchanged 98-page bound');
