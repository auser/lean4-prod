// Compiler-owned transport; no enrollment, authority, network or application policy.
const mounts = new WeakSet();
class WorkspaceBootstrapError extends Error {
  constructor() {
    super('workspace-bootstrap-unavailable');
    Object.defineProperties(this, {
      name: {value: 'WorkspaceBootstrapError', enumerable: true},
      code: {value: 'workspace-bootstrap-unavailable', enumerable: true},
    });
  }
}
const fail = () => new WorkspaceBootstrapError();
const hexadecimal = bytes => [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
async function verified(record, signal) {
  const url = new URL(record.path, import.meta.url);
  const response = await fetch(url, {credentials: 'omit', redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer', signal});
  if (!response.ok || response.url !== url.href || !response.body) throw fail();
  const bytes = new Uint8Array(record.size), reader = response.body.getReader();
  let offset = 0;
  try {
    while (true) {
      const part = await reader.read();
      if (part.done) break;
      if (!part.value.length || part.value.length > bytes.length - offset) throw fail();
      bytes.set(part.value, offset); offset += part.value.length;
    }
  } finally { try { await reader.cancel(); } catch { /* No payload escapes. */ } reader.releaseLock(); }
  if (offset !== bytes.length || hexadecimal(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))) !== record.sha256) throw fail();
  return bytes;
}
async function load() {
  const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 15000), urls = new Map();
  try {
    const content = new Map();
    // Sequential fetches bound concurrency and make failure cancel the whole load.
    for (const record of [...BINDING.sdk, ...BINDING.guests, BINDING.labels]) content.set(record.path, await verified(record, controller.signal));
    const decoder = new TextDecoder('utf-8', {fatal: true, ignoreBOM: true});
    for (const record of BINDING.modules) {
      let source = decoder.decode(content.get(record.path));
      for (const dependency of record.dependencies) {
        const before = "} from './" + dependency + "';", url = urls.get('sdk/browser/' + dependency);
        if (!url || source.split(before).length !== 2) throw fail();
        source = source.replace(before, '} from ' + JSON.stringify(url) + ';');
      }
      urls.set(record.path, URL.createObjectURL(new Blob([source], {type: 'text/javascript'})));
    }
    const modules = {};
    for (const record of BINDING.guests) {
      const module = await WebAssembly.compile(content.get(record.path));
      if (WebAssembly.Module.imports(module).length) throw fail();
      modules[record.role + 'Module'] = module;
    }
    const {openWorkspaceView} = await import(urls.get('sdk/browser/view-host.mjs'));
    if (typeof openWorkspaceView !== 'function') throw fail();
    return {openWorkspaceView, modules, labels: content.get(BINDING.labels.path)};
  } catch { throw fail(); }
  finally { clearTimeout(timer); controller.abort(); for (const url of urls.values()) URL.revokeObjectURL(url); }
}
// Trusted bootstrap only. Identity and organization enrollment are prerequisites,
// never synthesized from an email address, a local flag or the public asset set.
export async function mount(root, store, headName) {
  try {
    if (arguments.length !== 3 || !(root instanceof HTMLElement) || !root.isConnected || mounts.has(root)
      || typeof headName !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(headName)) throw fail();
  } catch { throw fail(); }
  mounts.add(root);
  try {
    const {openWorkspaceView, modules, labels} = await load();
    const view = await openWorkspaceView({...modules, labels, root, store, headName});
    let closed = false;
    return Object.freeze({
      dispatch: function (bytes) { if (arguments.length !== 1 || closed) throw fail(); return view.dispatch(bytes); },
      close: function () {
        if (arguments.length !== 0) throw fail();
        if (!closed) { closed = true; try { view.close(); } finally { mounts.delete(root); } }
      },
    });
  } catch { mounts.delete(root); try { root.replaceChildren(); } catch { /* Terminal failure stays payload-free. */ } throw fail(); }
}
