//! Evidence for custom Serde forms in `sunrise-mod.toml`.

#[rustfmt::skip]
pub(super) const REVIEWED: &[super::Reviewed] = &[
    (("manifest.rs", "version", "deserialize_with = \"version_from_str\""), &["manifest_full", "manifest_minimal", "refused_manifest_bad_version"]),
    (("manifest.rs", "dependencies", "deserialize_with = \"policy_map\""), &["manifest_full", "refused_manifest_bad_dependency_id"]),
    (("manifest.rs", "incompatible", "deserialize_with = \"policy_map\""), &["manifest_full", "src/dependencies.rs:policy_schema_errors_are_refused_when_the_manifest_is_parsed"]),
    (("manifest.rs", "discouraged", "deserialize_with = \"policy_map\""), &["manifest_full", "src/dependencies.rs:policy_schema_errors_are_refused_when_the_manifest_is_parsed"]),
    (("manifest.rs", "load_before", "deserialize_with = \"load_before_ids\""), &["manifest_full", "src/dependencies.rs:policy_schema_errors_are_refused_when_the_manifest_is_parsed"]),
    (("manifest.rs", "Property", "untagged"), &["manifest_full", "refused_manifest_bad_property_type"]),
    (("manifest.rs", "Dependency", "impl Deserialize"), &["manifest_full", "refused_manifest_unknown_dependency_key"]),
    (("manifest.rs", "url", "deserialize_with = \"DependencyDetails::deserialize_url\""), &["manifest_full", "src/dependencies.rs:policy_schema_errors_are_refused_when_the_manifest_is_parsed"]),
    (("manifest.rs", "Rule", "impl Deserialize"), &["manifest_full", "keys_manifest_forms", "refused_manifest_unknown_rule_key"]),
    (("manifest.rs", "url", "deserialize_with = \"RuleDetails::deserialize_url\""), &["manifest_full", "refused_manifest_bad_rule_url"]),
    (("manifest/jar.rs", "JarRef", "try_from = \"Fields\""), &["manifest_full", "refused_manifest_bad_jar_id", "refused_manifest_bad_jar_hash", "refused_manifest_empty_jar_version"]),
    (("manifest/runtime_files.rs", "RuntimeTable", "try_from = \"BTreeMap<String, String>\""), &["refused_manifest_runtime_on_content", "refused_manifest_empty_runtime", "refused_manifest_bad_runtime_hash", "refused_manifest_bad_runtime_path"]),
    // Not content declarations. Name the reader's unit-test fixtures instead of
    // admitting exact-build native mods or cached root metadata into this corpus.
    (("manifest/jar.rs", "JarRequirement", "try_from = \"RequirementFields\""), &["src/manifest/jar.rs:decoded_root_requirements_validate_owners_at_the_read_boundary"]),
    (("manifest/mixin.rs", "options", "flatten"), &["src/manifest/mixin.rs:the_tools_keys_pass_through_and_the_packages_are_checked"]),
    (("manifest/mixin.rs", "fields", "flatten"), &["src/manifest/mixin.rs:mixins_bridges_states_modules_share_one_id_space"]),
    (("manifest/mixin.rs", "Kind", "impl Deserialize"), &["src/manifest/mixin.rs:the_tools_keys_pass_through_and_the_packages_are_checked", "src/manifest/mixin.rs:hostile_shapes_are_refused_without_panicking"]),
];
