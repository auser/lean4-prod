import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const maximum = 1048576, maximumPages = 256;
let cases = 0, memoryPeak = 0;
for (const size of [0, 1, 128, 255, 256, 4096, maximum - 1, maximum]) {
  const input = Buffer.alloc(size);
  for (let index = 0; index < size; index++) input[index] = index % 251;
  for (let repeat = 0; repeat < 2; repeat++) {
    const {exports} = new WebAssembly.Instance(module, {});
    const at = exports.holo_alloc(size) >>> 0;
    new Uint8Array(exports.memory.buffer, at, size).set(input);
    const result = BigInt.asUintN(64, exports.holo_run(at, size));
    const offset = Number(result >> 32n), length = Number(result & 0xffffffffn);
    assert.equal(length, size);
    assert.ok(offset + length <= exports.memory.buffer.byteLength);
    assert.deepEqual(Buffer.from(new Uint8Array(exports.memory.buffer, offset, length)), input);
    memoryPeak = Math.max(memoryPeak, exports.memory.buffer.byteLength);
    assert.ok(memoryPeak <= maximumPages * 65536);
    cases++;
  }
}
const {exports} = new WebAssembly.Instance(module, {});
assert.throws(() => exports.holo_alloc(maximum + 1), WebAssembly.RuntimeError);
const initialPages = exports.memory.buffer.byteLength / 65536;
assert.equal(exports.memory.grow(maximumPages - initialPages), initialPages);
assert.throws(() => exports.memory.grow(1), RangeError);
assert.equal(cases, 16);
console.log(JSON.stringify({cases, memoryPeak, maximum, maximumPages}));
