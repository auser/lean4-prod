import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const packageDirectory = path.resolve(process.argv[2]);
const sdk = await import(pathToFileURL(path.join(packageDirectory, "uor.js")));
const wasm = await readFile(path.join(packageDirectory, "uor_bg.wasm"));
await sdk.default({ module_or_path: wasm });

assert.equal(sdk.prod_uor_framework_wittbits(8n), 8n);
assert.equal(sdk.prod_uor_framework_wittbits(4096n), 4096n);
assert.equal(sdk.prod_uor_framework_wittbytes(8n), 1n);
assert.equal(sdk.prod_uor_framework_wittbytes(4096n), 512n);
assert.equal(sdk.prod_uor_framework_addiscommutative(), false);
