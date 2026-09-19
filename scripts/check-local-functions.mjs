import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const directory = path.resolve(process.argv[2]);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
function run(program, args) {
  const result = spawnSync(program, args, {
    cwd: directory,
    encoding: 'utf8',
    timeout: 120_000,
    maxBuffer: 4 * 1024 * 1024,
  });
  assert.equal(result.error, undefined, `${program}: ${result.error}`);
  assert.equal(result.status, 0, `${program}: ${result.stdout}\n${result.stderr}`);
  return result.stdout;
}
const coverage = JSON.parse(fs.readFileSync(path.join(directory, 'coverage.json')));
assert.deepEqual(coverage.opaque_nodes, []);
assert.deepEqual(coverage.external_calls, []);
const ir = fs.readFileSync(path.join(directory, 'kernel.ir'), 'utf8');
assert.match(ir, /\(jp /, 'the real shared branch is exercised');
assert.match(ir, /\(jmp /);
const generated = fs.readFileSync(path.join(directory, 'generated.rs'), 'utf8');
const ffi = fs.readFileSync(path.join(directory, 'ffi.rs'), 'utf8');
const errors = JSON.stringify(path.join(root, 'rust/prod-core/src/error.rs'));
const common = `#![allow(non_snake_case, unused_variables, unused_parens)]
${generated}
#[path = ${errors}] mod compute_error;
pub use compute_error::ComputeError;
`;
const runner = `extern crate local_functions;
fn main() {
    let mut count = 0;
    for mode in [0, 1, 2, 3, 99, u64::MAX] {
        for state in [0, 17, 42, u64::MAX] {
            for candidate in [0, 17, 42, u64::MAX] {
                assert_eq!(local_functions::sharedEnumScalar(mode >= 2, mode % 2 == 1, state, candidate),
                    candidate == if mode == 1 || mode == 2 { state } else { 17 });
                count += 1;
            }
        }
    }
    assert_eq!(count, 96);
    for input in [0, 1, 2147483648, 4294967295, 4294967296, u64::MAX] {
        assert_eq!(local_functions::largeNatReceiver(input), 4294967295_u64.saturating_sub(input));
    }
    println!("96 shared-branch cases passed");
}
`;
fs.writeFileSync(path.join(directory, 'runner.rs'), runner);
for (const standard of [true, false]) {
  for (const optimized of [false, true]) {
    fs.writeFileSync(path.join(directory, 'library.rs'), `${standard ? '' : '#![no_std]\n'}${common}`);
    const profile = optimized ? ['-C', 'opt-level=3', '-C', 'overflow-checks=yes'] : [];
    run('rustc', ['--edition=2021', '--crate-name=local_functions', '--crate-type=rlib',
      ...profile, 'library.rs', '-o', 'liblocal_functions.rlib']);
    run('rustc', ['--edition=2021', ...profile, 'runner.rs', '--extern',
      'local_functions=liblocal_functions.rlib', '-o', 'runner']);
    assert.equal(run(path.join(directory, 'runner'), []), '96 shared-branch cases passed\n');
  }
}
fs.writeFileSync(path.join(directory, 'library.rs'), `${common}\n${ffi}`);
run('rustc', ['--edition=2021', '--crate-name=local_functions', '--crate-type=cdylib',
  'library.rs', '-o', 'liblocal_functions.so']);
fs.writeFileSync(path.join(directory, 'runner.c'), `#include "fixture.h"
#include <assert.h>
#include <stdint.h>
int main(void) {
    uint64_t modes[] = {0, 1, 2, 3, 99, UINT64_MAX};
    uint64_t values[] = {0, 17, 42, UINT64_MAX};
    unsigned count = 0;
    for (unsigned m = 0; m < 6; ++m) {
        for (unsigned s = 0; s < 4; ++s) {
            for (unsigned c = 0; c < 4; ++c) {
                uint64_t expected = modes[m] == 1 || modes[m] == 2 ? values[s] : 17;
                assert(prod_sharedenum_sharedenumscalar(modes[m] >= 2, modes[m] % 2 == 1, values[s], values[c]) ==
                    (values[c] == expected));
                ++count;
            }
        }
    }
    assert(count == 96);
    uint64_t operands[] = {0, 1, UINT64_C(2147483648), UINT64_C(4294967295), UINT64_C(4294967296), UINT64_MAX};
    for (unsigned i = 0; i < 6; ++i) {
        uint64_t expected = operands[i] > UINT64_C(4294967295) ? 0 : UINT64_C(4294967295) - operands[i];
        assert(prod_sharedenum_largenatreceiver(operands[i]) == expected);
    }
    return 0;
}
`);
run('clang', ['-std=c11', '-Wall', '-Wextra', '-Werror', 'runner.c', '-L.',
  '-llocal_functions', `-Wl,-rpath,${directory}`, '-o', 'c-runner']);
run(path.join(directory, 'c-runner'), []);
run('rustc', ['--edition=2021', '--crate-name=local_functions', '--crate-type=cdylib',
  '--target=wasm32-unknown-unknown', 'library.rs', '-o', 'fixture.wasm']);
const wasm = new WebAssembly.Module(fs.readFileSync(path.join(directory, 'fixture.wasm')));
assert.deepEqual(WebAssembly.Module.imports(wasm), []);
const exports = new WebAssembly.Instance(wasm).exports;
const entry = exports.prod_sharedenum_sharedenumscalar;
assert.equal(typeof entry, 'function');
let count = 0;
for (const mode of [0n, 1n, 2n, 3n, 99n, 2n ** 64n - 1n]) {
  for (const state of [0n, 17n, 42n, 2n ** 64n - 1n]) {
    for (const candidate of [0n, 17n, 42n, 2n ** 64n - 1n]) {
      const expected = candidate === (mode === 1n || mode === 2n ? state : 17n);
      assert.equal(entry(Number(mode >= 2n), Number(mode % 2n === 1n), state, candidate), Number(expected));
      ++count;
    }
  }
}
assert.equal(count, 96);
for (const input of [0n, 1n, 2147483648n, 4294967295n, 4294967296n, 2n ** 64n - 1n]) {
  const expected = input > 4294967295n ? 0n : 4294967295n - input;
  assert.equal(exports.prod_sharedenum_largenatreceiver(input), expected);
}
console.log('shared-branch export: native std/no_std/debug/optimized, C and Wasm passed');
