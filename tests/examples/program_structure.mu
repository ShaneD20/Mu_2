/* Spec §3: file layout, imports, globals and the entry point. */
import log                      /* imports first, file-wide */
import shapes

share as limit: 100             /* top-level bindings */
as #total: 0

main f() -> u8 { => 0 }         /* definitions */
