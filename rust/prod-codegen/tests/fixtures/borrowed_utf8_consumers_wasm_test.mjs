import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
const maximum = 1_048_576;
const maximumPages = Number(process.argv[3]);
assert.ok(Number.isSafeInteger(maximumPages) && maximumPages > 0);
const encoder = new TextEncoder();
const rows = ['', 'a', 'λ', 'é', '\0', '𐀀', 'a\u0301'].map(text => {
  const bytes = encoder.encode(text); return [bytes, bytes];
});
for (const size of [8192, 65536, maximum - 1, maximum]) {
  const bytes = encoder.encode('λ'.repeat(Math.floor(size / 2)) + (size % 2 ? 'a' : ''));
  rows.push([bytes, bytes]);
}
for (const bytes of [[255], [192, 128], [237, 160, 128], [240, 159], [244, 144, 128, 128]])
  rows.push([Uint8Array.from(bytes), Uint8Array.of(255)]);
for (const [input, expected] of rows) {
  for (let repeat = 0; repeat < 2; repeat++) {
    const {exports} = new WebAssembly.Instance(module, {});
    const pointer = exports.holo_alloc(input.length);
    new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
    const packed = BigInt.asUintN(64, exports.holo_run(pointer, input.length));
    const start = Number(packed >> 32n), length = Number(packed & 0xffffffffn);
    assert.ok(start + length <= exports.memory.buffer.byteLength);
    assert.ok(exports.memory.buffer.byteLength <= maximumPages * 65536);
    assert.deepEqual(new Uint8Array(exports.memory.buffer, start, length), expected);
  }
}
const {exports} = new WebAssembly.Instance(module, {});
assert.throws(() => exports.holo_alloc(maximum + 1), WebAssembly.RuntimeError);
console.log(`PASS ${rows.length * 2} actual import-free Wasm UTF-8 cases within ${maximumPages} pages`);
