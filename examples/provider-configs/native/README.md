# Native Provider Config Examples

These files are release-package examples for native Codex, Claude, and Pi
bindings. They are safe to publish because credential fields are references,
not secret values.

Validate from a checkout or release artifact with:

```sh
whip --json doctor --provider-config examples/provider-configs/native/native.example.json
scripts/check-native-provider-configs.sh
```

For strict release validation, point `WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS` at
this file or at an environment-specific copy:

```sh
WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIGS=examples/provider-configs/native/native.example.json \
WHIPPLESCRIPT_NATIVE_PROVIDER_CONFIG_STRICT=1 \
scripts/check-native-provider-configs.sh
```

Before using these bindings for real provider turns, replace placeholder
credential refs, workspace policy, model names, profile ids, and health checks
with environment-specific values.
