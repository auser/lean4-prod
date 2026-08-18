import Lean
import Prod
import UORProd

open Lean

namespace UORProdExport

def run : CoreM (Except String Prod.ModuleExport) := do
  try
    pure (.ok (← Prod.exportModule `UORProd "UOR.Framework"))
  catch e =>
    pure (.error (← e.toMessageData.toString))

private def parseOutDir : List String → System.FilePath
  | "--out" :: dir :: _ => dir
  | _ :: rest => parseOutDir rest
  | [] => "../../output/uor"

end UORProdExport

unsafe def main (args : List String) : IO Unit := do
  Lean.initSearchPath (← Lean.findSysroot)
  Lean.enableInitializersExecution
  let env ← Lean.importModules #[{ module := `UORProd }]
    {} (leakEnv := true) (loadExts := true)
  let coreCtx : Core.Context := { fileName := "uor-prod-export", fileMap := default }
  let eio := (ReaderT.run UORProdExport.run coreCtx).run { env := env }
  let result ← EIO.toIO' eio
  let exported ← match result with
    | .ok (.ok output, _) => pure output
    | .ok (.error msg, _) => throw (IO.userError s!"uor-prod-export failed: {msg}")
    | .error _ =>
      throw (IO.userError "uor-prod-export failed: uncaught exception")
  let out := UORProdExport.parseOutDir args
  IO.FS.createDirAll out
  IO.FS.writeFile (out / "kernel.ir") exported.ir
  IO.FS.writeFile (out / "roots.json") exported.roots
  IO.FS.writeFile (out / "coverage.md") exported.coverage
  IO.println s!"uor-prod-export: wrote {out / "kernel.ir"}, {out / "roots.json"}, {out / "coverage.md"}"
