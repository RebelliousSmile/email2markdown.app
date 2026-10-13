"""Unit tests for src.email_block: header and body of the ```email block."""
from __future__ import annotations

import sys
from pathlib import Path

TOOLS_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(TOOLS_ROOT))

from src.email_block import split_block, with_block  # noqa: E402

BLOCK = (
    "```email\n"
    'subject = "Devis"\n'
    'from = "Alice <alice@example.com>"\n'
    'to = ["Bob <bob@example.com>", "Carol <carol@example.com>"]\n'
    'cc = ["Dan <dan@example.com>"]\n'
    "date = 2026-07-02T10:18:19+00:00\n"
    "body = '''\n"
    "Bonjour,\n\nMerci.\n"
    "'''\n"
    "```\n"
)
ATTACHMENTS = "\n## Pièces jointes\n\n- [a.pdf](a.pdf)\n"


def test_with_block_completes_header_and_body():
    meta, body = with_block({"tags": ["email"]}, BLOCK + ATTACHMENTS)

    assert meta["from"] == "Alice <alice@example.com>"
    assert meta["to"] == "Bob <bob@example.com>, Carol <carol@example.com>"
    assert meta["cc"] == "Dan <dan@example.com>"
    assert meta["subject"] == "Devis"
    assert body.startswith("Bonjour,")
    assert "Pièces jointes" in body
    assert "```" not in body


def test_with_block_frontmatter_wins_over_block():
    meta, _ = with_block({"subject": "Ancien sujet"}, BLOCK)

    assert meta["subject"] == "Ancien sujet"
    assert meta["from"] == "Alice <alice@example.com>"


def test_with_block_invalid_toml_leaves_note_unchanged():
    content = "```email\nsubject = \n```\ntexte"

    meta, body = with_block({"tags": []}, content)

    assert (meta, body) == ({"tags": []}, content)


def test_with_block_note_without_block_is_unchanged():
    content = "Ancienne note\n\nCorps."

    meta, body = with_block({"from": "a@b.c"}, content)

    assert (meta, body) == ({"from": "a@b.c"}, content)


def test_split_block_longer_fence_survives_inner_code_fence():
    content = (
        "````email\nsubject = \"x\"\nbody = '''\n```\ncode\n```\n'''\n````\n"
    )

    block, rest = split_block(content)

    assert block is not None and "```\ncode\n```" in block["body"]
    assert rest.strip() == ""
