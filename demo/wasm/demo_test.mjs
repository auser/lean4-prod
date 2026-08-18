import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  MAX_NAT,
  add,
  errorMessage,
  less,
  parseNat,
  triggerOverflow,
} from "./demo-core.mjs";

const demoDirectory = path.dirname(fileURLToPath(import.meta.url));
const packageDirectory = path.resolve(process.argv[2]);
const sdk = await import(pathToFileURL(path.join(packageDirectory, "fixture.js")));
const wasm = await readFile(path.join(packageDirectory, "fixture_bg.wasm"));
await sdk.default({ module_or_path: wasm });

const html = await readFile(path.join(demoDirectory, "index.html"), "utf8");
const app = await readFile(path.join(demoDirectory, "app.js"), "utf8");
for (const id of ["add-form", "less-form", "overflow-button"]) {
  assert.match(html, new RegExp(`id=["']${id}["']`));
  assert.match(app, new RegExp(`#${id}`));
}
assert.match(app, /output\/fixture\/wasm\/fixture\.js/);

assert.equal(parseNat(" 42 ", "Value"), 42n);
assert.throws(() => parseNat("-1", "Value"), /non-negative whole number/);
assert.throws(() => parseNat(MAX_NAT + 1n, "Value"), /unsigned 64-bit/);
assert.equal(add(sdk, "40", "2"), 42n);
assert.equal(less(sdk, "7", "11"), true);
assert.equal(less(sdk, "11", "7"), false);
assert.throws(() => triggerOverflow(sdk), /addition overflow/);
assert.equal(errorMessage(new Error("visible")), "visible");
