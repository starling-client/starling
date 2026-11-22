# Injection points removed

Sunrise no longer supports `[[inject]]` or emits `points.json`. The `sunrise-mod-points` crate (`mod_points`), its generated argument records, `PointId`, `listeners` and registration API are gone. The SDK no longer ships their catalog or library, and `cargo sunrise-mod` no longer has an `inject` command or an `update --accept` option.

Typed hooks, content mods, the internals catalog, and all `[native.*]` routes remain.
