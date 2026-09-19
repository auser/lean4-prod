import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {lstatSync, readFileSync, readdirSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixture = join(root, 'fixtures/lexlean-decimal');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const read = path => {
  assert.ok(lstatSync(path).isFile(), `fixture must be a regular file: ${path}`);
  return readFileSync(path);
};
const safe = path => {
  assert.equal(typeof path, 'string');
  assert.ok(path.split('/').every(part => /^[A-Za-z0-9_.-]+$/.test(part) && !['.', '..'].includes(part)));
  return path;
};
const receipt = JSON.parse(read(join(fixture, 'fixture-manifest.json')));
assert.deepEqual(Object.keys(receipt).sort(), ['build_id', 'build_manifest_sha256', 'compiler_crate_sha256', 'compiler_revision', 'compiler_semantics_id', 'generated_path', 'generated_sha256', 'semantic_id', 'source_id', 'spec']);
assert.equal(receipt.spec, 'lean4-prod/lexlean-decimal-fixture/1');
assert.match(receipt.compiler_revision, /^[0-9a-f]{40}$/);
assert.match(receipt.compiler_crate_sha256, /^[0-9a-f]{64}$/);
const raw = read(join(fixture, 'build/manifest.json'));
assert.equal(hash(raw), receipt.build_manifest_sha256);
const manifest = JSON.parse(raw);
assert.equal(manifest.spec, 'lexlean/build-manifest/1');
assert.equal(manifest.language, '1.1');
assert.equal(manifest.compiler.version, '0.3.0');
assert.equal(manifest.compiler.semantics_id, receipt.compiler_semantics_id);
for (const key of ['build_id', 'semantic_id', 'source_id']) assert.equal(manifest[key], receipt[key]);
assert.deepEqual(manifest.selection, ['Main']);
assert.deepEqual(manifest.modules, [{lean_module: 'DecimalFixture.Main', module: 'Main', source_path: 'src/Main.lex.tex'}]);
const inputs = manifest.inputs.filter(row => row.kind !== 'lexicon');
assert.deepEqual(inputs.map(row => row.path).sort(), ['lexlean.lock', 'lexlean.toml', 'src/Main.lex.tex']);
for (const row of inputs) {
  const bytes = read(join(fixture, safe(row.path)));
  assert.equal(bytes.length, row.byte_length);
  assert.equal(hash(bytes), row.sha256);
}
const lock = read(join(fixture, 'lexlean.lock')).toString('utf8');
assert.equal(/^compiler_semantics = "([0-9a-f]{64})"$/m.exec(lock)?.[1], receipt.compiler_semantics_id);
const workspace = [...lock.matchAll(/\[\[workspace_file\]\]\npath = "([^"]+)"\nsha256 = "([0-9a-f]{64})"/g)];
assert.deepEqual(workspace.map(row => row[1]).sort(), ['lake-manifest.json', 'lakefile.toml', 'lean-toolchain']);
for (const row of workspace) assert.equal(hash(read(join(fixture, safe(row[1])))), row[2]);
assert.equal(manifest.outputs.length, 5);
assert.equal(new Set(manifest.outputs.map(row => row.path)).size, 5);
for (const row of manifest.outputs) {
  const bytes = read(join(fixture, 'build', safe(row.path)));
  assert.equal(bytes.length, row.byte_length);
  assert.equal(hash(bytes), row.sha256);
}
const files = directory => readdirSync(directory, {withFileTypes: true}).flatMap(row => {
  const path = join(directory, row.name);
  assert.ok(row.isDirectory() || row.isFile());
  return row.isDirectory() ? files(path) : [path.slice(join(fixture, 'build').length + 1)];
}).sort();
assert.deepEqual(files(join(fixture, 'build')), ['manifest.json', ...manifest.outputs.map(row => row.path)].sort());
assert.equal(receipt.generated_path, 'lean/Conformance/LexLeanDecimal.lean');
const generated = read(join(root, receipt.generated_path));
assert.equal(hash(generated), receipt.generated_sha256);
assert.deepEqual(generated, read(join(fixture, 'build/modules/DecimalFixture/Main.lean')));
assert.ok(!/\b(?:sorry|admit|axiom|opaque)\b/.test(generated.toString('utf8')));
console.log('typed decimal fixture source, compiler identity, generated module and complete output binding passed');
