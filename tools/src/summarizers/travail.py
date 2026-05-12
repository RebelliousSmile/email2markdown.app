"""Summarizer for 'travail' email groups (threaded work conversations)."""

from __future__ import annotations

import os

from src.age import age_in_days


def _build_llm_client(config: dict):
    import anthropic

    api_key: str = (
        config.get("llm", {}).get("api_key", "")
        or os.environ.get("ANTHROPIC_API_KEY", "")
    )
    return anthropic.Anthropic(api_key=api_key)


def _format_emails(group: list[dict]) -> str:
    parts = []
    for email in group:
        date_str = email["date"].strftime("%Y-%m-%d %H:%M")
        parts.append(
            f"--- Email du {date_str} ---\n"
            f"De: {email.get('sender', '')}\n"
            f"Sujet: {email.get('subject', '')}\n\n"
            f"{email.get('body', '')}"
        )
    return "\n\n".join(parts)


def summarize(group: list[dict], config: dict) -> str:
    """Summarize a travail email group using the LLM.

    Args:
        group: List of email dicts sorted by date ascending (same thread).
        config: Project config dict with thresholds and llm settings.

    Returns:
        LLM-generated summary as a string.
    """
    is_recent = age_in_days(group[-1]["date"]) <= config["thresholds"]["travail_days"]
    emails_text = _format_emails(group)

    if is_recent:
        prompt = (
            "Tu es un assistant de gestion d'emails professionnel.\n"
            "Analyse ce fil de discussion et génère une fiche complète en markdown français.\n\n"
            "La fiche doit contenir exactement ces sections :\n"
            "## Chronologie\n"
            "(liste des emails avec date et point clé de chaque message)\n\n"
            "## Statut actuel\n"
            "(état du dossier après le dernier message)\n\n"
            "## Actions en suspens\n"
            "(liste des actions restant à faire, ou 'Aucune' si terminé)\n\n"
            "## Participants\n"
            "(liste des intervenants)\n\n"
            f"Emails à analyser :\n\n{emails_text}"
        )
    else:
        prompt = (
            "Tu es un assistant de gestion d'emails professionnel.\n"
            "Résume ce fil de discussion en 2-3 lignes maximum en français.\n"
            "Indique : le statut final du dossier et la date du dernier message.\n\n"
            f"Emails à analyser :\n\n{emails_text}"
        )

    client = _build_llm_client(config)
    model: str = config.get("llm", {}).get("model", "claude-haiku-4-5")

    message = client.messages.create(
        model=model,
        max_tokens=1024,
        messages=[{"role": "user", "content": prompt}],
    )
    return message.content[0].text.strip()
