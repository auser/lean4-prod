import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const module = await WebAssembly.compile(await readFile(process.argv[2]));
const fallible = process.argv[3] === "fallible";
assert.deepEqual(WebAssembly.Module.imports(module), []);
const guest = (await WebAssembly.instantiate(module, {})).exports;
assert.deepEqual(Object.keys(guest).sort(), ["holo_alloc", "holo_run", "memory"]);

function invoke(bytes) {
  const pointer = guest.holo_alloc(bytes.length);
  new Uint8Array(guest.memory.buffer, pointer, bytes.length).set(bytes);
  return guest.holo_run(pointer, bytes.length);
}

function output(packed) {
  const value = BigInt.asUintN(64, packed);
  const pointer = Number(value >> 32n);
  const length = Number(value & 0xffff_ffffn);
  assert.equal(pointer % 8, 0);
  return [...new Uint8Array(guest.memory.buffer, pointer, length)];
}

// Real successful execution, including empty, arbitrary binary and resident calls.
const firstInput = [0, 1, 127, 128, 255];
const first = invoke(firstInput);
assert.deepEqual(output(first), firstInput);
assert.deepEqual(output(invoke([])), []);
assert.deepEqual(output(invoke([9, 8, 7])), [9, 8, 7]);
assert.deepEqual(output(first), firstInput);

if (fallible) {
  assert.throws(() => invoke([255]), WebAssembly.RuntimeError);
  // The failing call returns no success value; a later valid call remains usable.
  assert.deepEqual(output(invoke([1, 2])), [1, 2]);
  assert.deepEqual(output(first), firstInput);
} else {
  assert.deepEqual(output(invoke([255])), [255]);
}

assert.deepEqual(output(invoke(Array(64).fill(42))), Array(64).fill(42));
assert.throws(() => invoke(Array(65).fill(42)), WebAssembly.RuntimeError);
assert.throws(() => guest.holo_alloc(129), WebAssembly.RuntimeError);
assert.throws(() => guest.holo_run(-1, 0), WebAssembly.RuntimeError);
console.log(`Core-Wasm ${fallible ? "fallible" : "infallible"} Bytes ABI passed`);
