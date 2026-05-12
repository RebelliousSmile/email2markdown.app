"""Unit tests for folder_classifier coldstart improvements:
- locked_levels short-circuit when len(locked) == 3
- reuse-first prompt with known_classes
- soft mapping retry when first non-locked level mismatches existing vocabulary

Ollama is mocked everywhere — no network call.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest

TOOLS_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(TOOLS_ROOT))

from src import folder_classifier  # noqa: E402


@pytest.fixture
def empty_data_dir(tmp_path: Path) -> Path:
    (tmp_path / "data").mkdir()
    return tmp_path / "data"


def _config(data_dir: Path, min_samples: int = 20) -> dict:
    return {
        "classify": {
            "data_dir": str(data_dir),
            "confidence_threshold": 0.75,
            "min_samples_before_ml": min_samples,
            "cold_start_model": "qwen3:8b",
        }
    }


def test_locked_levels_full_shortcircuits_without_llm(empty_data_dir: Path) -> None:
    email = {"subject": "Hello", "sender": "x@y.z", "email_type": "direct"}
    config = _config(empty_data_dir)

    with patch("ollama.chat") as mock_chat:
        path = folder_classifier.propose_path(
            email, config, locked_levels=["A", "B", "C"]
        )

    assert path == "A/B/C"
    mock_chat.assert_not_called()


def test_reuse_first_known_class_returned(empty_data_dir: Path) -> None:
    (empty_data_dir / "known_classes.json").write_text(
        json.dumps(["Travail/Projets/ClientX"]), encoding="utf-8"
    )

    email = {"subject": "RE: ClientX prod", "sender": "ops@clientx.com", "email_type": "direct"}
    config = _config(empty_data_dir)

    fake = MagicMock()
    fake.return_value = {"message": {"content": "Travail/Projets/ClientX"}}
    with patch("ollama.chat", fake):
        path = folder_classifier.propose_path(email, config)

    assert path == "Travail/Projets/ClientX"


def test_soft_mapping_retry_when_first_level_unknown(empty_data_dir: Path) -> None:
    (empty_data_dir / "known_classes.json").write_text(
        json.dumps(["Travail/Projets/X", "Travail/Projets/Y"]), encoding="utf-8"
    )

    email = {"subject": "Quelque chose", "sender": "a@b.c", "email_type": "direct"}
    config = _config(empty_data_dir)

    responses = iter([
        {"message": {"content": "Pro/Projets/Z"}},  # first try — "Pro" not in known
        {"message": {"content": "Travail/Projets/Z"}},  # retry — picks existing top
    ])

    def chat_stub(*args, **kwargs):
        return next(responses)

    with patch("ollama.chat", side_effect=chat_stub):
        path = folder_classifier.propose_path(email, config)

    assert path.split("/")[0] == "Travail"
