# Typed decimal and split fixture

`src/Main.lex.tex` is authoritative; LexLean generated every committed Lean
byte. The retained build manifest binds its source, lock, configuration and
complete output set. `fixture-manifest.json` pins the compiler and copied module.

Reproduce with that LexLean compiler: `lexlean fmt --check`, `lexlean lock --check`,
then `lexlean verify`. `just typed-decimal` rechecks the frozen binding, exports
the generated module twice and executes its generated native and Wasm code.
The fixture tests compiler behavior; it is not an application release receipt.

The split extension preserves UInt32 maximum, zero, one and parameter bounds;
all original decimal definitions and acceptance vectors are unchanged.
