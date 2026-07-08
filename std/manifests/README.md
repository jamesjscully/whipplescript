# std/manifests

Canonical embedded standard-package manifests. These files are compiled into
the platform binary and are its single source of truth for std packages:

- the CLI registers them as `EMBEDDED_STD_MANIFESTS` (contract registration —
  `use std.memory` / `use std.messaging` resolve with no package lock), and
- the parser's build script generates the `EFFECT_OPERATION_GRAMMAR` table from
  each construct's `grammar` object (spec/construct-grammar.md, DR-0011), so
  the manifests are also the source of the parse grammar for `recall`, `learn`,
  `curate`, and `send`.

The `std.*` package namespace is reserved: a package lock can never provide
these names, so the embedded copies always win. Vendor demo packages stay in
`examples/packages/`.
