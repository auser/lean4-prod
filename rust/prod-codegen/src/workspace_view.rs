//! Allocation-conscious, deterministic browser-only Workspace View projection.
//!
//! The compiler verifies bindings, not their authority. The SDK caller MUST
//! supply kernel-verified model/IR evidence from one immutable source snapshot
//! and its own verified installed host inventory. An application model cannot
//! select JavaScript, dependency URLs or SDK digests. This API does not enroll
//! identities, infer organization authority, implement replication, or authorize
//! publication. Those remain separately modeled and verified requirements.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use serde_json::{json, Value};
use wasmparser::{ExternalKind, Parser, Payload, ValType, Validator};

use crate::view::{file, sha256, valid_digest};
use crate::PackageFile;

const MAX_WASM_BYTES: usize = 4 * 1024 * 1024;
const MAX_SDK_BYTES: usize = 256 * 1024;
const LABEL_KEYS: &[&str] = &[
    "action",
    "action0",
    "action1",
    "action2",
    "action3",
    "action4",
    "asOf",
    "author",
    "body",
    "close",
    "closed",
    "conflict",
    "contributor",
    "event",
    "inputError",
    "members",
    "message",
    "messages",
    "next",
    "none",
    "offset",
    "owner",
    "pending",
    "principal",
    "reader",
    "ready",
    "refresh",
    "rejected",
    "replay",
    "result",
    "role",
    "select",
    "spec",
    "submit",
    "title",
    "total",
    "unavailable",
    "unknown",
    "workspace",
];
// Dependency order, not output order. The loader imports the verified in-memory
// graph, never a second network response after hashing the first one.
const SDK_MODULES: &[(&str, &[&str])] = &[
    ("identity.mjs", &[]),
    ("view-error.mjs", &[]),
    ("store.mjs", &["identity.mjs"]),
    ("journal.mjs", &["identity.mjs"]),
    ("commands.mjs", &["identity.mjs", "journal.mjs"]),
    ("queries.mjs", &["identity.mjs", "journal.mjs"]),
    ("view-dom.mjs", &["identity.mjs", "view-error.mjs"]),
    (
        "view-host.mjs",
        &[
            "identity.mjs",
            "commands.mjs",
            "queries.mjs",
            "view-dom.mjs",
            "view-error.mjs",
        ],
    ),
];

/// The four closed generated guests. Each has a distinct fixed ABI/resource role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkspaceGuestRole {
    View,
    Command,
    Query,
    Journal,
}

impl WorkspaceGuestRole {
    fn name(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Command => "command",
            Self::Query => "query",
            Self::Journal => "journal",
        }
    }
    fn root(self) -> &'static str {
        match self {
            Self::View => {
                "PrismPM.Foundation.View.Workspace.V1.Interaction.workspaceInteractionBytes"
            }
            Self::Command => "PrismPM.Foundation.Browser.V1.WorkspaceCommand.workspaceCommandBytes",
            Self::Query => "PrismPM.Foundation.Browser.V1.WorkspaceQuery.workspaceQueryBytes",
            Self::Journal => "PrismPM.Foundation.Browser.V1.WorkspaceJournal.workspaceJournalBytes",
        }
    }
    fn caps(self) -> (u32, u32, u32) {
        match self {
            Self::View => (133728, 71055, 128),
            Self::Command => (139873, 74243, 64),
            Self::Query => (1166279, 66803, 512),
            Self::Journal => (1235980, 1166008, 640),
        }
    }
}

/// One compiler-built raw Core-Wasm guest and its verified source binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGuest {
    pub role: WorkspaceGuestRole,
    pub root: String,
    pub source_snapshot_sha256: String,
    pub ir_sha256: String,
    pub proof_sha256: String,
    pub wasm_sha256: String,
    pub wasm: Vec<u8>,
    pub request_maximum: u32,
    pub response_maximum: u32,
    pub maximum_pages: u32,
}

/// Closed, evaluated labels and model identity. No executable fields exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceViewV1 {
    pub model_id: String,
    pub view_model_id: String,
    pub source_snapshot_sha256: String,
    pub labels_root: String,
    pub labels_proof_sha256: String,
    pub labels_sha256: String,
    pub labels: Vec<u8>,
}

/// A trusted SDK-owned asset; NEVER obtained from application model fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSdkAsset {
    pub path: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

/// Verified SDK inventory plus four compiler-built guests from the same source
/// snapshot as the evaluated model. The caller authenticates `sdk_inventory`;
/// it is not a self-signed authority claim accepted from a producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBrowserBinding {
    pub sdk_inventory_sha256: String,
    pub sdk_assets: Vec<WorkspaceSdkAsset>,
    pub guests: Vec<WorkspaceGuest>,
}

/// Browser-only assets. Unlike Numeric/Text this has no implied Hologram View
/// oracle, wasm-bindgen adapter, or capability-free legacy release claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedWorkspaceViewV1 {
    pub browser_assets: Vec<PackageFile>,
    pub view_manifest: PackageFile,
}

/// Closed, payload-free failures; untrusted bytes never enter diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceViewError {
    ModelBinding,
    Labels,
    SdkInventory,
    SdkAsset,
    SdkDependency,
    GuestSet,
    GuestBinding,
    GuestModule,
    GuestAbi,
    GuestMemory,
}
impl fmt::Display for WorkspaceViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ModelBinding => "invalid workspace model binding",
            Self::Labels => "invalid canonical workspace labels",
            Self::SdkInventory => "invalid workspace SDK inventory",
            Self::SdkAsset => "invalid workspace SDK asset",
            Self::SdkDependency => "invalid workspace SDK dependency closure",
            Self::GuestSet => "invalid workspace guest set",
            Self::GuestBinding => "invalid workspace guest source binding",
            Self::GuestModule => "invalid workspace guest module",
            Self::GuestAbi => "invalid workspace guest ABI",
            Self::GuestMemory => "invalid workspace guest memory or table limits",
        })
    }
}
impl core::error::Error for WorkspaceViewError {}

fn qualified_root(value: &str) -> bool {
    value.len() <= 256
        && value.contains('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphabetic() || byte == b'_' || index > 0 && byte.is_ascii_digit()
                })
        })
}

fn labels(view: &WorkspaceViewV1) -> Result<Value, WorkspaceViewError> {
    if [
        &view.model_id,
        &view.view_model_id,
        &view.source_snapshot_sha256,
        &view.labels_proof_sha256,
        &view.labels_sha256,
    ]
    .iter()
    .any(|digest| !valid_digest(digest))
        || !qualified_root(&view.labels_root)
    {
        return Err(WorkspaceViewError::ModelBinding);
    }
    if view.labels.is_empty()
        || view.labels.len() > 8192
        || sha256(&view.labels) != view.labels_sha256
    {
        return Err(WorkspaceViewError::Labels);
    }
    let value: Value =
        serde_json::from_slice(&view.labels).map_err(|_| WorkspaceViewError::Labels)?;
    let object = value.as_object().ok_or(WorkspaceViewError::Labels)?;
    if object
        .keys()
        .map(String::as_str)
        .ne(LABEL_KEYS.iter().copied())
        || object.values().any(|item| {
            item.as_str()
                .is_none_or(|text| text.is_empty() || text.len() > 256)
        })
        || object.get("spec").and_then(Value::as_str) != Some("prismpm/workspace-view-labels/1")
        || serde_json::to_vec(&value).map_err(|_| WorkspaceViewError::Labels)? != view.labels
    {
        return Err(WorkspaceViewError::Labels);
    }
    Ok(value)
}

fn js_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || matches!(byte, b'_' | b'$')
                || index > 0 && byte.is_ascii_digit()
        })
}

// Deliberately closed SDK source convention, not a JavaScript parser. Only
// single-line declaration headers and local named exports are admitted. Other
// spellings (including comments, multiline headers and every re-export) fail
// closed; dependency keywords in strings/comments are conservatively rejected.
fn sdk_export(line: &str) -> bool {
    let Some(rest) = line.trim().strip_prefix("export ") else {
        return false;
    };
    if let Some(names) = rest.strip_prefix('{').and_then(|s| s.strip_suffix("};")) {
        return names.split(',').all(|name| {
            let mut words = name.split_ascii_whitespace();
            match (words.next(), words.next(), words.next(), words.next()) {
                (Some(name), None, None, None) => js_identifier(name),
                (Some(name), Some("as"), Some(alias), None) => {
                    js_identifier(name) && js_identifier(alias)
                }
                _ => false,
            }
        });
    }
    for prefix in ["const ", "class ", "function ", "async function "] {
        if let Some(declaration) = rest.strip_prefix(prefix) {
            let end = declaration
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                .unwrap_or(declaration.len());
            if !js_identifier(&declaration[..end]) {
                return false;
            }
            let tail = declaration[end..].trim_start();
            return match prefix {
                "const " => tail.starts_with('='),
                "class " => {
                    tail.starts_with('{')
                        || tail.strip_prefix("extends ").is_some_and(|parent| {
                            parent
                                .split_once('{')
                                .is_some_and(|(name, _)| js_identifier(name.trim_end()))
                        })
                }
                _ => tail.starts_with('('),
            };
        }
    }
    false
}

fn sdk(binding: &WorkspaceBrowserBinding) -> Result<Vec<Value>, WorkspaceViewError> {
    if !valid_digest(&binding.sdk_inventory_sha256) || binding.sdk_assets.len() != SDK_MODULES.len()
    {
        return Err(WorkspaceViewError::SdkInventory);
    }
    let mut hashes = BTreeSet::new();
    let mut previous: Option<&str> = None;
    let mut inventory = Vec::new();
    for asset in &binding.sdk_assets {
        let name = asset
            .path
            .strip_prefix("sdk/browser/")
            .ok_or(WorkspaceViewError::SdkAsset)?;
        let (_, dependencies) = SDK_MODULES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .ok_or(WorkspaceViewError::SdkAsset)?;
        if previous.is_some_and(|path| path >= asset.path.as_str())
            || asset.bytes.is_empty()
            || asset.bytes.len() > MAX_SDK_BYTES
            || !valid_digest(&asset.sha256)
            || sha256(&asset.bytes) != asset.sha256
            || !hashes.insert(&asset.sha256)
        {
            return Err(WorkspaceViewError::SdkAsset);
        }
        previous = Some(&asset.path);
        let source =
            core::str::from_utf8(&asset.bytes).map_err(|_| WorkspaceViewError::SdkAsset)?;
        let mut imports = Vec::new();
        for line in source.lines() {
            if !line.trim_start().starts_with("import") {
                // SDK assets are trusted code, but this profile permits only
                // its fixed static imports. Reject every remaining import token
                // conservatively, including strings/comments, rather than accept
                // dynamic import, import.meta or obfuscated dependency fallbacks.
                let mut exports = 0;
                for token in line.split(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '_' && character != '$'
                }) {
                    if token == "import" {
                        return Err(WorkspaceViewError::SdkDependency);
                    }
                    exports += usize::from(token == "export");
                }
                if exports > 1 || exports == 1 && !sdk_export(line) {
                    return Err(WorkspaceViewError::SdkDependency);
                }
                continue;
            }
            let line = line.trim();
            let (bindings, tail) = line
                .strip_prefix("import {")
                .and_then(|rest| rest.split_once("} from './"))
                .ok_or(WorkspaceViewError::SdkDependency)?;
            let dependency = tail
                .strip_suffix("';")
                .ok_or(WorkspaceViewError::SdkDependency)?;
            if bindings.is_empty()
                || bindings.bytes().any(|byte| {
                    !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b',' | b' ')
                })
                || imports.contains(&dependency)
            {
                return Err(WorkspaceViewError::SdkDependency);
            }
            imports.push(dependency);
        }
        if imports.as_slice() != *dependencies {
            return Err(WorkspaceViewError::SdkDependency);
        }
        inventory.push(json!({"path":asset.path,"sha256":asset.sha256,"size":asset.bytes.len()}));
    }
    let bytes = serde_json::to_vec(&inventory).map_err(|_| WorkspaceViewError::SdkInventory)?;
    if sha256(&bytes) != binding.sdk_inventory_sha256 {
        return Err(WorkspaceViewError::SdkInventory);
    }
    Ok(inventory)
}

fn wasm(guest: &WorkspaceGuest) -> Result<(), WorkspaceViewError> {
    if guest.wasm.len() < 8
        || guest.wasm.len() > MAX_WASM_BYTES
        || !guest.wasm.starts_with(b"\0asm\x01\0\0\0")
    {
        return Err(WorkspaceViewError::GuestModule);
    }
    Validator::new()
        .validate_all(&guest.wasm)
        .map_err(|_| WorkspaceViewError::GuestModule)?;
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut memories = 0;
    let mut required = BTreeSet::new();
    for payload in Parser::new(0).parse_all(&guest.wasm) {
        match payload.map_err(|_| WorkspaceViewError::GuestModule)? {
            Payload::ImportSection(section) if section.count() != 0 => {
                return Err(WorkspaceViewError::GuestAbi)
            }
            Payload::StartSection { .. } => return Err(WorkspaceViewError::GuestAbi),
            // The closed raw guest profile emits no indirect-call tables. Even
            // an unexported table can allocate outside the linear-memory cap.
            Payload::TableSection(section) if section.count() != 0 => {
                return Err(WorkspaceViewError::GuestMemory)
            }
            Payload::TypeSection(section) => {
                for item in section.into_iter_err_on_gc_types() {
                    types.push(item.map_err(|_| WorkspaceViewError::GuestAbi)?);
                }
            }
            Payload::FunctionSection(section) => {
                for item in section {
                    functions.push(item.map_err(|_| WorkspaceViewError::GuestAbi)?);
                }
            }
            Payload::MemorySection(section) => {
                for item in section {
                    let memory = item.map_err(|_| WorkspaceViewError::GuestMemory)?;
                    memories += 1;
                    if memories != 1
                        || memory.memory64
                        || memory.shared
                        || memory.page_size_log2.is_some()
                        || memory.initial == 0
                        || memory.initial > u64::from(guest.maximum_pages)
                        || memory.maximum != Some(u64::from(guest.maximum_pages))
                    {
                        return Err(WorkspaceViewError::GuestMemory);
                    }
                }
            }
            Payload::ExportSection(section) => {
                for item in section {
                    let export = item.map_err(|_| WorkspaceViewError::GuestAbi)?;
                    match (export.name, export.kind) {
                        ("memory", ExternalKind::Memory) if export.index == 0 => {
                            required.insert("memory");
                        }
                        ("holo_alloc" | "holo_run", ExternalKind::Func) => {
                            let index = functions
                                .get(export.index as usize)
                                .ok_or(WorkspaceViewError::GuestAbi)?;
                            let signature = types
                                .get(*index as usize)
                                .ok_or(WorkspaceViewError::GuestAbi)?;
                            let (parameters, results): (&[ValType], &[ValType]) =
                                if export.name == "holo_alloc" {
                                    (&[ValType::I32], &[ValType::I32])
                                } else {
                                    (&[ValType::I32, ValType::I32], &[ValType::I64])
                                };
                            if signature.params() != parameters || signature.results() != results {
                                return Err(WorkspaceViewError::GuestAbi);
                            }
                            required.insert(export.name);
                        }
                        ("__data_end" | "__heap_base", ExternalKind::Global) => {}
                        _ => return Err(WorkspaceViewError::GuestAbi),
                    }
                }
            }
            _ => {}
        }
    }
    if memories != 1 {
        return Err(WorkspaceViewError::GuestMemory);
    }
    if required.len() != 3 {
        return Err(WorkspaceViewError::GuestAbi);
    }
    Ok(())
}

fn guest_records(
    view: &WorkspaceViewV1,
    binding: &WorkspaceBrowserBinding,
) -> Result<Vec<Value>, WorkspaceViewError> {
    if binding.guests.len() != 4 {
        return Err(WorkspaceViewError::GuestSet);
    }
    let mut roles = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    let mut records = Vec::new();
    for guest in &binding.guests {
        if !roles.insert(guest.role) || !hashes.insert(&guest.wasm_sha256) {
            return Err(WorkspaceViewError::GuestSet);
        }
        if guest.root != guest.role.root()
            || guest.source_snapshot_sha256 != view.source_snapshot_sha256
            || [&guest.ir_sha256, &guest.proof_sha256, &guest.wasm_sha256]
                .iter()
                .any(|digest| !valid_digest(digest))
            || guest.wasm.len() > MAX_WASM_BYTES
            || sha256(&guest.wasm) != guest.wasm_sha256
            || (
                guest.request_maximum,
                guest.response_maximum,
                guest.maximum_pages,
            ) != guest.role.caps()
        {
            return Err(WorkspaceViewError::GuestBinding);
        }
        wasm(guest)?;
        records.push(json!({"abi":"hologram:guest/core-wasm@1","maximum_pages":guest.maximum_pages,
            "path":format!("guests/{}.wasm",guest.role.name()),"proof_sha256":guest.proof_sha256,
            "request_maximum":guest.request_maximum,"response_maximum":guest.response_maximum,
            "role":guest.role.name(),"root":guest.root,"ir_sha256":guest.ir_sha256,
            "sha256":guest.wasm_sha256,"size":guest.wasm.len(),"source_snapshot_sha256":guest.source_snapshot_sha256}));
    }
    records.sort_by(|left, right| left["role"].as_str().cmp(&right["role"].as_str()));
    Ok(records)
}

/// Generate a closed deterministic asset tree without I/O, ambient SDK lookup,
/// network access or application code interpretation. Allocation is limited by
/// 4 × 4 MiB guests, 8 × 256 KiB SDK modules and an 8 KiB labels artifact.
///
/// `workspace.mjs` exports only `mount(root, store, headName)`. The trusted
/// bootstrap must provide an ALREADY enrolled identity via the selected store.
/// No index.html is emitted. These are composable component assets, not a
/// publishable application. A separately verified modeled enrollment/bootstrap
/// composition is required before a deployment projection can be accepted.
pub fn generate_workspace_view_v1(
    view: &WorkspaceViewV1,
    binding: &WorkspaceBrowserBinding,
) -> Result<GeneratedWorkspaceViewV1, WorkspaceViewError> {
    labels(view)?;
    let inventory = sdk(binding)?;
    let guests = guest_records(view, binding)?;
    let mut assets = binding
        .sdk_assets
        .iter()
        .map(|asset| PackageFile {
            path: asset.path.clone(),
            bytes: asset.bytes.clone(),
        })
        .collect::<Vec<_>>();
    for guest in &binding.guests {
        assets.push(PackageFile {
            path: format!("guests/{}.wasm", guest.role.name()),
            bytes: guest.wasm.clone(),
        });
    }
    assets.push(PackageFile {
        path: "labels.json".into(),
        bytes: view.labels.clone(),
    });
    let descriptor = json!({"guests":guests,"labels":{"path":"labels.json","sha256":view.labels_sha256,"size":view.labels.len()},
        "modules":SDK_MODULES.iter().map(|(name,deps)| json!({"path":format!("sdk/browser/{name}"),"dependencies":deps})).collect::<Vec<_>>(),
        "sdk":inventory});
    let descriptor =
        serde_json::to_string(&descriptor).map_err(|_| WorkspaceViewError::ModelBinding)?;
    assets.push(file(
        "workspace.mjs",
        format!(
            "const BINDING = {descriptor};\n{}",
            include_str!("workspace_view_browser.js")
        ),
    ));
    assets.push(file("workspace.css", "*{box-sizing:border-box}body{font-family:system-ui,sans-serif;margin:0;color:#172033;background:#f6f7fb}main{width:min(70rem,calc(100% - 2rem));margin:2rem auto;padding:1.5rem;background:white}form{display:grid;gap:.75rem}label{font-weight:600}button,input,textarea,select{font:inherit;min-height:2.75rem;max-width:100%;padding:.5rem}button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible{outline:3px solid #234fdb;outline-offset:3px}table{display:block;max-width:100%;overflow:auto;border-collapse:collapse}td,th{text-align:left;padding:.5rem;border:1px solid #c9d1df;overflow-wrap:anywhere}output{display:block;white-space:pre-wrap;overflow-wrap:anywhere}button:disabled{cursor:not-allowed}\n".into()));
    assets.sort_by(|left, right| left.path.cmp(&right.path));
    let records = assets.iter().map(|asset|json!({"path":asset.path,"sha256":sha256(&asset.bytes),"size":asset.bytes.len()})).collect::<Vec<_>>();
    let manifest = json!({"schema":"lean4-prod/workspace-browser-assets/1","profile":"prism.workspace-view/1","kind":"component",
        "required_csp":{"connect-src":["'self'"],"script-src":["'self'","blob:","'wasm-unsafe-eval'"],"style-src":["'self'"],"base-uri":["'none'"],"form-action":["'none'"]},
        "model_id":view.model_id,"view_model_id":view.view_model_id,"source_snapshot_sha256":view.source_snapshot_sha256,
        "sdk_inventory_sha256":binding.sdk_inventory_sha256,"guests":guests,"files":records,
        "labels":{"path":"labels.json","root":view.labels_root,"proof_sha256":view.labels_proof_sha256,"sha256":view.labels_sha256},
        "mount":{"module":"workspace.mjs","export":"mount","arguments":["root","store","headName"],"returns":["dispatch","close"],"requires":"enrolled-possessed-identity"}});
    let manifest = serde_json::to_vec(&manifest).map_err(|_| WorkspaceViewError::ModelBinding)?;
    Ok(GeneratedWorkspaceViewV1 {
        browser_assets: assets,
        view_manifest: PackageFile {
            path: "workspace-assets.json".into(),
            bytes: manifest,
        },
    })
}
