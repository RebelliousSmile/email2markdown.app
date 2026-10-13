"""Read the ```email block of an exported note.

The frontmatter of a note describes the *note* (tags, proof of origin, linked
files). The message's own header (subject, from, to, cc, date) and body live in a
fenced block whose content is TOML, rendered by the Obsidian "Notebook" plugin.

Notes exported before that change still carry the header in the frontmatter, so
every reader goes through :func:`with_block`: frontmatter values win, the block
fills what the frontmatter does not have.
"""

from __future__ import annotations

import re
from typing import Any

try:  # Python >= 3.11
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - older interpreters
    import tomli as tomllib  # type: ignore[no-redef]


HEADER_KEYS = ("subject", "from", "to", "cc", "date")

_FENCE_OPEN = re.compile(r"^(?P<ticks>`{3,})email[ \t]*$")


def split_block(content: str) -> tuple[dict[str, Any] | None, str]:
    """Return ``(block, rest)``.

    ``block`` is the parsed TOML table of the first ```email block (None when the
    note has none, or when it is not valid TOML); ``rest`` is the note's text with
    that block removed (attachment links, anything the author added).
    """
    lines = content.splitlines(keepends=True)

    for start, line in enumerate(lines):
        opening = _FENCE_OPEN.match(line.rstrip("\r\n"))
        if not opening:
            continue

        ticks = len(opening.group("ticks"))
        for end in range(start + 1, len(lines)):
            stripped = lines[end].strip()
            if len(stripped) >= ticks and set(stripped) == {"`"}:
                source = "".join(lines[start + 1 : end])
                rest = "".join(lines[:start] + lines[end + 1 :])
                try:
                    return tomllib.loads(source), rest
                except tomllib.TOMLDecodeError:
                    return None, content
        break

    return None, content


def with_block(metadata: dict[str, Any], content: str) -> tuple[dict[str, Any], str]:
    """Complete ``metadata`` and ``content`` from the note's ```email block.

    Returns ``(metadata, body)``: the frontmatter dict with the header fields the
    block provides added when absent, and the message text (the block's body plus
    whatever follows it) in place of the raw note content.
    """
    block, rest = split_block(content)
    if block is None:
        return metadata, content

    merged = dict(metadata)
    for key in HEADER_KEYS:
        if key not in merged and key in block:
            value = block[key]
            # to / cc are lists in the block; the frontmatter used a single string.
            if key in ("to", "cc") and isinstance(value, list):
                value = ", ".join(str(item) for item in value)
            merged[key] = value

    body = str(block.get("body", "")).strip("\n")
    tail = rest.strip()
    return merged, f"{body}\n\n{tail}".strip("\n") if tail else body
