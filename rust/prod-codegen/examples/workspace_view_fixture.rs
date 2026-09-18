//! Test-only JSON/file driver for exact compiler API integration fixtures.
//! Production callers obtain these bindings from the verified installed SDK.
use prod_codegen::{
    generate_workspace_view_v1, WorkspaceBrowserBinding, WorkspaceGuest, WorkspaceGuestRole,
    WorkspaceSdkAsset, WorkspaceViewV1,
};
use serde_json::Value;
use std::{error::Error, fs, path::Path};

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing fixture string {key}").into())
}
fn number(value: &Value, key: &str) -> Result<u32, Box<dyn Error>> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| format!("missing fixture number {key}").into())
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let [input, output] = args.as_slice() else {
        return Err("expected FIXTURE_JSON ABSENT_OUTPUT".into());
    };
    let document: Value = serde_json::from_slice(&fs::read(input)?)?;
    let model = &document["view"];
    let view = WorkspaceViewV1 {
        model_id: text(model, "model_id")?.into(),
        view_model_id: text(model, "view_model_id")?.into(),
        source_snapshot_sha256: text(model, "source_snapshot_sha256")?.into(),
        labels_root: text(model, "labels_root")?.into(),
        labels_proof_sha256: text(model, "labels_proof_sha256")?.into(),
        labels_sha256: text(model, "labels_sha256")?.into(),
        labels: fs::read(text(model, "file")?)?,
    };
    let sdk_assets = document["sdk"]
        .as_array()
        .ok_or("fixture SDK array")?
        .iter()
        .map(|asset| {
            Ok(WorkspaceSdkAsset {
                path: text(asset, "path")?.into(),
                sha256: text(asset, "sha256")?.into(),
                bytes: fs::read(text(asset, "file")?)?,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let guests = document["guests"]
        .as_array()
        .ok_or("fixture guests array")?
        .iter()
        .map(|guest| {
            let role = match text(guest, "role")? {
                "view" => WorkspaceGuestRole::View,
                "command" => WorkspaceGuestRole::Command,
                "query" => WorkspaceGuestRole::Query,
                "journal" => WorkspaceGuestRole::Journal,
                _ => return Err("fixture guest role".into()),
            };
            Ok(WorkspaceGuest {
                role,
                root: text(guest, "root")?.into(),
                source_snapshot_sha256: text(guest, "source_snapshot_sha256")?.into(),
                ir_sha256: text(guest, "ir_sha256")?.into(),
                proof_sha256: text(guest, "proof_sha256")?.into(),
                wasm_sha256: text(guest, "wasm_sha256")?.into(),
                wasm: fs::read(text(guest, "file")?)?,
                request_maximum: number(guest, "request_maximum")?,
                response_maximum: number(guest, "response_maximum")?,
                maximum_pages: number(guest, "maximum_pages")?,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let binding = WorkspaceBrowserBinding {
        sdk_inventory_sha256: text(&document, "sdk_inventory_sha256")?.into(),
        sdk_assets,
        guests,
    };
    let result = generate_workspace_view_v1(&view, &binding)?;
    let root = Path::new(output);
    fs::create_dir(root)?;
    for file in result
        .browser_assets
        .into_iter()
        .chain([result.view_manifest])
    {
        let path = root.join(file.path);
        fs::create_dir_all(path.parent().ok_or("fixture parent")?)?;
        fs::write(path, file.bytes)?;
    }
    println!(
        "Generated bound component fixture assets; not source-proof or application acceptance"
    );
    Ok(())
}
