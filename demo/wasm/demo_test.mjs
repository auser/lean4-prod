import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  MAX_NAT,
  addIsCommutative,
  errorMessage,
  parseNat,
  wittBits,
  wittBytes,
} from "./demo-core.mjs";

const demoDirectory = path.dirname(fileURLToPath(import.meta.url));
const packageDirectory = path.resolve(process.argv[2]);
const sdk = await import(pathToFileURL(path.join(packageDirectory, "uor.js")));
const wasm = await readFile(path.join(packageDirectory, "uor_bg.wasm"));
await sdk.default({ module_or_path: wasm });

const html = await readFile(path.join(demoDirectory, "index.html"), "utf8");
const app = await readFile(path.join(demoDirectory, "app.js"), "utf8");
for (const id of [
  "witt-form",
  "metadata-button",
  "witt-result",
  "metadata-result",
]) {
  assert.match(html, new RegExp(`id=["']${id}["']`));
  assert.match(app, new RegExp(`#${id}`));
}
assert.match(app, /output\/uor\/wasm\/uor\.js/);
assert.match(html, /51c01382200b0179d6640b07e9c8119364ab69a1/);

assert.equal(parseNat(" 42 ", "Value"), 42n);
assert.throws(() => parseNat("-1", "Value"), /non-negative whole number/);
assert.throws(() => parseNat(MAX_NAT + 1n, "Value"), /unsigned 64-bit/);
assert.equal(wittBits(sdk, "64"), 64n);
assert.equal(wittBytes(sdk, "64"), 8n);
assert.equal(wittBytes(sdk, "4096"), 512n);
assert.equal(addIsCommutative(sdk), false);
assert.equal(errorMessage(new Error("visible")), "visible");
