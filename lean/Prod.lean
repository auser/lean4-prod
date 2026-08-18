import Prod.Attribute
import Prod.Extract
import Prod.Lower
import Prod.Roots
import Prod.Coverage
import Prod.Export

/-!
# Prod — the Lean → sexp IR extractor

Root module for `ProdLib`. Deliberately does NOT import the worked example
(`Example`): the library is example-agnostic. `Prod.Emit` is excluded for the
same reason — it imports `Example` and is built via the `prod-export`
executable target, which has it as its root.
-/
