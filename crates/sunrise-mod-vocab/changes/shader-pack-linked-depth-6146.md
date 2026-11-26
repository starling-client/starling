Linked shader-pack passes now honor the pipeline's declared depth comparison (flipped for
standard-Z where directional) and write flag independently. If a pipeline relied on a writable
pack target implicitly enabling depth writes, add `.depth_write(true)` to its declaration.
This does not make previously read-only slots writable without an explicit target.
