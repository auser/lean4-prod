//! Generator contract fixtures, not generated application acceptance.
use prod_codegen::{
    generate_workspace_view_v1, WorkspaceBrowserBinding, WorkspaceGuest, WorkspaceGuestRole,
    WorkspaceSdkAsset, WorkspaceViewError, WorkspaceViewV1,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn leb(mut value: u32) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        result.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
    result
}
fn section(module: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    module.push(tag);
    module.extend(leb(bytes.len() as u32));
    module.extend(bytes);
}
// Minimal ABI fixture: no application semantics, used solely to reject malformed
// memory/ABI/binding inputs. Real model integration uses four compiler-built guests.
fn abi_fixture(pages: u32) -> Vec<u8> {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    section(
        &mut bytes,
        1,
        &[2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 1, 0x7e],
    );
    section(&mut bytes, 3, &[2, 0, 1]);
    let mut memory = vec![1, 1, 1];
    memory.extend(leb(pages));
    section(&mut bytes, 5, &memory);
    let mut exports = vec![3];
    for (name, kind, index) in [("memory", 2, 0), ("holo_alloc", 0, 0), ("holo_run", 0, 1)] {
        exports.push(name.len() as u8);
        exports.extend(name.bytes());
        exports.extend([kind, index]);
    }
    section(&mut bytes, 7, &exports);
    section(
        &mut bytes,
        10,
        &[2, 4, 0, 0x20, 0, 0x0b, 4, 0, 0x42, 0, 0x0b],
    );
    bytes
}
fn fixture() -> (WorkspaceViewV1, WorkspaceBrowserBinding) {
    let mut labels = serde_json::Map::new();
    for key in "action action0 action1 action2 action3 action4 asOf author body close closed conflict contributor event inputError members message messages next none offset owner pending principal reader ready refresh rejected replay result role select submit title total unavailable unknown workspace".split(' ') {
        labels.insert(key.into(), json!(key));
    }
    labels.insert("spec".into(), json!("prismpm/workspace-view-labels/1"));
    labels.insert("title".into(), json!("<img src=x onerror=bad()> ☃"));
    let labels = serde_json::to_vec(&labels).unwrap();
    let view = WorkspaceViewV1 {
        model_id: "11".repeat(32),
        view_model_id: "22".repeat(32),
        source_snapshot_sha256: "33".repeat(32),
        labels_root: "Fixture.workspaceLabelsBytes".into(),
        labels_proof_sha256: "44".repeat(32),
        labels_sha256: sha(&labels),
        labels,
    };
    let mut assets = Vec::new();
    for (name, dependencies) in [
        ("identity", vec![]),
        ("view-error", vec![]),
        ("store", vec!["identity"]),
        ("journal", vec!["identity"]),
        ("commands", vec!["identity", "journal"]),
        ("queries", vec!["identity", "journal"]),
        ("view-dom", vec!["identity", "view-error"]),
        (
            "view-host",
            vec!["identity", "commands", "queries", "view-dom", "view-error"],
        ),
    ] {
        let imports = dependencies
            .iter()
            .map(|name| format!("import {{marker}} from './{name}.mjs';\n"))
            .collect::<String>();
        let bytes =
            format!("// {name} fixture, never a product SDK\n{imports}export const marker = 0;\n")
                .into_bytes();
        assets.push(WorkspaceSdkAsset {
            path: format!("sdk/browser/{name}.mjs"),
            sha256: sha(&bytes),
            bytes,
        });
    }
    assets.sort_by(|left, right| left.path.cmp(&right.path));
    let sdk_inventory_sha256 = inventory(&assets);
    let mut guests = Vec::new();
    for (role, module, entry, request, response, pages) in [
        (
            WorkspaceGuestRole::View,
            "View.Workspace.V1.Interaction",
            "workspaceInteractionBytes",
            133728,
            71055,
            128,
        ),
        (
            WorkspaceGuestRole::Command,
            "Browser.V1.WorkspaceCommand",
            "workspaceCommandBytes",
            139873,
            74243,
            64,
        ),
        (
            WorkspaceGuestRole::Query,
            "Browser.V1.WorkspaceQuery",
            "workspaceQueryBytes",
            1166279,
            66803,
            512,
        ),
        (
            WorkspaceGuestRole::Journal,
            "Browser.V1.WorkspaceJournal",
            "workspaceJournalBytes",
            1235980,
            1166008,
            640,
        ),
    ] {
        let wasm = abi_fixture(pages);
        guests.push(WorkspaceGuest {
            role,
            root: format!("PrismPM.Foundation.{module}.{entry}"),
            source_snapshot_sha256: view.source_snapshot_sha256.clone(),
            ir_sha256: "55".repeat(32),
            proof_sha256: "66".repeat(32),
            wasm_sha256: sha(&wasm),
            wasm,
            request_maximum: request,
            response_maximum: response,
            maximum_pages: pages,
        });
    }
    (
        view,
        WorkspaceBrowserBinding {
            sdk_inventory_sha256,
            sdk_assets: assets,
            guests,
        },
    )
}
fn inventory(assets: &[WorkspaceSdkAsset]) -> String {
    sha(&serde_json::to_vec(
        &assets
            .iter()
            .map(|asset| json!({"path":asset.path,"sha256":asset.sha256,"size":asset.bytes.len()}))
            .collect::<Vec<_>>(),
    )
    .unwrap())
}
fn refresh_labels(view: &mut WorkspaceViewV1, value: Value) {
    view.labels = serde_json::to_vec(&value).unwrap();
    view.labels_sha256 = sha(&view.labels);
}

#[test]
fn deterministic_closed_asset_tree_binds_every_byte_and_explicit_prerequisite() {
    let (view, binding) = fixture();
    let result = generate_workspace_view_v1(&view, &binding).unwrap();
    assert_eq!(result, generate_workspace_view_v1(&view, &binding).unwrap());
    assert_eq!(result.browser_assets.len(), 15);
    assert!(result
        .browser_assets
        .windows(2)
        .all(|pair| pair[0].path < pair[1].path));
    let manifest: Value = serde_json::from_slice(&result.view_manifest.bytes).unwrap();
    assert_eq!(manifest["mount"]["requires"], "enrolled-possessed-identity");
    for (asset, record) in result
        .browser_assets
        .iter()
        .zip(manifest["files"].as_array().unwrap())
    {
        assert_eq!(record["path"], asset.path);
        assert_eq!(record["sha256"], sha(&asset.bytes));
        assert_eq!(record["size"], asset.bytes.len());
    }
    assert!(!result
        .browser_assets
        .iter()
        .any(|asset| asset.path == "index.html"));
    assert_eq!(manifest["kind"], "component");
    assert_eq!(manifest["required_csp"]["form-action"], json!(["'none'"]));
    let loader = String::from_utf8(
        result
            .browser_assets
            .iter()
            .find(|asset| asset.path == "workspace.mjs")
            .unwrap()
            .bytes
            .clone(),
    )
    .unwrap();
    assert_eq!(loader.matches("export ").count(), 1);
    assert!(loader.contains("export async function mount(root, store, headName)"));
    assert!(!loader.contains("createIdentity("));
    assert!(!loader.contains("saveIdentity("));
    assert!(loader.contains("URL.createObjectURL"));
    assert!(loader.contains("URL.revokeObjectURL"));
    assert!(loader.contains("redirect: 'error'"));
}

#[test]
fn invalid_model_digests_and_labels_are_rejected_without_escaping_payloads() {
    let (view, binding) = fixture();
    for field in 0..5 {
        let mut bad = view.clone();
        *[
            &mut bad.model_id,
            &mut bad.view_model_id,
            &mut bad.source_snapshot_sha256,
            &mut bad.labels_proof_sha256,
            &mut bad.labels_sha256,
        ][field] = "AA".repeat(32);
        assert_eq!(
            generate_workspace_view_v1(&bad, &binding),
            Err(WorkspaceViewError::ModelBinding)
        );
    }
    for root in ["", "bad()", "A..B", "A.1bad", "A.B\n"] {
        let mut bad = view.clone();
        bad.labels_root = root.into();
        assert_eq!(
            generate_workspace_view_v1(&bad, &binding),
            Err(WorkspaceViewError::ModelBinding)
        );
    }
    for bytes in [
        b"{}".to_vec(),
        vec![255],
        vec![b' '; 8193],
        [view.labels.as_slice(), b"\n"].concat(),
        [&b"{\"action\":\"duplicate\","[..], &view.labels[1..]].concat(),
    ] {
        let mut bad = view.clone();
        bad.labels = bytes;
        bad.labels_sha256 = sha(&bad.labels);
        assert_eq!(
            generate_workspace_view_v1(&bad, &binding),
            Err(WorkspaceViewError::Labels)
        );
    }
    for value in [json!(""), json!("é".repeat(129)), json!(false), json!({})] {
        let mut bad = view.clone();
        let mut labels: Value = serde_json::from_slice(&bad.labels).unwrap();
        labels["title"] = value;
        refresh_labels(&mut bad, labels);
        assert_eq!(
            generate_workspace_view_v1(&bad, &binding),
            Err(WorkspaceViewError::Labels)
        );
    }
    let mut exact = view;
    let mut labels: Value = serde_json::from_slice(&exact.labels).unwrap();
    labels["title"] = json!("é".repeat(128));
    refresh_labels(&mut exact, labels);
    assert!(generate_workspace_view_v1(&exact, &binding).is_ok());
}

#[test]
fn sdk_inventory_is_exact_closed_sorted_and_digest_bound() {
    let (view, binding) = fixture();
    let mut bad = binding.clone();
    bad.sdk_assets.pop();
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::SdkInventory)
    );
    for path in [
        "identity.mjs",
        "sdk/browser/extra.mjs",
        "sdk/browser/../identity.mjs",
        "https://x.test/identity.mjs",
    ] {
        let mut bad = binding.clone();
        bad.sdk_assets[0].path = path.into();
        assert_eq!(
            generate_workspace_view_v1(&view, &bad),
            Err(WorkspaceViewError::SdkAsset)
        );
    }
    let mut bad = binding.clone();
    bad.sdk_assets.reverse();
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::SdkAsset)
    );
    let mut bad = binding.clone();
    bad.sdk_assets[0].bytes.push(b' ');
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::SdkAsset)
    );
    let mut bad = binding.clone();
    bad.sdk_inventory_sha256 = "77".repeat(32);
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::SdkInventory)
    );
    let mut bad = binding.clone();
    bad.sdk_assets[0].bytes = b"import {untrusted} from './extra.mjs';\n".to_vec();
    bad.sdk_assets[0].sha256 = sha(&bad.sdk_assets[0].bytes);
    bad.sdk_inventory_sha256 = inventory(&bad.sdk_assets);
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::SdkDependency)
    );
}

#[test]
fn sdk_exports_preserve_closed_local_declarations_and_reject_reexport_spellings() {
    let (view, binding) = fixture();
    let with_source = |source: &str| {
        let mut changed = binding.clone();
        let asset = changed
            .sdk_assets
            .iter_mut()
            .find(|asset| asset.path.ends_with("/identity.mjs"))
            .unwrap();
        asset.bytes = source.as_bytes().to_vec();
        asset.sha256 = sha(&asset.bytes);
        changed.sdk_inventory_sha256 = inventory(&changed.sdk_assets);
        changed
    };
    for source in [
        "export const marker = 0;",
        "export class Marker {}",
        "export class Marker extends Error {}",
        "export function marker() {}",
        "export async function marker() {}",
        "const marker = 0;\nexport {marker};",
        "const marker = 0;\nexport {marker, marker as other};",
    ] {
        assert!(
            generate_workspace_view_v1(&view, &with_source(source)).is_ok(),
            "{source}"
        );
    }
    for source in [
        "export{marker}from'./extra.mjs';",
        "export\n{marker} from './extra.mjs';",
        "export {marker} from './extra.mjs';",
        "export {marker}\nfrom './extra.mjs';",
        "export/* gap */{marker}from'./extra.mjs';",
        "export {marker}/* gap */from'./extra.mjs';",
        "export {marker}\n/* gap */from './extra.mjs';",
        "export {marker}from'https://example.invalid/extra.mjs';",
        "export {marker}from\"https://example.invalid/extra.mjs\";",
        "export * from './extra.mjs';",
        "export * as other from './extra.mjs';",
        "export const marker = 0;export{marker}from'./extra.mjs';",
        "const marker = 0;export{marker}from'./extra.mjs';",
        "export\u{2028}{marker}from'./extra.mjs';",
        "export const marker = import('./extra.mjs');",
    ] {
        assert_eq!(
            generate_workspace_view_v1(&view, &with_source(source)),
            Err(WorkspaceViewError::SdkDependency),
            "{source}"
        );
    }
}

#[test]
fn exact_four_guests_match_roles_source_snapshot_roots_digests_and_caps() {
    let (view, binding) = fixture();
    let mut bad = binding.clone();
    bad.guests.pop();
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::GuestSet)
    );
    let mut bad = binding.clone();
    bad.guests[0] = bad.guests[1].clone();
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::GuestSet)
    );
    for index in 0..4 {
        for field in 0..8 {
            let mut bad = binding.clone();
            let guest = &mut bad.guests[index];
            match field {
                0 => guest.source_snapshot_sha256 = "77".repeat(32),
                1 => guest.root = "Wrong.entry".into(),
                2 => guest.ir_sha256 = "invalid".into(),
                3 => guest.proof_sha256 = "invalid".into(),
                4 => guest.wasm.push(0),
                5 => guest.request_maximum += 1,
                6 => guest.response_maximum += 1,
                _ => guest.maximum_pages += 1,
            }
            assert_eq!(
                generate_workspace_view_v1(&view, &bad),
                Err(WorkspaceViewError::GuestBinding)
            );
        }
    }
}

#[test]
fn real_wasm_validation_rejects_malformed_modules_abi_and_memory() {
    let (view, binding) = fixture();
    for (bytes, error) in [
        (vec![0; 8], WorkspaceViewError::GuestModule),
        (b"\0asm\x01\0\0\0".to_vec(), WorkspaceViewError::GuestMemory),
        (abi_fixture(127), WorkspaceViewError::GuestMemory),
    ] {
        let mut bad = binding.clone();
        bad.guests[0].wasm = bytes;
        bad.guests[0].wasm_sha256 = sha(&bad.guests[0].wasm);
        assert_eq!(generate_workspace_view_v1(&view, &bad), Err(error));
    }
    let mut bad = binding;
    let bytes = &mut bad.guests[0].wasm;
    let offset = bytes
        .windows(8)
        .position(|part| part == b"holo_run")
        .unwrap();
    bytes[offset] = b'x';
    bad.guests[0].wasm_sha256 = sha(bytes);
    assert_eq!(
        generate_workspace_view_v1(&view, &bad),
        Err(WorkspaceViewError::GuestAbi)
    );
}

// Structural Wasm mutation helpers only; none of these test modules implement
// application behavior or claim a kernel proof.
fn replace_section(module: &[u8], selected: u8, replacement: &[u8]) -> Vec<u8> {
    let mut result = module[..8].to_vec();
    let mut cursor = 8;
    let mut inserted = false;
    while cursor < module.len() {
        let start = cursor;
        let tag = module[cursor];
        cursor += 1;
        let mut length = 0_usize;
        let mut shift = 0;
        loop {
            let byte = module[cursor];
            cursor += 1;
            length |= usize::from(byte & 127) << shift;
            if byte & 128 == 0 {
                break;
            }
            shift += 7;
        }
        let end = cursor + length;
        if !inserted && tag >= selected {
            section(&mut result, selected, replacement);
            inserted = true;
        }
        if tag != selected {
            result.extend_from_slice(&module[start..end]);
        }
        cursor = end;
    }
    if !inserted {
        section(&mut result, selected, replacement);
    }
    result
}

fn rejected_wasm(bytes: Vec<u8>, error: WorkspaceViewError, otherwise_valid: bool) {
    if otherwise_valid {
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("valid Wasm independently isolates the profile rejection");
    }
    let (view, mut binding) = fixture();
    binding.guests[0].wasm_sha256 = sha(&bytes);
    binding.guests[0].wasm = bytes;
    assert_eq!(generate_workspace_view_v1(&view, &binding), Err(error));
}

#[test]
fn profile_rejects_otherwise_valid_imports_and_start_sections() {
    let module = abi_fixture(128);
    // One function import m.f with type0. Existing exports remain valid Wasm
    // (the profile rejects imports before inspecting their ABI identities).
    let imported = replace_section(&module, 2, &[1, 1, b'm', 1, b'f', 0, 0]);
    rejected_wasm(imported, WorkspaceViewError::GuestAbi, true);

    let with_void = replace_section(
        &module,
        1,
        &[
            3, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 1, 0x7e, 0x60, 0, 0,
        ],
    );
    let with_function = replace_section(&with_void, 3, &[3, 0, 1, 2]);
    let with_code = replace_section(
        &with_function,
        10,
        &[3, 4, 0, 0x20, 0, 0x0b, 4, 0, 0x42, 0, 0x0b, 2, 0, 0x0b],
    );
    rejected_wasm(
        replace_section(&with_code, 8, &[2]),
        WorkspaceViewError::GuestAbi,
        true,
    );
}

#[test]
fn profile_rejects_malformed_types_and_valid_but_wrong_function_signatures() {
    let module = abi_fixture(128);
    rejected_wasm(
        replace_section(&module, 1, &[1, 0x61, 0, 0]),
        WorkspaceViewError::GuestModule,
        false,
    );
    // Point holo_run at the allocator: a valid Wasm function export with the
    // wrong (i32)->i32 signature, not a parser/header/name failure.
    let mut exports = vec![3];
    for (name, kind, index) in [("memory", 2, 0), ("holo_alloc", 0, 0), ("holo_run", 0, 0)] {
        exports.push(name.len() as u8);
        exports.extend(name.bytes());
        exports.extend([kind, index]);
    }
    rejected_wasm(
        replace_section(&module, 7, &exports),
        WorkspaceViewError::GuestAbi,
        true,
    );
    // Independently test the allocator's signature while preserving valid Wasm.
    let mut exports = vec![3];
    for (name, kind, index) in [("memory", 2, 0), ("holo_alloc", 0, 1), ("holo_run", 0, 1)] {
        exports.push(name.len() as u8);
        exports.extend(name.bytes());
        exports.extend([kind, index]);
    }
    rejected_wasm(
        replace_section(&module, 7, &exports),
        WorkspaceViewError::GuestAbi,
        true,
    );
}

#[test]
fn profile_rejects_otherwise_valid_shared_memory64_multiple_and_unbounded_memories() {
    let module = abi_fixture(128);
    for flags in [3, 5] {
        // maximum+shared; maximum+memory64
        let mut memory = vec![1, flags, 1];
        memory.extend(leb(128));
        rejected_wasm(
            replace_section(&module, 5, &memory),
            WorkspaceViewError::GuestMemory,
            true,
        );
    }
    let mut memories = vec![2, 1, 1];
    memories.extend(leb(128));
    memories.extend([1, 1]);
    memories.extend(leb(128));
    rejected_wasm(
        replace_section(&module, 5, &memories),
        WorkspaceViewError::GuestMemory,
        true,
    );
    rejected_wasm(
        replace_section(&module, 5, &[1, 0, 1]),
        WorkspaceViewError::GuestMemory,
        true,
    );
}

#[test]
fn profile_rejects_otherwise_valid_internal_tables_outside_linear_memory_caps() {
    let module = abi_fixture(128);
    let mut huge = vec![1, 0x70, 1];
    huge.extend(leb(1_000_000_000));
    huge.extend(leb(1_000_000_000));
    for table in [
        huge,
        vec![1, 0x70, 0, 1],                   // unbounded funcref
        vec![2, 0x70, 1, 1, 1, 0x70, 1, 1, 1], // multiple bounded tables
        vec![1, 0x70, 1, 1, 1],                // even a small bounded table is outside this profile
        vec![1, 0x6f, 1, 1, 1],                // externref
        vec![1, 0x70, 5, 1, 1],                // table64
    ] {
        rejected_wasm(
            replace_section(&module, 4, &table),
            WorkspaceViewError::GuestMemory,
            true,
        );
    }
}

#[test]
fn duplicate_roles_and_duplicate_content_hashes_are_distinct_closed_failures() {
    let (view, binding) = fixture();
    let mut role = binding.clone();
    role.guests[1] = role.guests[0].clone();
    assert_eq!(
        generate_workspace_view_v1(&view, &role),
        Err(WorkspaceViewError::GuestSet)
    );
    let mut hash = binding.clone();
    hash.guests[1].wasm = hash.guests[0].wasm.clone();
    hash.guests[1].wasm_sha256 = hash.guests[0].wasm_sha256.clone();
    assert_ne!(hash.guests[0].role, hash.guests[1].role);
    assert_eq!(
        generate_workspace_view_v1(&view, &hash),
        Err(WorkspaceViewError::GuestSet)
    );
    let mut sdk = binding;
    let identity = sdk
        .sdk_assets
        .iter()
        .position(|asset| asset.path.ends_with("/identity.mjs"))
        .unwrap();
    let error = sdk
        .sdk_assets
        .iter()
        .position(|asset| asset.path.ends_with("/view-error.mjs"))
        .unwrap();
    sdk.sdk_assets[error].bytes = sdk.sdk_assets[identity].bytes.clone();
    sdk.sdk_assets[error].sha256 = sdk.sdk_assets[identity].sha256.clone();
    sdk.sdk_inventory_sha256 = inventory(&sdk.sdk_assets);
    assert_eq!(
        generate_workspace_view_v1(&view, &sdk),
        Err(WorkspaceViewError::SdkAsset)
    );
}

#[test]
fn labels_reject_utf8_bom_but_preserve_ufe_ff_inside_display_values() {
    let (view, binding) = fixture();
    let mut prefixed = view.clone();
    prefixed.labels.splice(..0, [0xef, 0xbb, 0xbf]);
    prefixed.labels_sha256 = sha(&prefixed.labels);
    assert_eq!(
        generate_workspace_view_v1(&prefixed, &binding),
        Err(WorkspaceViewError::Labels)
    );
    let mut text = view;
    let mut labels: Value = serde_json::from_slice(&text.labels).unwrap();
    labels["title"] = json!("\u{feff}literal display text");
    refresh_labels(&mut text, labels);
    let generated = generate_workspace_view_v1(&text, &binding).unwrap();
    let artifact = generated
        .browser_assets
        .iter()
        .find(|asset| asset.path == "labels.json")
        .unwrap();
    assert_eq!(artifact.bytes, text.labels);
}
