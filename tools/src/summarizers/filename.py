"""Generate output filenames for summarized email groups."""

from __future__ import annotations

import os
import re
import unicodedata


def _slugify(text: str, max_chars: int = 60) -> str:
    """Convert text to a URL-safe slug using only stdlib.

    Steps:
    1. Normalize to NFKD to decompose accented characters.
    2. Encode to ASCII ignoring non-ASCII bytes (strips accents).
    3. Lowercase.
    4. Replace non-alphanumeric characters with hyphens.
    5. Collapse consecutive hyphens.
    6. Strip leading/trailing hyphens.
    7. Truncate to max_chars.
    """
    normalized = unicodedata.normalize("NFKD", text)
    ascii_bytes = normalized.encode("ascii", "ignore")
    ascii_str = ascii_bytes.decode("ascii")
    lowered = ascii_str.lower()
    slugged = re.sub(r"[^a-z0-9]+", "-", lowered)
    slugged = slugged.strip("-")
    return slugged[:max_chars].rstrip("-")


def _fallback_filename(group: list[dict], category: str) -> str:
    date_str = group[0]["date"].strftime("%Y%m%d") if group else "unknown"
    return f"group-{category}-{date_str}.md"


def make_filename(group: list[dict], category: str, model: str, url: str) -> str:
    """Generate a filesystem-safe markdown filename for the summarized group.

    Args:
        group: List of email dicts.
        category: Email category string (travail, notification, newsletter, associatif).
        model: Ollama model name.
        url: Ollama server URL.

    Returns:
        A filename string ending with '.md'. Never empty.
    """
    if not group:
        return _fallback_filename(group, category)

    # Single email or travail group: slugify subject of first email
    if len(group) == 1 or category == "travail":
        subject = group[0].get("subject", "").strip()
        if subject:
            slug = _slugify(subject, max_chars=60)
            if slug:
                return f"{slug}.md"
        return _fallback_filename(group, category)

    # Multi-email groups for newsletter/notification/associatif:
    # Check whether all senders are the same
    senders = {email.get("sender", "") for email in group}
    subjects = [email.get("subject", "").strip() for email in group if email.get("subject", "").strip()]

    if len(senders) == 1 and subjects:
        # Single sender — slugify first subject
        slug = _slugify(subjects[0], max_chars=60)
        if slug:
            return f"{slug}.md"

    # Multiple senders or no clear subject — ask LLM for a short slug
    if subjects:
        subjects_text = "\n".join(f"- {s}" for s in subjects[:10])
        prompt = (
            "Generate a short slug (maximum 8 words, lowercase, hyphen-separated) "
            "that summarizes these email subjects. "
            "Reply with only the slug, no punctuation, no file extension.\n\n"
            f"Subjects:\n{subjects_text}"
        )
        try:
            import ollama
            client = ollama.Client(host=url)
            response = client.chat(
                model=model,
                messages=[{"role": "user", "content": prompt}],
            )
            raw = response.message.content.strip()
            slug = _slugify(raw, max_chars=60)
            if slug:
                return f"{slug}.md"
        except Exception:
            pass

    return _fallback_filename(group, category)
