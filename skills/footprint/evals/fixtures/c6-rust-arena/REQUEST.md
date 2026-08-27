Use `nodes.rs` as the current boxed representation. Keep number, binary, and 96-byte text payload behavior for 2,000,000 nodes. Move nodes to one arena. Use a `u32` newtype index. Keep rare large payloads in a side table.

Run the probe before and after. Put exact output and moves in `REPORT.md`.
