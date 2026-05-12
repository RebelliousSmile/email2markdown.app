"""Summarizer for 'associatif' email groups (association communications)."""

from __future__ import annotations

import os
import re


def _build_llm_client(config: dict):
    import anthropic

    api_key: str = (
        config.get("llm", {}).get("api_key", "")
        or os.environ.get("ANTHROPIC_API_KEY", "")
    )
    return anthropic.Anthropic(api_key=api_key)


def _extract_links(body: str) -> list[str]:
    """Extract all HTTP URLs from email body."""
    return re.findall(r"https?://\S+", body)


def _format_emails(group: list[dict]) -> str:
    parts = []
    for email in sorted(group, key=lambda e: e["date"]):
        date_str = email["date"].strftime("%Y-%m-%d %H:%M")
        parts.append(
            f"--- Email du {date_str} ---\n"
            f"De: {email.get('sender', '')}\n"
            f"Sujet: {email.get('subject', '')}\n\n"
            f"{email.get('body', '')}"
        )
    return "\n\n".join(parts)


def _collect_all_links(group: list[dict]) -> list[str]:
    seen: set[str] = set()
    links: list[str] = []
    for email in group:
        for url in _extract_links(email.get("body", "")):
            url_clean = url.rstrip(".,;)")
            if url_clean not in seen:
                seen.add(url_clean)
                links.append(url_clean)
    return links


def summarize(group: list[dict], config: dict) -> str:
    """Summarize an associatif email group — always generates a full aggregated page.

    Args:
        group: List of email dicts (any age).
        config: Project config dict with llm settings.

    Returns:
        LLM-generated aggregated page as a markdown string.
    """
    sorted_group = sorted(group, key=lambda e: e["date"])
    emails_text = _format_emails(sorted_group)
    all_links = _collect_all_links(sorted_group)

    links_section = ""
    if all_links:
        links_list = "\n".join(f"- {url}" for url in all_links)
        links_section = f"\n\nLiens trouvés dans les emails (à préserver impérativement) :\n{links_list}"

    first_sender = sorted_group[0].get("sender", "Association inconnue")

    prompt = (
        "Tu es un assistant de synthèse associative.\n"
        "Génère une page agrégée complète en markdown français pour cette association.\n\n"
        "La page doit contenir exactement ces sections :\n"
        "## Nom de l'association\n"
        f"(déduit de l'expéditeur : {first_sender})\n\n"
        "## Contexte\n"
        "(ce que fait cette association, déduit des emails)\n\n"
        "## Chronologie\n"
        "(liste des emails avec date et point clé de chaque message)\n\n"
        "## Situation actuelle\n"
        "(dernier état connu de l'association / de ses activités)\n\n"
        "## Liens importants\n"
        "(tous les liens HTTP présents dans les emails, sous forme de liste markdown)\n\n"
        f"Emails à analyser :{links_section}\n\n{emails_text}"
    )

    client = _build_llm_client(config)
    model: str = config.get("llm", {}).get("model", "claude-haiku-4-5")

    message = client.messages.create(
        model=model,
        max_tokens=1024,
        messages=[{"role": "user", "content": prompt}],
    )
    return message.content[0].text.strip()
