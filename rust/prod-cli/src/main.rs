//! prod-cli: Command-line tool for parsing Lean 4 IR and generating Rust
//!
//! Usage:
//!   prod parse module.ir
//!   prod gen module.ir [--output generated.rs]
//!   prod validate module.ir

mod roots;

use clap::{Parser, Subcommand, ValueEnum};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "prod")]
#[command(about = "Lean 4 → prod IR parser and Rust code generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse an IR file and print its AST
    Parse {
        /// Path to the IR file
        path: String,
    },
    /// Generate Rust code from an IR file (prints to stdout unless --output is given)
    Gen {
        /// Path to the IR file
        path: String,
        /// Output path for generated Rust code
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Generate a complete deterministic publishable Cargo library tree.
    Cargo {
        /// Path to the exported IR file.
        path: String,
        /// New output directory; an existing path is rejected.
        #[arg(short, long)]
        output: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        homepage: String,
        #[arg(long)]
        readme: String,
        #[arg(long)]
        license_mit: String,
        #[arg(long)]
        license_apache: String,
        /// Exact registry dependency as NAME=VERSION=CRATE_SHA256; repeat in
        /// bytewise package-name order.
        #[arg(long = "dependency")]
        dependencies: Vec<String>,
    },
    /// Generate an import-free Hologram Core-Wasm v1 guest package.
    CoreWasm {
        /// Path to the exported IR file.
        path: String,
        /// New output directory; an existing path is rejected.
        #[arg(short, long)]
        output: String,
        /// Exported IR definition with `(List UInt8) -> List UInt8` type.
        #[arg(long)]
        entry: String,
        /// Hologram manifest entry/export name.
        #[arg(long, default_value = "holo_run")]
        export_name: String,
        #[arg(long, default_value_t = 65_536)]
        input_allocation_cap: u32,
        #[arg(long, default_value_t = 65_536)]
        output_allocation_cap: u32,
        #[arg(long, default_value_t = 4)]
        maximum_pages: u32,
        #[arg(long, default_value = "lean4-prod-core-wasm")]
        crate_name: String,
    },
    /// Project a closed evaluated Foundation.View.V1 JSON value to Hologram
    /// and browser assets plus the registry-bound wasm adapter.
    View {
        /// Closed evaluated View/binding JSON input.
        input: String,
        /// New output directory; an existing path is rejected.
        #[arg(short, long)]
        output: String,
    },
    /// Generate a C header and matching Rust `extern "C"` wrappers.
    Header {
        /// Path to the IR file
        path: String,
        /// Output path for the generated C header
        #[arg(short, long)]
        output: String,
        /// Output path for the matching Rust wrapper source
        #[arg(long)]
        rust_output: String,
    },
    /// Generate C, Rust, Python, TypeScript, Kotlin, and WebAssembly SDK artifacts.
    Sdks {
        /// Path to the exported IR file
        path: String,
        /// Root directory for generated SDK bundles
        #[arg(short, long, default_value = "output")]
        output: String,
        /// Bundle name and generated artifact stem
        #[arg(long, default_value = "lean4-prod")]
        stem: String,
        /// Native library name used by the Rust/Kotlin SDKs
        #[arg(long, default_value = "lean4_prod")]
        library_name: String,
    },
    /// Generate SDK artifacts for one language only.
    Sdk {
        /// Path to the exported IR file
        path: String,
        /// Language artifact to generate
        #[arg(value_enum, short, long)]
        language: SdkLanguage,
        /// Root directory for generated SDK artifacts
        #[arg(short, long, default_value = "output")]
        output: String,
        /// Bundle name and generated artifact stem
        #[arg(long, default_value = "lean4-prod")]
        stem: String,
        /// Native library name used by the Rust/Kotlin SDKs
        #[arg(long, default_value = "lean4_prod")]
        library_name: String,
    },
    /// Validate an IR file (check for unsupported constructs)
    Validate {
        /// Path to the IR file
        path: String,
    },
    /// Analyze proof roots exported by Lean.
    Roots {
        #[command(subcommand)]
        command: RootCommands,
    },
    /// Render the published Lean-for-production subset contract.
    Subset {
        /// Path to subset.json, written by prod-export
        path: String,
        /// Output path for the rendered markdown
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Clone, ValueEnum)]
enum SdkLanguage {
    C,
    Rust,
    Python,
    Typescript,
    Kotlin,
    Wasm,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewInput {
    view: ViewValue,
    binding: ViewBinding,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewValue {
    title: String,
    heading: String,
    left_label: String,
    right_label: String,
    operation_label: String,
    submit_label: String,
    input_error: String,
    division_by_zero_error: String,
    overflow_error: String,
    operations: Vec<ViewOperationInput>,
    initial_operation: u8,
    model_id: String,
    view_model_id: String,
    generated_core_sha256: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewOperationInput {
    label: String,
    request_name: String,
    rust_variant: String,
    discriminant: u8,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewBinding {
    package_name: String,
    package_version: String,
    core_crate_name: String,
    core_crate_version: String,
    core_operation_type: String,
    core_error_type: String,
    core_function: String,
}

/// The Lean half of the subset contract, written by `prod-export`
/// (`Prod.subsetJson`, `lean/Prod/Emit.lean`).
#[derive(Debug, serde::Deserialize)]
struct SubsetFile {
    operators: Vec<String>,
    deciders: Vec<String>,
    types: Vec<String>,
}

/// Render the published subset contract: the Lean half (`subset.json`,
/// what `Lower.lean` accepts) merged with the Rust half (`prod_codegen::
/// REJECTIONS`, what `Error` reports). Generated, never hand-written: a
/// hand-maintained contract drifts from the implementation, and a drifted
/// contract is worse than none.
fn render_subset(subset: &SubsetFile) -> String {
    let mut out = String::from(
        "# Lean-for-production: the supported subset\n\n\
         <!-- GENERATED by `just subset`. Do not edit by hand. -->\n\n\
         This is the fragment of Lean 4 that `prod-export` lowers and\n\
         `prod-codegen` renders. Anything outside it is rejected with the\n\
         named error rather than silently mis-compiled.\n\n\
         ## Types\n\n",
    );
    for t in &subset.types {
        out.push_str(&format!("- `{}`\n", t));
    }
    out.push_str(
        "\n**Erased invariants.** A Lean structure may carry `Prop` fields\n\
         expressing invariants over the computational fields — for example\n\
         `UorAtlas.Instance.valid : q ≥ 1 ∧ T ≥ 1 ∧ O ≥ 1`. `Prop` fields are\n\
         erased on export, correctly: they are proofs, not data, and carry no\n\
         runtime representation. The consequence is that the generated Rust\n\
         struct does **not** enforce the invariant — `Instance { q: 0, T: 0,\n\
         O: 0 }` is constructible in Rust where the Lean type forbids it.\n\
         Callers that need the invariant must re-check it in Rust; the\n\
         generated struct is a plain data carrier, not a refinement type.\n",
    );
    out.push_str("\n## Operators\n\n");
    for op in &subset.operators {
        out.push_str(&format!("- `{}`\n", op));
    }
    out.push_str("\n## Decidable guards\n\n");
    for d in &subset.deciders {
        out.push_str(&format!("- `{}`\n", d));
    }
    out.push_str(
        "\n## Rejections\n\nEverything else fails, precisely:\n\n| Error | Cause |\n|---|---|\n",
    );
    for (variant, cause) in prod_codegen::REJECTIONS {
        out.push_str(&format!("| `{}` | {} |\n", variant, cause));
    }
    out
}

#[derive(Subcommand)]
enum RootCommands {
    /// Check dependency acyclicity and root dependency coverage.
    Check {
        path: String,
        /// Include auto-generated roots (default: hand-written roots only)
        #[arg(long)]
        all: bool,
    },
    /// Print roots on the Pareto front of proof size, kernel depth, and check time.
    Pareto {
        path: String,
        /// Include auto-generated roots (default: hand-written roots only)
        #[arg(long)]
        all: bool,
    },
    /// Generate bridges between roots sharing kernel dependencies.
    Connect {
        path: String,
        root1: Option<String>,
        root2: Option<String>,
        /// Include auto-generated roots (default: hand-written roots only)
        #[arg(long)]
        all: bool,
    },
}

/// Collect every unresolved callee name reachable from an expression.
///
/// The traversal is `prod_ir::Expr::children()`, not a copy of it: this used
/// to hand-match every recursive shape and had already drifted (it never
/// looked inside `Expr::Shr`, added later), so `prod validate` could report a
/// clean bill of health for IR containing an extern call.
fn collect_externs(expr: &prod_ir::Expr, out: &mut Vec<String>) {
    if let prod_ir::Expr::Extern(name, _) = expr {
        out.push(name.clone());
    }
    for child in expr.children() {
        collect_externs(child, out);
    }
}

fn wasm_package_name(stem: &str) -> String {
    let mut name = stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.starts_with('-') {
        name.remove(0);
    }
    while name.ends_with('-') {
        name.pop();
    }
    if name.is_empty() {
        name.push_str("lean4-prod-sdk");
    }
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        name.insert_str(0, "lean-");
    }
    name
}

fn copy_directory_contents(source: &Path, destination: &Path) {
    fs::create_dir_all(destination)
        .unwrap_or_else(|e| panic!("Failed to create {}: {e}", destination.display()));
    for entry in
        fs::read_dir(source).unwrap_or_else(|e| panic!("Failed to read {}: {e}", source.display()))
    {
        let entry = entry.unwrap_or_else(|e| panic!("Failed to inspect package artifact: {e}"));
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .unwrap_or_else(|e| panic!("Failed to inspect {}: {e}", source_path.display()))
            .is_dir()
        {
            copy_directory_contents(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).unwrap_or_else(|e| {
                panic!(
                    "Failed to copy wasm-pack artifact {} to {}: {e}",
                    source_path.display(),
                    destination_path.display()
                )
            });
        }
    }
}

fn publish_generated_package(package: prod_codegen::GeneratedPackage, output: &Path) {
    if output.exists() {
        panic!(
            "Refusing to replace existing generated package {}",
            output.display()
        );
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .unwrap_or_else(|e| panic!("Failed to create {}: {e}", parent.display()));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_nanos();
    let staging = parent.join(format!(
        ".lean4-prod-package-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&staging)
        .unwrap_or_else(|e| panic!("Failed to create {}: {e}", staging.display()));
    for item in package.files {
        let relative = Path::new(&item.path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            let _ = fs::remove_dir_all(&staging);
            panic!("Generated package contains unsafe path {}", item.path);
        }
        let destination = staging.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("Failed to create {}: {e}", parent.display()));
        }
        fs::write(&destination, item.bytes)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", destination.display()));
    }
    fs::rename(&staging, output).unwrap_or_else(|e| {
        let _ = fs::remove_dir_all(&staging);
        panic!(
            "Failed to publish generated package {} atomically: {e}",
            output.display()
        )
    });
}

fn build_wasm_sdk(source: &str, output: &Path, stem: &str) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_nanos();
    let staging = std::env::temp_dir().join(format!("lean4-prod-wasm-sdk-{nonce}"));
    let src_dir = staging.join("src");
    fs::create_dir_all(&src_dir)
        .unwrap_or_else(|e| panic!("Failed to create wasm SDK staging directory: {e}"));
    let package_name = wasm_package_name(stem);
    fs::write(
        staging.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nwasm-bindgen = \"0.2.122\"\n"
        ),
    )
    .unwrap_or_else(|e| panic!("Failed to write wasm SDK manifest: {e}"));
    fs::write(src_dir.join("lib.rs"), source)
        .unwrap_or_else(|e| panic!("Failed to write wasm SDK source: {e}"));

    let status = Command::new("wasm-pack")
        .current_dir(&staging)
        .args(["build", "--target", "web", "--release", "--out-dir", "pkg"])
        .status()
        .unwrap_or_else(|e| panic!("Failed to run wasm-pack (install wasm-pack first): {e}"));
    if !status.success() {
        panic!("wasm-pack failed with status {status}");
    }

    let package_dir = staging.join("pkg");
    let mut has_wasm = false;
    let mut has_javascript = false;
    let mut has_typescript = false;
    for entry in fs::read_dir(&package_dir)
        .unwrap_or_else(|e| panic!("Failed to read wasm-pack output: {e}"))
    {
        let entry = entry.unwrap_or_else(|e| panic!("Failed to inspect wasm-pack output: {e}"));
        let file_name = entry.file_name();
        let file_name_text = file_name.to_string_lossy();
        has_wasm |= file_name_text.ends_with(".wasm");
        has_javascript |= file_name_text.ends_with(".js");
        has_typescript |= file_name_text.ends_with(".d.ts");
    }
    if !(has_wasm && has_javascript && has_typescript) {
        let _ = fs::remove_dir_all(staging);
        panic!(
            "wasm-pack did not produce the required .wasm, .js, and .d.ts artifacts in {}",
            output.display()
        );
    }
    if output.exists() {
        fs::remove_dir_all(output)
            .unwrap_or_else(|e| panic!("Failed to replace {}: {e}", output.display()));
    }
    copy_directory_contents(&package_dir, output);
    let _ = fs::remove_dir_all(staging);
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Parse { path } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(&content)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            println!("Module: {}", module.name);
            for def in &module.definitions {
                println!("  Def: {} -> {:?}", def.name, def.ret);
                println!("    Params: {:?}", def.params);
                println!("    Body: {:?}", def.body);
            }
        }
        Commands::Gen { path, output } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(&content)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));

            let body = prod_codegen::generate_module(&module)
                .unwrap_or_else(|e| panic!("Codegen error: {}", e));

            let mut out = String::from("#![allow(dead_code)]\n\n");
            out.push_str(&format!(
                "// Generated from Lean 4 module: {}\n\n",
                module.name
            ));
            out.push_str(&body);

            match output {
                Some(output) => {
                    fs::write(&output, out)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", output, e));
                    println!("Generated: {}", output);
                }
                None => print!("{}", out),
            }
        }
        Commands::Cargo {
            path,
            output,
            name,
            version,
            description,
            repository,
            homepage,
            readme,
            license_mit,
            license_apache,
            dependencies,
        } => {
            let ir = fs::read(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let text = std::str::from_utf8(&ir)
                .unwrap_or_else(|e| panic!("IR {} is not UTF-8: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(text)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            let package = prod_codegen::generate_cargo_package(
                &module,
                &prod_codegen::CargoPackageSpec {
                    name,
                    version,
                    description,
                    repository,
                    homepage,
                    readme: fs::read_to_string(&readme)
                        .unwrap_or_else(|e| panic!("Failed to read {}: {}", readme, e)),
                    license_mit: fs::read_to_string(&license_mit)
                        .unwrap_or_else(|e| panic!("Failed to read {}: {}", license_mit, e)),
                    license_apache: fs::read_to_string(&license_apache)
                        .unwrap_or_else(|e| panic!("Failed to read {}: {}", license_apache, e)),
                    input_sha256: format!("{:x}", Sha256::digest(&ir)),
                    dependencies: dependencies
                        .into_iter()
                        .map(|value| {
                            let fields = value.split('=').collect::<Vec<_>>();
                            if fields.len() != 3 {
                                panic!(
                                    "Invalid --dependency {value}; expected NAME=VERSION=CRATE_SHA256"
                                );
                            }
                            prod_codegen::CargoDependency {
                                name: fields[0].to_owned(),
                                version: fields[1].to_owned(),
                                checksum: fields[2].to_owned(),
                                default_features: false,
                                features: Vec::new(),
                            }
                        })
                        .collect(),
                },
            )
            .unwrap_or_else(|e| panic!("Cargo package generation error: {}", e));
            let root = Path::new(&output);
            publish_generated_package(package, root);
            println!("Generated Cargo package: {}", root.display());
        }
        Commands::CoreWasm {
            path,
            output,
            entry,
            export_name,
            input_allocation_cap,
            output_allocation_cap,
            maximum_pages,
            crate_name,
        } => {
            let ir = fs::read(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let text = std::str::from_utf8(&ir)
                .unwrap_or_else(|e| panic!("IR {} is not UTF-8: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(text)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            let package = prod_codegen::generate_core_wasm_package(
                &module,
                &prod_codegen::CoreWasmSpec {
                    crate_name,
                    entry,
                    export_name,
                    input_allocation_cap,
                    output_allocation_cap,
                    maximum_pages,
                    input_ir_sha256: format!("{:x}", Sha256::digest(&ir)),
                },
            )
            .unwrap_or_else(|e| panic!("Core-Wasm generation error: {}", e));
            let root = Path::new(&output);
            publish_generated_package(package, root);
            println!("Generated Core-Wasm package: {}", root.display());
        }
        Commands::View { input, output } => {
            let value: ViewInput = serde_json::from_slice(
                &fs::read(&input).unwrap_or_else(|error| panic!("Failed to read {input}: {error}")),
            )
            .unwrap_or_else(|error| panic!("Invalid closed View input {input}: {error}"));
            let view = prod_codegen::EvaluatedViewV1 {
                title: value.view.title,
                heading: value.view.heading,
                left_label: value.view.left_label,
                right_label: value.view.right_label,
                operation_label: value.view.operation_label,
                submit_label: value.view.submit_label,
                input_error: value.view.input_error,
                division_by_zero_error: value.view.division_by_zero_error,
                overflow_error: value.view.overflow_error,
                operations: value
                    .view
                    .operations
                    .into_iter()
                    .map(|operation| prod_codegen::ViewOperation {
                        label: operation.label,
                        request_name: operation.request_name,
                        rust_variant: operation.rust_variant,
                        discriminant: operation.discriminant,
                    })
                    .collect(),
                initial_operation: value.view.initial_operation,
                model_id: value.view.model_id,
                view_model_id: value.view.view_model_id,
                generated_core_sha256: value.view.generated_core_sha256,
            };
            let binding = prod_codegen::BrowserAdapterBinding {
                package_name: value.binding.package_name,
                package_version: value.binding.package_version,
                core_crate_name: value.binding.core_crate_name,
                core_crate_version: value.binding.core_crate_version,
                core_operation_type: value.binding.core_operation_type,
                core_error_type: value.binding.core_error_type,
                core_function: value.binding.core_function,
            };
            let generated = prod_codegen::generate_view_v1(&view, &binding)
                .unwrap_or_else(|error| panic!("View generation error: {error}"));
            let mut files = Vec::new();
            for item in generated.hologram_assets {
                files.push(prod_codegen::PackageFile {
                    path: format!("hologram/{}", item.path),
                    bytes: item.bytes,
                });
            }
            files.push(prod_codegen::PackageFile {
                path: format!("hologram/{}", generated.hologram_bundle.path),
                bytes: generated.hologram_bundle.bytes,
            });
            for item in generated.browser_assets {
                files.push(prod_codegen::PackageFile {
                    path: format!("browser/{}", item.path),
                    bytes: item.bytes,
                });
            }
            for item in generated.browser_adapter.files {
                files.push(prod_codegen::PackageFile {
                    path: format!("browser-adapter/{}", item.path),
                    bytes: item.bytes,
                });
            }
            files.push(generated.view_manifest);
            files.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
            publish_generated_package(prod_codegen::GeneratedPackage { files }, Path::new(&output));
            println!("Generated View projections: {output}");
        }
        Commands::Header {
            path,
            output,
            rust_output,
        } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(&content)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            let bindings = prod_codegen::generate_c_bindings(&module)
                .unwrap_or_else(|e| panic!("C ABI generation error: {}", e));
            fs::write(&output, bindings.header)
                .unwrap_or_else(|e| panic!("Failed to write {}: {}", output, e));
            fs::write(&rust_output, bindings.rust)
                .unwrap_or_else(|e| panic!("Failed to write {}: {}", rust_output, e));
            println!("Generated: {}", output);
            println!("Generated: {}", rust_output);
        }
        Commands::Sdks {
            path,
            output,
            stem,
            library_name,
        } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(&content)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            let bindings = prod_codegen::generate_sdks(&module, &library_name)
                .unwrap_or_else(|e| panic!("SDK generation error: {}", e));
            let root = Path::new(&output).join(&stem);
            let c_dir = root.join("c");
            let rust_dir = root.join("rust");
            let python_dir = root.join("python");
            let typescript_dir = root.join("typescript");
            let kotlin_dir = root.join("kotlin");
            let wasm_dir = root.join("wasm");
            for directory in [
                &c_dir,
                &rust_dir,
                &python_dir,
                &typescript_dir,
                &kotlin_dir,
                &wasm_dir,
            ] {
                fs::create_dir_all(directory)
                    .unwrap_or_else(|e| panic!("Failed to create {}: {}", directory.display(), e));
            }
            fs::write(c_dir.join(format!("{}.h", stem)), bindings.c_header)
                .unwrap_or_else(|e| panic!("Failed to write C header: {}", e));
            fs::write(c_dir.join(format!("{}_ffi.rs", stem)), bindings.c_wrapper)
                .unwrap_or_else(|e| panic!("Failed to write C Rust wrapper: {}", e));
            fs::write(rust_dir.join("lib.rs"), bindings.rust)
                .unwrap_or_else(|e| panic!("Failed to write Rust SDK: {}", e));
            fs::write(
                python_dir.join(format!("{}.py", stem.replace('-', "_"))),
                bindings.python,
            )
            .unwrap_or_else(|e| panic!("Failed to write Python SDK: {}", e));
            fs::write(typescript_dir.join("index.ts"), bindings.typescript)
                .unwrap_or_else(|e| panic!("Failed to write TypeScript SDK: {}", e));
            fs::write(kotlin_dir.join("Lean4Prod.kt"), bindings.kotlin)
                .unwrap_or_else(|e| panic!("Failed to write Kotlin SDK: {}", e));
            build_wasm_sdk(&bindings.wasm, &wasm_dir, &stem);
            println!("Generated SDK bundle: {}", root.display());
        }
        Commands::Sdk {
            path,
            language,
            output,
            stem,
            library_name,
        } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let (_, module) = prod_ir::parser::parse_module(&content)
                .unwrap_or_else(|e| panic!("Parse error: {:?}", e));
            let bindings = prod_codegen::generate_sdks(&module, &library_name)
                .unwrap_or_else(|e| panic!("SDK generation error: {}", e));
            let root = Path::new(&output).join(&stem);

            match language {
                SdkLanguage::C => {
                    let directory = root.join("c");
                    fs::create_dir_all(&directory).unwrap_or_else(|e| {
                        panic!("Failed to create {}: {}", directory.display(), e)
                    });
                    let header = directory.join(format!("{}.h", stem));
                    let wrapper = directory.join(format!("{}_ffi.rs", stem));
                    fs::write(&header, bindings.c_header)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", header.display(), e));
                    fs::write(&wrapper, bindings.c_wrapper)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", wrapper.display(), e));
                    println!(
                        "Generated C SDK: {} and {}",
                        header.display(),
                        wrapper.display()
                    );
                }
                SdkLanguage::Rust => {
                    let directory = root.join("rust");
                    fs::create_dir_all(&directory).unwrap_or_else(|e| {
                        panic!("Failed to create {}: {}", directory.display(), e)
                    });
                    let path = directory.join("lib.rs");
                    fs::write(&path, bindings.rust)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", path.display(), e));
                    println!("Generated Rust SDK: {}", path.display());
                }
                SdkLanguage::Python => {
                    let directory = root.join("python");
                    fs::create_dir_all(&directory).unwrap_or_else(|e| {
                        panic!("Failed to create {}: {}", directory.display(), e)
                    });
                    let path = directory.join(format!("{}.py", stem.replace('-', "_")));
                    fs::write(&path, bindings.python)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", path.display(), e));
                    println!("Generated Python SDK: {}", path.display());
                }
                SdkLanguage::Typescript => {
                    let directory = root.join("typescript");
                    fs::create_dir_all(&directory).unwrap_or_else(|e| {
                        panic!("Failed to create {}: {}", directory.display(), e)
                    });
                    let path = directory.join("index.ts");
                    fs::write(&path, bindings.typescript)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", path.display(), e));
                    println!("Generated TypeScript SDK: {}", path.display());
                }
                SdkLanguage::Kotlin => {
                    let directory = root.join("kotlin");
                    fs::create_dir_all(&directory).unwrap_or_else(|e| {
                        panic!("Failed to create {}: {}", directory.display(), e)
                    });
                    let path = directory.join("Lean4Prod.kt");
                    fs::write(&path, bindings.kotlin)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", path.display(), e));
                    println!("Generated Kotlin SDK: {}", path.display());
                }
                SdkLanguage::Wasm => {
                    let directory = root.join("wasm");
                    build_wasm_sdk(&bindings.wasm, &directory, &stem);
                    println!("Generated WebAssembly SDK: {}", directory.display());
                }
            }
        }
        Commands::Validate { path } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            match prod_ir::parser::parse_module(&content) {
                Ok((_, module)) => {
                    println!(
                        "✓ Valid IR: {} definitions in module '{}'",
                        module.definitions.len(),
                        module.name
                    );
                    let opaque: Vec<_> = module
                        .definitions
                        .iter()
                        .filter(|d| matches!(d.body, prod_ir::Expr::Opaque(_)))
                        .collect();
                    if !opaque.is_empty() {
                        println!("⚠ {} definitions contain opaque expressions", opaque.len());
                    }
                    let mut unresolved: Vec<String> = Vec::new();
                    for def in &module.definitions {
                        collect_externs(&def.body, &mut unresolved);
                    }
                    unresolved.sort();
                    unresolved.dedup();
                    if !unresolved.is_empty() {
                        println!("✗ {} unresolved call(s):", unresolved.len());
                        for name in &unresolved {
                            println!("    {}", name);
                        }
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("✗ Invalid IR: {:?}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Roots { command } => match command {
            RootCommands::Check { path, all } => {
                let roots = roots::load(&path).unwrap_or_else(|e| panic!("{e}"));
                let roots = roots::filter_roots(roots, all);
                let report = roots::check(&roots);
                println!(
                    "Loaded {} roots{}.",
                    roots.len(),
                    if all {
                        " (including auto-generated)"
                    } else {
                        ""
                    }
                );
                println!(
                    "Checking dependency acyclicity... {}",
                    if report.acyclic { "OK" } else { "FAILED" }
                );
                println!(
                    "Roots with empty dependencies: {}",
                    report.empty_dependencies.len()
                );
                if !report.duplicate_ids.is_empty() {
                    println!(
                        "Warning: duplicate root ids: {}",
                        report.duplicate_ids.join(", ")
                    );
                }
                if report.acyclic && report.empty_dependencies.is_empty() {
                    println!("All root checks passed.");
                } else {
                    std::process::exit(1);
                }
            }
            RootCommands::Pareto { path, all } => {
                let roots = roots::load(&path).unwrap_or_else(|e| panic!("{e}"));
                let roots = roots::filter_roots(roots, all);
                let front = roots::pareto_front(&roots);
                println!("Pareto front: {} roots", front.len());
                for idx in front {
                    let root = &roots[idx];
                    println!(
                        "  {} [{}] (proof_term_size={}, kernel_depth={}, check_time_ns={})",
                        roots::short_name(&root.id),
                        root.id,
                        root.proof_term_size,
                        root.kernel_depth,
                        root.check_time_ns
                    );
                }
            }
            RootCommands::Connect {
                path,
                root1,
                root2,
                all,
            } => {
                let roots = roots::load(&path).unwrap_or_else(|e| panic!("{e}"));
                let roots = roots::filter_roots(roots, all);
                let candidates = roots::bridges(&roots);
                let selected = match (root1, root2) {
                    (Some(a), Some(b)) => candidates
                        .into_iter()
                        .filter(|bridge| {
                            (roots::id_matches(&roots[bridge.left].id, &a)
                                && roots::id_matches(&roots[bridge.right].id, &b))
                                || (roots::id_matches(&roots[bridge.left].id, &b)
                                    && roots::id_matches(&roots[bridge.right].id, &a))
                        })
                        .collect(),
                    (None, None) => candidates,
                    _ => {
                        eprintln!("connect expects either zero or two root ids");
                        std::process::exit(2);
                    }
                };
                for bridge in &selected {
                    let left = roots::short_name(&roots[bridge.left].id);
                    let right = roots::short_name(&roots[bridge.right].id);
                    println!("bridge_{left}_{right}: {left} ↔ {right}");
                    println!("  shared kernel dependencies: {}", bridge.shared.join(", "));
                }
                println!("Generated {} bridge hypotheses.", selected.len());
            }
        },
        Commands::Subset { path, output } => {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            let subset: SubsetFile = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path, e));
            let rendered = render_subset(&subset);
            match output {
                Some(output) => {
                    fs::write(&output, &rendered)
                        .unwrap_or_else(|e| panic!("Failed to write {}: {}", output, e));
                    println!("Generated: {}", output);
                }
                None => print!("{}", rendered),
            }
        }
    }
}

#[cfg(test)]
mod sdk_tests {
    use super::wasm_package_name;

    #[test]
    fn wasm_package_names_are_valid_and_stable() {
        assert_eq!(wasm_package_name("lean4-prod"), "lean4-prod");
        assert_eq!(wasm_package_name("42 / Demo SDK"), "lean-42---demo-sdk");
        assert_eq!(wasm_package_name("---"), "lean4-prod-sdk");
    }
}
