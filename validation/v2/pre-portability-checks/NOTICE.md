This development pass completed with all command exit statuses zero. Its long
suite binary predates the final pointer-width hash-reduction fix. The source
was changed while that binary ran, and the old verifier only hashed sources
at the end. Therefore source-hashes.json is NOT evidence that this pass tested
those exact final sources. Do not use this folder as the final correctness gate.

The verifier now hashes sources both before and after the complete pass and
rejects any change. The definitive rerun is ../final-checks/.
