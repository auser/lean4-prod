import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
assert.deepEqual(WebAssembly.Module.imports(module), []);
function invoke(input) {
  const {exports} = new WebAssembly.Instance(module, {});
  const pointer = exports.holo_alloc(input.length);
  new Uint8Array(exports.memory.buffer, pointer, input.length).set(input);
  const packed = BigInt.asUintN(64, exports.holo_run(pointer, input.length));
  const start = Number(packed >> 32n), length = Number(packed & 0xffffffffn);
  assert.ok(length <= 4 && start + length <= exports.memory.buffer.byteLength);
  assert.ok(exports.memory.buffer.byteLength <= 4 * 65536);
  return [...new Uint8Array(exports.memory.buffer, start, length)];
}
const encoder = new TextEncoder();
const rows = [
  ['', [1, 0, 1, 1]], ['a', [1, 0, 1, 1]], ['|', [1, 0, 0, 1]],
  ['a|b', [1, 0, 0, 1]], ['a|b|c', [1, 0, 0, 0]], ['a||', [1, 0, 0, 0]],
  ['é|𐐷', [1, 0, 0, 1]], ['a\0|b', [1, 0, 0, 1]],
  ['x'.repeat(128), [1, 0, 1, 1]], ['|'.repeat(128), [1, 0, 0, 0]],
];
for (const [text, expected] of rows) for (let repeat = 0; repeat < 2; repeat++)
  assert.deepEqual(invoke(encoder.encode(text)), expected, JSON.stringify(text));
assert.deepEqual(invoke(Uint8Array.of(255)), [255]);
assert.throws(() => invoke(new Uint8Array(129)), WebAssembly.RuntimeError);
console.log(`PASS ${rows.length} maximum/zero/one/two-field cases twice in actual import-free Wasm; UTF-8 and allocation bounds`);
