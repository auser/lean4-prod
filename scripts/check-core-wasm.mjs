import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const bytes = await readFile(process.argv[2]);

async function fresh() {
  const { instance } = await WebAssembly.instantiate(bytes, {});
  assert.deepEqual(Object.keys(instance.exports).sort(), ["holo_alloc", "holo_run", "memory"]);
  return instance.exports;
}

const guest = await fresh();
const input = Uint8Array.from([0, 1, 2, 127, 128, 255]);
const inputPointer = guest.holo_alloc(input.length);
assert.equal(inputPointer % 8, 0);
new Uint8Array(guest.memory.buffer, inputPointer, input.length).set(input);
const packed = BigInt.asUintN(64, guest.holo_run(inputPointer, input.length));
const outputPointer = Number(packed >> 32n);
const outputLength = Number(packed & 0xffff_ffffn);
assert.equal(outputPointer % 8, 0);
assert.equal(outputLength, input.length);
assert.deepEqual(
  [...new Uint8Array(guest.memory.buffer, outputPointer, outputLength)],
  [...input],
);

// A resident instance may be called repeatedly. Its monotonic allocator keeps
// the first returned output readable and unchanged until the instance drops.
const second = Uint8Array.from([9, 8, 7]);
const secondInputPointer = guest.holo_alloc(second.length);
new Uint8Array(guest.memory.buffer, secondInputPointer, second.length).set(second);
const secondPacked = BigInt.asUintN(64, guest.holo_run(secondInputPointer, second.length));
const secondOutputPointer = Number(secondPacked >> 32n);
const secondOutputLength = Number(secondPacked & 0xffff_ffffn);
assert.deepEqual(
  [...new Uint8Array(guest.memory.buffer, outputPointer, outputLength)],
  [...input],
);
assert.deepEqual(
  [...new Uint8Array(guest.memory.buffer, secondOutputPointer, secondOutputLength)],
  [...second],
);

assert.throws(() => guest.holo_alloc(-1), WebAssembly.RuntimeError);
assert.throws(() => guest.holo_alloc(65_537), WebAssembly.RuntimeError);
assert.throws(() => guest.holo_run(-1, 0), WebAssembly.RuntimeError);
assert.throws(() => guest.holo_run(0, 65_537), WebAssembly.RuntimeError);

const another = await fresh();
assert.equal(another.holo_alloc(0), inputPointer);

console.log("Core-Wasm ABI execution passed");
