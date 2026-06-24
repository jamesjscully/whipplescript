#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
try:
    import jsonschema  # noqa: F401
except Exception as exc:
    raise SystemExit(
        "python jsonschema package is required; run under `nix develop` or "
        f"install `requirements-dev.txt`: {exc}"
    )
PY

TMP_DIR="$(mktemp -d)"
# A lock records its package `source.path` relative to the lock file's own
# directory (and a portable lock forbids `..`), so the lock must live at the
# project root for `examples/packages/memory.json` to resolve. Keep it at the root
# (not in $TMP_DIR) and clean it up with the temp dir.
LOCK_PATH="$ROOT/.artifact-differential-package-lock.json"
trap 'rm -rf "$TMP_DIR" "$LOCK_PATH"' EXIT

cargo run --quiet -p whipplescript -- package catalog \
  > "$TMP_DIR/platform-construct-catalog.json"
cargo run --quiet -p whipplescript -- package lock \
  --output "$LOCK_PATH" \
  examples/packages/memory.json \
  >/dev/null
cargo run --quiet -p whipplescript -- --json check \
  --package-lock "$LOCK_PATH" \
  examples/package-memory.whip \
  > "$TMP_DIR/package-memory-check.json"
cargo run --quiet -p whipplescript -- --json compile \
  --package-lock "$LOCK_PATH" \
  examples/package-memory.whip \
  > "$TMP_DIR/package-memory-compile.json"
cargo run --quiet -p whipplescript -- verify-report --emit construct-graph \
  "$TMP_DIR/package-memory-check.json" \
  > "$TMP_DIR/package-memory-construct-graph.json"
cargo run --quiet -p whipplescript -- verify-report --emit lowered-ir \
  "$TMP_DIR/package-memory-check.json" \
  > "$TMP_DIR/package-memory-lowered-ir.json"
cargo run --quiet -p whipplescript -- verify-report --emit artifacts \
  "$TMP_DIR/package-memory-check.json" \
  > "$TMP_DIR/package-memory-artifacts.json"

python3 - "$ROOT" "$TMP_DIR" <<'PY'
import copy
import hashlib
import json
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
tmp_dir = Path(sys.argv[2])
check_report = json.loads((tmp_dir / "package-memory-check.json").read_text())
compile_report = json.loads((tmp_dir / "package-memory-compile.json").read_text())
construct_graph_bundle = json.loads(
    (tmp_dir / "package-memory-construct-graph.json").read_text()
)
lowered_ir_bundle = json.loads((tmp_dir / "package-memory-lowered-ir.json").read_text())
artifacts_bundle = json.loads((tmp_dir / "package-memory-artifacts.json").read_text())
cases_dir = tmp_dir / "cases"
cases_dir.mkdir()


def canonical_json(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def rehash_package_contract(entry):
    body = copy.deepcopy(entry["package_contract"])
    body.pop("package_contract_digest", None)
    digest = hashlib.sha256(canonical_json(body).encode("utf-8")).hexdigest()
    entry["package_contract"]["package_contract_digest"] = digest
    entry["construct_graph"]["package_contract_digest"] = digest


def check_report_entry(document):
    return document[0]


def compile_report_entry(document):
    return document


def verified_bundle_entry(document):
    return document["entries"][0]


ENTRY_SOURCES = [
    ("check", check_report, check_report_entry),
    ("compile", compile_report, compile_report_entry),
    ("verified-cg", construct_graph_bundle, verified_bundle_entry),
    ("verified-lowered-ir", lowered_ir_bundle, verified_bundle_entry),
    ("verified-artifacts", artifacts_bundle, verified_bundle_entry),
]

VERIFIED_BUNDLES = [
    ("verified-cg", construct_graph_bundle),
    ("verified-lowered-ir", lowered_ir_bundle),
    ("verified-artifacts", artifacts_bundle),
]


def add_case(name, source, entry_getter, mutator):
    mutated = copy.deepcopy(source)
    try:
        mutator(entry_getter(mutated))
    except Exception as exc:
        raise SystemExit(f"failed to build artifact admission case {name}: {exc}") from exc
    (cases_dir / f"{name}.json").write_text(json.dumps(mutated))


def add_bundle_case(name, source, mutator):
    mutated = copy.deepcopy(source)
    try:
        mutator(mutated)
    except Exception as exc:
        raise SystemExit(f"failed to build artifact admission case {name}: {exc}") from exc
    (cases_dir / f"{name}.json").write_text(json.dumps(mutated))


def add_verified_bundle_cases(name, mutator):
    for prefix, source in VERIFIED_BUNDLES:
        add_bundle_case(f"{prefix}-{name}", source, mutator)


def add_entry_cases(name, mutator):
    for prefix, source, entry_getter in ENTRY_SOURCES:
        add_case(f"{prefix}-{name}", source, entry_getter, mutator)


def add_full_case(name, mutator):
    add_case(f"check-{name}", check_report, check_report_entry, mutator)
    add_case(f"compile-{name}", compile_report, compile_report_entry, mutator)
    add_case(
        f"verified-lowered-ir-{name}",
        lowered_ir_bundle,
        verified_bundle_entry,
        mutator,
    )
    add_case(
        f"verified-artifacts-{name}",
        artifacts_bundle,
        verified_bundle_entry,
        mutator,
    )


def add_construct_graph_cases(name, mutator):
    add_case(f"check-{name}", check_report, check_report_entry, mutator)
    add_case(f"compile-{name}", compile_report, compile_report_entry, mutator)
    add_case(
        f"verified-cg-{name}",
        construct_graph_bundle,
        verified_bundle_entry,
        mutator,
    )
    add_case(
        f"verified-lowered-ir-{name}",
        lowered_ir_bundle,
        verified_bundle_entry,
        mutator,
    )
    add_case(
        f"verified-artifacts-{name}",
        artifacts_bundle,
        verified_bundle_entry,
        mutator,
    )


def first_construct_graph_interface(entry):
    for node in entry["construct_graph"]["nodes"]:
        for key in ["declared_required_interfaces", "declared_provided_interfaces"]:
            values = node.get(key)
            if isinstance(values, list) and values:
                return values[0]
    raise RuntimeError("construct graph did not contain a declared interface")


def bad_entrypoint_value(entry):
    refs = entry["lowered_ir_report"]["core_objects"][0]["entrypoint_refs"]
    key = next(iter(refs.keys()))
    refs[key] = 1


def core_object_entrypoint_without_refs(object_kind, runtime_entrypoint):
    def mutate(entry):
        for core_object in entry["lowered_ir_report"]["core_objects"]:
            if (
                core_object.get("object_kind") == "effect"
                and core_object.get("runtime_entrypoint") == "effect_graph_template"
            ):
                core_object["object_kind"] = object_kind
                core_object["runtime_entrypoint"] = runtime_entrypoint
                return
        raise RuntimeError("lowered IR report did not contain an effect core object")

    return mutate


def duplicate_construct_graph_node_id(entry):
    nodes = entry["construct_graph"]["nodes"]
    duplicate = copy.deepcopy(nodes[0])
    duplicate["construct_id"] = f"{duplicate.get('construct_id', 'construct')}.semantic-duplicate"
    nodes.append(duplicate)


def duplicate_construct_graph_port_id(entry):
    ports = entry["construct_graph"]["ports"]
    duplicate = copy.deepcopy(ports[0])
    duplicate["resource_identity"] = (
        f"{duplicate.get('resource_identity', 'resource')}:semantic-duplicate"
    )
    ports.append(duplicate)


def duplicate_construct_graph_edge_ref(entry):
    edges = entry["construct_graph"]["edges"]
    duplicate = copy.deepcopy(edges[0])
    evidence = duplicate.setdefault("evidence", [])
    if isinstance(evidence, list):
        evidence.append("semantic-duplicate-edge")
    edges.append(duplicate)


def duplicate_construct_graph_dependency_ref(entry):
    dependencies = entry["construct_graph"]["effect_dependencies"]
    duplicate = copy.deepcopy(dependencies[0])
    duplicate["rule_name"] = f"{duplicate.get('rule_name', 'rule')}:semantic-duplicate"
    dependencies.append(duplicate)


def duplicate_lowered_core_object_id(entry):
    core_objects = entry["lowered_ir_report"]["core_objects"]
    duplicate = copy.deepcopy(core_objects[0])
    refs = duplicate.setdefault("resource_refs", [])
    if isinstance(refs, list):
        refs.append("semantic-duplicate-resource")
    core_objects.append(duplicate)


def lowered_edge_ref(lowering):
    return f"{lowering['required_port_id']}->{lowering['provided_port_id']}"


def produced_refs_by_owner(entry, owner_kind):
    lowered = entry["lowered_ir_report"]
    if owner_kind == "node":
        return (
            lowered.get("node_lowerings", []),
            "node_id",
            lambda lowering: lowering.get("node_id"),
        )
    if owner_kind == "edge":
        return (
            lowered.get("edge_lowerings", []),
            None,
            lowered_edge_ref,
        )
    if owner_kind == "dependency":
        return (
            lowered.get("dependency_lowerings", []),
            "dependency_ref",
            lambda lowering: lowering.get("dependency_ref"),
        )
    raise RuntimeError(f"unsupported owner kind {owner_kind}")


def append_object_to_lowering_owner(entry, object_id, owner_kind, owner_ref=None):
    lowerings, ref_field, ref_fn = produced_refs_by_owner(entry, owner_kind)
    for lowering in lowerings:
        candidate_ref = ref_fn(lowering)
        if owner_ref is not None and candidate_ref != owner_ref:
            continue
        refs = lowering.get("produced_core_object_refs")
        if not isinstance(refs, list):
            continue
        if object_id not in refs:
            refs.append(object_id)
            return candidate_ref
    if ref_field is None:
        raise RuntimeError(f"lowered IR report did not contain target {owner_kind} owner")
    raise RuntimeError(
        f"lowered IR report did not contain target {owner_kind} owner {owner_ref!r}"
    )


def remove_object_from_all_lowering_owners(entry, object_id):
    lowered = entry["lowered_ir_report"]
    for collection in [
        "node_lowerings",
        "edge_lowerings",
        "dependency_lowerings",
    ]:
        for lowering in lowered.get(collection, []):
            refs = lowering.get("produced_core_object_refs")
            if isinstance(refs, list):
                lowering["produced_core_object_refs"] = [
                    ref for ref in refs if ref != object_id
                ]


def first_core_object_with_owner(entry, owner_kind):
    for core_object in entry["lowered_ir_report"]["core_objects"]:
        if (
            isinstance(core_object, dict)
            and core_object.get("owner_kind") == owner_kind
            and isinstance(core_object.get("object_id"), str)
            and isinstance(core_object.get("owner_ref"), str)
        ):
            return core_object
    raise RuntimeError(f"lowered IR report did not contain a {owner_kind}-owned core object")


def duplicate_lowered_core_object_node_owner(entry):
    core_object = first_core_object_with_owner(entry, "node")
    append_object_to_lowering_owner(
        entry,
        core_object["object_id"],
        "node",
        owner_ref=None,
    )


def duplicate_lowered_core_object_node_edge_owner(entry):
    core_object = first_core_object_with_owner(entry, "node")
    append_object_to_lowering_owner(entry, core_object["object_id"], "edge")


def duplicate_lowered_core_object_dependency_owner(entry):
    core_object = first_core_object_with_owner(entry, "dependency")
    append_object_to_lowering_owner(entry, core_object["object_id"], "dependency")


def duplicate_lowered_core_object_edge_owner(entry):
    lowered = entry["lowered_ir_report"]
    edge_lowerings = lowered.get("edge_lowerings")
    if not isinstance(edge_lowerings, list) or len(edge_lowerings) < 2:
        raise RuntimeError("lowered IR report did not contain two edge lowerings")
    core_object = first_core_object_with_owner(entry, "node")
    object_id = core_object["object_id"]
    remove_object_from_all_lowering_owners(entry, object_id)
    first_edge_ref = lowered_edge_ref(edge_lowerings[0])
    core_object["owner_kind"] = "edge"
    core_object["owner_ref"] = first_edge_ref
    edge_lowerings[0]["produced_core_object_refs"].append(object_id)
    edge_lowerings[1]["produced_core_object_refs"].append(object_id)


def stale_ir_hash(entry):
    entry["ir_hash"] = (
        "0000000000000000"
        if entry.get("ir_hash") != "0000000000000000"
        else "1111111111111111"
    )


def stale_package_contract_digest(entry):
    entry["package_contract"]["package_contract_digest"] = (
        "0" * 64
        if entry["package_contract"].get("package_contract_digest") != "0" * 64
        else "1" * 64
    )


def stale_package_contract_registry_rehashed(entry):
    entry["package_contract"]["contract_registry"]["diagnostics"].append(
        {
            "code": "synthetic.contract_registry_mismatch",
            "message": "synthetic embedded registry mismatch",
        }
    )
    rehash_package_contract(entry)


def stale_platform_catalog_rehashed(entry):
    entry["package_contract"]["platform_construct_catalog"]["interface_kinds"].append(
        "synthetic.stale_interface_kind"
    )
    rehash_package_contract(entry)


def stale_contract_registry_schema_rehashed(field, value):
    def mutate(entry):
        for registry in [
            entry["contract_registry"],
            entry["package_contract"]["contract_registry"],
        ]:
            package_library_ids = {
                library.get("id")
                for library in registry.get("libraries", [])
                if isinstance(library, dict)
                and isinstance(library.get("id"), str)
                and not library.get("standard", False)
                and library.get("version") != "unlocked"
            }
            contracts = registry.get("effect_contracts")
            if not isinstance(contracts, list) or not contracts:
                raise RuntimeError("contract registry did not contain effect contracts")
            for contract in contracts:
                if (
                    isinstance(contract, dict)
                    and contract.get("library_id") in package_library_ids
                ):
                    contract[field] = value
                    break
            else:
                raise RuntimeError("contract registry did not contain package effect contracts")
        rehash_package_contract(entry)

    return mutate


def stale_construct_input_field_rehashed(entry):
    for registry in [
        entry["contract_registry"],
        entry["package_contract"]["contract_registry"],
    ]:
        package_library_ids = {
            library.get("id")
            for library in registry.get("libraries", [])
            if isinstance(library, dict)
            and isinstance(library.get("id"), str)
            and not library.get("standard", False)
            and library.get("version") != "unlocked"
        }
        contracts = registry.get("effect_contracts")
        constructs = registry.get("constructs")
        if not isinstance(contracts, list) or not isinstance(constructs, list):
            raise RuntimeError("contract registry did not contain contracts and constructs")
        for construct in constructs:
            if (
                not isinstance(construct, dict)
                or construct.get("library_id") not in package_library_ids
                or construct.get("lowering_target") != "capability_call"
            ):
                continue
            target_capability = construct.get("target_capability")
            contract = next(
                (
                    candidate
                    for candidate in contracts
                    if isinstance(candidate, dict)
                    and candidate.get("id") == target_capability
                    and candidate.get("effect_kind") == "capability.call"
                    and candidate.get("library_id") in package_library_ids
                ),
                None,
            )
            if not isinstance(contract, dict) or not isinstance(
                contract.get("input_schema"), str
            ):
                continue
            try:
                input_schema = json.loads(contract["input_schema"])
            except Exception:
                continue
            if not isinstance(input_schema, dict):
                continue
            fields = construct.get("fields")
            if not isinstance(fields, list):
                continue
            for input_field in input_schema:
                field = next(
                    (
                        candidate
                        for candidate in fields
                        if isinstance(candidate, dict)
                        and candidate.get("name") == input_field
                    ),
                    None,
                )
                if isinstance(field, dict):
                    field["required"] = False
                    break
            else:
                continue
            break
        else:
            raise RuntimeError("contract registry did not contain a construct input field")
    rehash_package_contract(entry)


def stale_construct_field_kind_rehashed(entry):
    for registry in [
        entry["contract_registry"],
        entry["package_contract"]["contract_registry"],
    ]:
        package_library_ids = {
            library.get("id")
            for library in registry.get("libraries", [])
            if isinstance(library, dict)
            and isinstance(library.get("id"), str)
            and not library.get("standard", False)
            and library.get("version") != "unlocked"
        }
        constructs = registry.get("constructs")
        if not isinstance(constructs, list):
            raise RuntimeError("contract registry did not contain constructs")
        for construct in constructs:
            if (
                not isinstance(construct, dict)
                or construct.get("library_id") not in package_library_ids
            ):
                continue
            fields = construct.get("fields")
            if not isinstance(fields, list):
                continue
            for field in fields:
                if isinstance(field, dict) and isinstance(field.get("kind"), str):
                    field["kind"] = "synthetic.unsupported_field_kind"
                    break
            else:
                continue
            break
        else:
            raise RuntimeError("contract registry did not contain a package construct field")
    rehash_package_contract(entry)


def stale_construct_target_capability_rehashed(entry):
    missing = "synthetic.missing_capability"
    for registry in [
        entry["contract_registry"],
        entry["package_contract"]["contract_registry"],
    ]:
        package_library_ids = {
            library.get("id")
            for library in registry.get("libraries", [])
            if isinstance(library, dict)
            and isinstance(library.get("id"), str)
            and not library.get("standard", False)
            and library.get("version") != "unlocked"
        }
        constructs = registry.get("constructs")
        if not isinstance(constructs, list):
            raise RuntimeError("contract registry did not contain constructs")
        for construct in constructs:
            if (
                not isinstance(construct, dict)
                or construct.get("library_id") not in package_library_ids
                or construct.get("lowering_target") != "capability_call"
            ):
                continue
            construct["target_capability"] = missing
            requires = construct.get("requires")
            if isinstance(requires, list):
                for interface in requires:
                    if (
                        isinstance(interface, dict)
                        and interface.get("kind") == "Capability"
                    ):
                        interface["name"] = missing
                        break
            break
        else:
            raise RuntimeError("contract registry did not contain a package capability construct")
    rehash_package_contract(entry)


def stale_effect_required_capability_rehashed(entry):
    missing = "synthetic.missing_capability"
    for registry in [
        entry["contract_registry"],
        entry["package_contract"]["contract_registry"],
    ]:
        package_library_ids = {
            library.get("id")
            for library in registry.get("libraries", [])
            if isinstance(library, dict)
            and isinstance(library.get("id"), str)
            and not library.get("standard", False)
            and library.get("version") != "unlocked"
        }
        contracts = registry.get("effect_contracts")
        if not isinstance(contracts, list):
            raise RuntimeError("contract registry did not contain effect contracts")
        for contract in contracts:
            if (
                isinstance(contract, dict)
                and contract.get("library_id") in package_library_ids
                and contract.get("effect_kind") == "capability.call"
            ):
                contract["required_capabilities"] = [missing]
                break
        else:
            raise RuntimeError("contract registry did not contain a package effect contract")
    rehash_package_contract(entry)


def stale_construct_keyword_duplicate_rehashed(entry):
    for registry in [
        entry["contract_registry"],
        entry["package_contract"]["contract_registry"],
    ]:
        package_library_ids = {
            library.get("id")
            for library in registry.get("libraries", [])
            if isinstance(library, dict)
            and isinstance(library.get("id"), str)
            and not library.get("standard", False)
            and library.get("version") != "unlocked"
        }
        constructs = registry.get("constructs")
        if not isinstance(constructs, list):
            raise RuntimeError("contract registry did not contain constructs")
        for construct in constructs:
            if (
                isinstance(construct, dict)
                and construct.get("library_id") in package_library_ids
                and isinstance(construct.get("id"), str)
            ):
                duplicate = copy.deepcopy(construct)
                duplicate["id"] = f"{construct['id']}.duplicate"
                constructs.append(duplicate)
                break
        else:
            raise RuntimeError("contract registry did not contain a package construct")
    rehash_package_contract(entry)


add_verified_bundle_cases("bundle-schema-number", lambda bundle: bundle.__setitem__("schema", 1))
add_verified_bundle_cases("bundle-status-error", lambda bundle: bundle.__setitem__("status", "error"))
add_verified_bundle_cases("bundle-emit-number", lambda bundle: bundle.__setitem__("emit", 1))
add_verified_bundle_cases("bundle-emit-unknown", lambda bundle: bundle.__setitem__("emit", "unknown"))
add_verified_bundle_cases("bundle-entries-scalar", lambda bundle: bundle.__setitem__("entries", "bad"))
add_verified_bundle_cases("bundle-entries-empty", lambda bundle: bundle.__setitem__("entries", []))
add_verified_bundle_cases("bundle-extra-field", lambda bundle: bundle.__setitem__("unexpected", "bad"))
add_verified_bundle_cases(
    "bundle-entry-extra-field",
    lambda bundle: bundle["entries"][0].__setitem__("unexpected", "bad"),
)
add_bundle_case(
    "verified-cg-bundle-entry-spurious-lowered-ir",
    construct_graph_bundle,
    lambda bundle: bundle["entries"][0].__setitem__(
        "lowered_ir_report",
        copy.deepcopy(lowered_ir_bundle["entries"][0]["lowered_ir_report"]),
    ),
)
add_bundle_case(
    "verified-lowered-ir-bundle-entry-missing-lowered-ir",
    lowered_ir_bundle,
    lambda bundle: bundle["entries"][0].pop("lowered_ir_report", None),
)
add_bundle_case(
    "verified-artifacts-bundle-entry-missing-lowered-ir",
    artifacts_bundle,
    lambda bundle: bundle["entries"][0].pop("lowered_ir_report", None),
)
add_bundle_case(
    "verified-lowered-ir-bundle-entry-missing-construct-graph",
    lowered_ir_bundle,
    lambda bundle: bundle["entries"][0].pop("construct_graph", None),
)
add_bundle_case(
    "verified-artifacts-bundle-entry-missing-construct-graph",
    artifacts_bundle,
    lambda bundle: bundle["entries"][0].pop("construct_graph", None),
)


add_entry_cases("entry-source-hash-number", lambda entry: entry.__setitem__("source_hash", 1))
add_entry_cases("entry-ir-hash-number", lambda entry: entry.__setitem__("ir_hash", 1))
add_entry_cases("entry-ir-hash-stale", stale_ir_hash)
add_entry_cases("entry-snapshot-number", lambda entry: entry.__setitem__("snapshot", 1))
add_entry_cases(
    "entry-contract-registry-diagnostics",
    lambda entry: entry["contract_registry"]["diagnostics"].append(
        {
            "code": "synthetic.contract_registry_diagnostic",
            "message": "synthetic contract registry diagnostic",
        }
    ),
)
add_entry_cases("entry-package-contract-digest-stale", stale_package_contract_digest)
add_entry_cases(
    "entry-package-contract-registry-mismatch-rehashed",
    stale_package_contract_registry_rehashed,
)
add_entry_cases(
    "entry-package-contract-platform-catalog-stale-rehashed",
    stale_platform_catalog_rehashed,
)
add_entry_cases(
    "entry-contract-registry-input-schema-unsupported-rehashed",
    stale_contract_registry_schema_rehashed("input_schema", "unsupported.custom"),
)
add_entry_cases(
    "entry-contract-registry-output-schema-tuple-rehashed",
    stale_contract_registry_schema_rehashed("output_schema", '["string","integer"]'),
)
add_entry_cases(
    "entry-construct-input-field-optional-rehashed",
    stale_construct_input_field_rehashed,
)
add_entry_cases(
    "entry-construct-field-kind-unsupported-rehashed",
    stale_construct_field_kind_rehashed,
)
add_entry_cases(
    "entry-construct-target-capability-missing-rehashed",
    stale_construct_target_capability_rehashed,
)
add_entry_cases(
    "entry-effect-required-capability-missing-rehashed",
    stale_effect_required_capability_rehashed,
)
add_entry_cases(
    "entry-construct-keyword-duplicate-rehashed",
    stale_construct_keyword_duplicate_rehashed,
)
add_construct_graph_cases(
    "cg-node-id-semantic-duplicate",
    duplicate_construct_graph_node_id,
)
add_construct_graph_cases(
    "cg-port-id-semantic-duplicate",
    duplicate_construct_graph_port_id,
)
add_construct_graph_cases(
    "cg-edge-ref-semantic-duplicate",
    duplicate_construct_graph_edge_ref,
)
add_construct_graph_cases(
    "cg-dependency-ref-semantic-duplicate",
    duplicate_construct_graph_dependency_ref,
)
add_full_case(
    "lir-core-object-id-semantic-duplicate",
    duplicate_lowered_core_object_id,
)
add_full_case(
    "lir-core-object-node-owner-duplicate",
    duplicate_lowered_core_object_node_owner,
)
add_full_case(
    "lir-core-object-node-edge-owner-duplicate",
    duplicate_lowered_core_object_node_edge_owner,
)
add_full_case(
    "lir-core-object-edge-owner-duplicate",
    duplicate_lowered_core_object_edge_owner,
)
add_full_case(
    "lir-core-object-dependency-owner-duplicate",
    duplicate_lowered_core_object_dependency_owner,
)


for field in [
    "graph_id",
    "platform_version",
    "package_lock_digest",
    "package_contract_digest",
    "source_digest",
]:
    add_construct_graph_cases(
        f"cg-root-{field}-number",
        lambda entry, field=field: entry["construct_graph"].__setitem__(field, 1),
    )

for field in [
    "node_id",
    "construct_id",
    "construct_family",
    "lowering_class",
    "lifecycle_profile",
    "owner",
    "lowering_output_kind",
]:
    add_construct_graph_cases(
        f"cg-node-{field}-number",
        lambda entry, field=field: entry["construct_graph"]["nodes"][0].__setitem__(
            field, 1
        ),
    )

add_construct_graph_cases(
    "cg-node-metadata-string",
    lambda entry: entry["construct_graph"]["nodes"][0].__setitem__(
        "metadata", "bad"
    ),
)

for field in [
    "required_ports",
    "produced_ports",
    "required_capabilities",
    "lowered_effect_capabilities",
    "allowed_core_object_kinds",
    "allowed_runtime_entrypoints",
]:
    add_construct_graph_cases(
        f"cg-node-{field}-scalar",
        lambda entry, field=field: entry["construct_graph"]["nodes"][0].__setitem__(
            field, "bad"
        ),
    )

for field in ["kind", "name", "type", "phase", "cardinality"]:
    add_construct_graph_cases(
        f"cg-interface-{field}-number",
        lambda entry, field=field: first_construct_graph_interface(entry).__setitem__(
            field, 1
        ),
    )

for field in [
    "port_id",
    "owner_node_id",
    "direction",
    "kind",
    "type",
    "phase",
    "resource_identity",
    "contract_version",
    "cardinality",
]:
    add_construct_graph_cases(
        f"cg-port-{field}-number",
        lambda entry, field=field: entry["construct_graph"]["ports"][0].__setitem__(
            field, 1
        ),
    )

for field in [
    "required_port_id",
    "provider_node_id",
    "provided_port_id",
    "resolution_reason",
    "resource_key",
]:
    add_construct_graph_cases(
        f"cg-edge-{field}-number",
        lambda entry, field=field: entry["construct_graph"]["edges"][0].__setitem__(
            field, 1
        ),
    )

add_construct_graph_cases(
    "cg-edge-order-index-string",
    lambda entry: entry["construct_graph"]["edges"][0].__setitem__(
        "order_index", "bad"
    ),
)
add_construct_graph_cases(
    "cg-edge-evidence-scalar",
    lambda entry: entry["construct_graph"]["edges"][0].__setitem__(
        "evidence", "bad"
    ),
)

for field in [
    "dependency_ref",
    "rule_name",
    "upstream_node_id",
    "predicate",
    "downstream_node_id",
]:
    add_construct_graph_cases(
        f"cg-dependency-{field}-number",
        lambda entry, field=field: entry["construct_graph"]["effect_dependencies"][
            0
        ].__setitem__(field, 1),
    )

add_construct_graph_cases(
    "cg-dependency-evidence-scalar",
    lambda entry: entry["construct_graph"]["effect_dependencies"][0].__setitem__(
        "evidence", "bad"
    ),
)

for field in [
    "graph_id",
    "accepted_program_digest",
    "lowerer_version",
    "package_lock_digest",
    "source_digest",
]:
    add_full_case(
        f"lir-root-{field}-number",
        lambda entry, field=field: entry["lowered_ir_report"].__setitem__(field, 1),
    )

for field in ["node_id", "lowering_class"]:
    add_full_case(
        f"lir-node-{field}-number",
        lambda entry, field=field: entry["lowered_ir_report"]["node_lowerings"][
            0
        ].__setitem__(field, 1),
    )

for field in ["required_port_id", "provided_port_id", "core_relation_ref"]:
    add_full_case(
        f"lir-edge-{field}-number",
        lambda entry, field=field: entry["lowered_ir_report"]["edge_lowerings"][
            0
        ].__setitem__(field, 1),
    )

for field in ["dependency_ref", "preserved_predicate"]:
    add_full_case(
        f"lir-dependency-{field}-number",
        lambda entry, field=field: entry["lowered_ir_report"]["dependency_lowerings"][
            0
        ].__setitem__(field, 1),
    )

for field in ["object_kind", "object_id", "owner_kind", "owner_ref", "runtime_entrypoint"]:
    add_full_case(
        f"lir-core-{field}-number",
        lambda entry, field=field: entry["lowered_ir_report"]["core_objects"][
            0
        ].__setitem__(field, 1),
    )

add_full_case(
    "lir-core-entrypoint-refs-scalar",
    lambda entry: entry["lowered_ir_report"]["core_objects"][0].__setitem__(
        "entrypoint_refs", "bad"
    ),
)
add_full_case("lir-core-entrypoint-ref-value-number", bad_entrypoint_value)
add_full_case(
    "lir-core-event-record-missing-refs",
    core_object_entrypoint_without_refs("event", "event_record"),
)
add_full_case(
    "lir-core-event-projection-missing-refs",
    core_object_entrypoint_without_refs("projection", "event_projection"),
)
add_full_case(
    "lir-core-diagnostic-record-missing-refs",
    core_object_entrypoint_without_refs("diagnostic", "diagnostic_record"),
)

catalog = tmp_dir / "platform-construct-catalog.json"
mismatches = []
for path in sorted(cases_dir.glob("*.json")):
    schema_ok = (
        subprocess.run(
            [
                "python3",
                "scripts/validate-artifact-reports.py",
                "--platform-catalog",
                str(catalog),
                str(path),
            ],
            cwd=root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        == 0
    )
    native_run = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "whipplescript", "--", "verify-report", str(path)],
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    native_ok = native_run.returncode == 0
    if schema_ok != native_ok:
        stderr = native_run.stderr.splitlines()
        mismatches.append(
            {
                "case": path.stem,
                "schema_ok": schema_ok,
                "native_ok": native_ok,
                "native_error": stderr[0] if stderr else "",
            }
        )

if mismatches:
    for mismatch in mismatches:
        print(
            "{case}: schema_ok={schema_ok} native_ok={native_ok} native_error={native_error}".format(
                **mismatch
            ),
            file=sys.stderr,
        )
    raise SystemExit(
        f"artifact admission differential found {len(mismatches)} mismatch(es)"
    )

print(
    f"artifact admission differential checked {len(list(cases_dir.glob('*.json')))} malformed reports"
)
PY
