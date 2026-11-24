"""Preflight for the JSON/JSONC configuration surface exercised in the spike."""

import json
import re
from pathlib import Path

TOKENS = re.compile(r'("(?:[^"\\]|\\[\s\S])*")|//[^\r\n]*|/\*[\s\S]*?\*/|(,)(?=(?:\s|//[^\r\n]*|/\*[\s\S]*?\*/)*[}\]])')
CONFIG_FILES = (
    "opencode.json", "opencode.jsonc", ".opencode.json",
    ".opencode/opencode.json", ".opencode/opencode.jsonc",
)
RESERVED = {"provider", "providers", "model", "small_model",
            "enabled_providers", "disabled_providers"}


def parse(source):
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate field")
            result[key] = value
        return result

    return json.loads(TOKENS.sub(lambda match: match[1] or " ", source), object_pairs_hook=object_pairs)


def fields(value, path=()):
    if isinstance(value, dict):
        for key, child in value.items():
            field = (*path, key)
            # Inspect config itself and the direct fields of agents/commands.
            # A generic option/custom JSON property named "model" is not an
            # OpenCode selector.
            if key in RESERVED and (not path or
                                    len(path) == 2 and path[0] in {"agent", "agents", "command", "commands"}):
                yield ".".join(field)
            yield from fields(child, field)


def preflight(path: Path):
    if (path / ".opencode").is_symlink():
        raise ValueError(".opencode: symbolic link not supported by the spike")
    for relative in CONFIG_FILES:
        file = path / relative
        if file.is_symlink():
            raise ValueError(f"{relative}: symbolic link not supported by the spike")
        if not file.exists():
            continue
        try:
            data = parse(file.read_text())
            if not isinstance(data, dict):
                raise ValueError
        except (UnicodeError, ValueError) as error:
            raise ValueError(f"{relative}: invalid OpenCode config") from error
        for field in fields(data):
            raise ValueError(f"{relative}: {field} belongs in Factory settings")
