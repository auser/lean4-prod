import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const maximum = 1_048_576;
const maximumPages = Number(process.argv[3] ?? 80);
assert.ok(Number.isSafeInteger(maximumPages) && maximumPages > 0);
for (const size of [0, 1, 8192, 65536, maximum - 1, maximum]) {
  const input = Uint8Array.from({length: size}, (_, index) => index % 251);
  for (let repeat = 0; repeat < 2; repeat++) {
    const {exports} = new WebAssembly.Instance(module, {});
    const pointer = exports.holo_alloc(size);
    new Uint8Array(exports.memory.buffer, pointer, size).set(input);
    const packed = BigInt.asUintN(64, exports.holo_run(pointer, size));
    const start = Number(packed >> 32n);
    const length = Number(packed & 0xffffffffn);
    assert.equal(length, size);
    assert.ok(start + length <= exports.memory.buffer.byteLength);
    assert.ok(exports.memory.buffer.byteLength <= maximumPages * 65536);
    assert.deepEqual(new Uint8Array(exports.memory.buffer, start, length), input);
  }
}
const {exports} = new WebAssembly.Instance(module, {});
assert.throws(() => exports.holo_alloc(maximum + 1), WebAssembly.RuntimeError);
console.log(`PASS actual import-free Wasm borrowed operands within ${maximumPages}-page fixture bound`);
