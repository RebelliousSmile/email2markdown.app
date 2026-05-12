"""Summarizer for 'notification' email groups (transactional/alert emails)."""

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


def summarize(group: list[dict], config: dict) -> str:
    """Summarize a notification email group using the LLM.

    Args:
        group: List of email dicts (may contain 1 email after dedup).
        config: Project config dict with thresholds and llm settings.

    Returns:
        LLM-generated summary as a string.
    """
    is_recent = age_in_days(group[0]["date"]) <= config["thresholds"]["notification_days"]

    email = group[0]
    date_str = email["date"].strftime("%Y-%m-%d %H:%M")
    subject = email.get("subject", "")
    body = email.get("body", "")
    sender = email.get("sender", "")

    email_block = (
        f"De: {sender}\n"
        f"Date: {date_str}\n"
        f"Sujet: {subject}\n\n"
        f"{body}"
    )

    if is_recent:
        prompt = (
            "Tu es un assistant de gestion d'emails.\n"
            "Analyse cette notification et fournis en français :\n"
            "1. L'action déclenchée (en une phrase claire)\n"
            "2. Les liens pertinents extraits du corps du message (URL utiles pour agir)\n\n"
            f"Email :\n\n{email_block}"
        )
    else:
        prompt = (
            "Tu es un assistant de gestion d'emails.\n"
            "Résume cette notification en une seule ligne en français.\n"
            "Format : [Action déclenchée] — [date]\n\n"
            f"Email :\n\n{email_block}"
        )

    client = _build_llm_client(config)
    model: str = config.get("llm", {}).get("model", "claude-haiku-4-5")

    message = client.messages.create(
        model=model,
        max_tokens=256,
        messages=[{"role": "user", "content": prompt}],
    )
    return message.content[0].text.strip()
