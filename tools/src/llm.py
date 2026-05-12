"""Ollama LLM wrapper — classification d'emails via modèle local."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Dict, Set

import ollama
from src.config import load_config


VALID_CATEGORIES: Set[str] = {"travail", "notification", "newsletter", "associatif"}


class LLMAPIError(Exception):
    pass


class LLMConfigurationError(Exception):
    pass


def _get_llm_config() -> Dict[str, Any]:
    try:
        full_config = load_config(Path(__file__).parent.parent / "config" / "config.yaml")
        return full_config.get("ollama", {})
    except Exception as e:
        print(f"Warning: Failed to load Ollama config: {e}. Using defaults.", file=sys.stderr)
        return {}


def _resolve_url(llm_config: Dict[str, Any]) -> str:
    return llm_config.get("url", "http://localhost:11434")


def _resolve_model(llm_config: Dict[str, Any]) -> str:
    return llm_config.get("model", "qwen3:8b")


def _build_classification_prompt(subject: str, body_excerpt: str) -> str:
    return (
        "Classify the following email into exactly one of these categories:\n"
        "travail, notification, newsletter, associatif\n\n"
        f"Subject: {subject}\n"
        f"Body (excerpt): {body_excerpt}\n\n"
        "Reply with only the category name, nothing else."
    )


def _validate_category(raw_category: str) -> str:
    normalized = raw_category.strip().lower()
    return normalized if normalized in VALID_CATEGORIES else "travail"


def classify_email(subject: str, body_excerpt: str) -> str:
    """Classify an email using a local Ollama model.

    Returns one of: travail, notification, newsletter, associatif.
    Defaults to 'travail' on any error.
    """
    if not subject or not isinstance(subject, str):
        print(f"Warning: Invalid subject: {subject}. Using default category.", file=sys.stderr)
        return "travail"

    if not body_excerpt or not isinstance(body_excerpt, str):
        print(f"Warning: Invalid body excerpt: {body_excerpt}. Using default category.", file=sys.stderr)
        return "travail"

    try:
        llm_config = _get_llm_config()
        model = _resolve_model(llm_config)
        url = _resolve_url(llm_config)
        prompt = _build_classification_prompt(subject, body_excerpt)

        client = ollama.Client(host=url)
        response = client.chat(
            model=model,
            messages=[{"role": "user", "content": prompt}],
        )
        raw_category = response.message.content
        return _validate_category(raw_category)

    except Exception as e:
        print(f"Error: LLM classification failed: {e}. Using default category.", file=sys.stderr)
        return "travail"
