# kampr-core

Providers, the pane registry, stream supervision, the wire encoder, and `local_node` — the part of
a node's config a client on the same machine reads.

## The wire is additive only

Older clients are installed on real phones. Unknown `t` values and unknown fields are ignored by
rule; an unrecognised `error.code` must still render its `message`. New optional fields and new
`t` values are fine; removing, renaming or retyping one is not.

`tests/it/wire_shapes.rs` holds that mechanically: it builds every `ServerMsg` and `ClientMsg` with
every optional field present and compares their paths and JSON types with
`tests/fixtures/wire-shapes.txt`. A removed or retyped path fails. An added one passes; record it
with `KAMPR_BLESS_WIRE=1 cargo test -p kampr-core --test it wire_shapes` so it is guarded from
then on. A new message type fails until it has a sample there.
