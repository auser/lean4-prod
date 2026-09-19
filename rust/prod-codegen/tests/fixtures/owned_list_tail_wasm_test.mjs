import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const inputs = ['', '|', 'é|𐐷', 'x'.repeat(262_144)];
for (const count of [1, 2, 64, 1024, 4096, 8192]) {
  inputs.push(Array.from({length: count}, () => 'x'.repeat(31)).join('|'));
}
const cases = inputs.map(text => [Buffer.from(text), Buffer.from(`head|${text}`)]);
cases.push([Buffer.from([0xff]), Buffer.alloc(0)]);
cases.push([Buffer.from('|'.repeat(8192)), Buffer.alloc(0)]);
for (const [input, expected] of cases) {
  for (let replay = 0; replay < 2; replay++) {
    const {exports} = new WebAssembly.Instance(module, {});
    const pointer = exports.holo_alloc(input.length);
    new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
    const packed = BigInt.asUintN(64, exports.holo_run(pointer, input.length));
    const offset = Number(packed >> 32n), length = Number(packed & 0xffffffffn);
    assert.ok(length <= 262_149 && offset + length <= exports.memory.buffer.byteLength);
    assert.ok(exports.memory.buffer.byteLength <= 64 * 65536);
    assert.deepEqual(Buffer.from(new Uint8Array(exports.memory.buffer, offset, length)), expected);
  }
}
const {exports} = new WebAssembly.Instance(module, {});
assert.throws(() => exports.holo_alloc(262_145), WebAssembly.RuntimeError);
console.log('PASS owned list prepend: exact bytes, UTF-8, field and allocation boundaries in import-free Wasm');
