"""Summarizer for 'newsletter' email groups."""

from __future__ import annotations

import os
import re

from src.age import age_in_days

# Keywords that typically appear near a view-online URL
_VIEW_ONLINE_KEYWORDS = re.compile(
    r"(voir en ligne|view online|view in browser|lire en ligne)",
    re.IGNORECASE,
)

_URL_PATTERN = re.compile(r"https?://\S+")


def _extract_view_online_url(body: str) -> str | None:
    """Return the first view-online URL found within 200 chars of a keyword."""
    for match in _VIEW_ONLINE_KEYWORDS.finditer(body):
        start = max(0, match.start() - 200)
        end = min(len(body), match.end() + 200)
        window = body[start:end]
        url_match = _URL_PATTERN.search(window)
        if url_match:
            # Strip trailing punctuation that might have been captured
            url = url_match.group(0).rstrip(".,;)")
            return url
    return None


def _collect_view_online_urls(group: list[dict]) -> list[str]:
    """Return deduplicated list of view-online URLs from all emails in group."""
    seen: set[str] = set()
    urls: list[str] = []
    for email in group:
        url = _extract_view_online_url(email.get("body", ""))
        if url and url not in seen:
            seen.add(url)
            urls.append(url)
    return urls


def _build_llm_client(config: dict):
    import anthropic

    api_key: str = (
        config.get("llm", {}).get("api_key", "")
        or os.environ.get("ANTHROPIC_API_KEY", "")
    )
    return anthropic.Anthropic(api_key=api_key)


def summarize(group: list[dict], config: dict) -> str | None:
    """Summarize a newsletter email group.

    Args:
        group: List of email dicts.
        config: Project config dict with thresholds and llm settings.

    Returns:
        Markdown string, or None if old with no view-online URL.
    """
    is_recent = age_in_days(group[0]["date"]) <= config["thresholds"]["newsletter_days"]
    view_online_urls = _collect_view_online_urls(group)

    if not is_recent:
        if not view_online_urls:
            return None
        # Old but has view-online URLs — return markdown links only
        lines = ["**Newsletter archivée — liens pour consulter en ligne :**", ""]
        for url in view_online_urls:
            lines.append(f"- [Voir en ligne]({url})")
        return "\n".join(lines)

    # Recent — use LLM for bullet points summary
    email = group[0]
    subject = email.get("subject", "")
    body = email.get("body", "")
    sender = email.get("sender", "")

    url_section = ""
    if view_online_urls:
        url_links = " | ".join(f"[Voir en ligne]({u})" for u in view_online_urls)
        url_section = f"\n\nLien(s) : {url_links}"

    prompt = (
        "Tu es un assistant de synthèse de newsletters.\n"
        "Génère en français une liste de points bullet des éléments notables de cette newsletter.\n"
        "Sois concis, 3-7 points maximum. Format markdown (tirets).\n\n"
        f"Expéditeur: {sender}\n"
        f"Sujet: {subject}\n\n"
        f"{body}"
    )

    client = _build_llm_client(config)
    model: str = config.get("llm", {}).get("model", "claude-haiku-4-5")

    message = client.messages.create(
        model=model,
        max_tokens=512,
        messages=[{"role": "user", "content": prompt}],
    )
    result = message.content[0].text.strip()
    return result + url_section
