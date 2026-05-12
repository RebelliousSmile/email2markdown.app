"""Group emails that should produce one output file together."""

from __future__ import annotations

import re


# Prefixes to strip when normalizing travail subjects.
_THREAD_PREFIX = re.compile(
    r"^\s*(re|fwd|fw|tr)\s*:\s*",
    re.IGNORECASE,
)


def _normalize_subject(subject: str) -> str:
    """Normalize a subject by stripping thread prefixes, lowercasing, and removing whitespace.
    
    Args:
        subject (str): The subject to normalize.
        
    Returns:
        str: The normalized subject.
        
    Example:
        >>> _normalize_subject("Re: Fwd: Meeting")
        "meeting"
    """
    result = subject.strip().lower()
    # Iteratively strip leading prefixes (handles "Re: Fwd: subject").
    while True:
        new = _THREAD_PREFIX.sub("", result)
        if new == result:
            break
        result = new.strip()
    return result


def group_emails(emails: list[dict]) -> dict[str, list[dict]]:
    """Group emails by category into logical output groups.
    
    Each email must already have a 'category' key set.
    
    Args:
        emails (list[dict]): A list of email dictionaries to group.
        
    Returns:
        dict[str, list[dict]]: A dictionary where:
            - key: group_id string formatted as "{category}::{key}"
            - value: list of emails belonging to that group
        
    Example:
        >>> emails = [{"category": "travail", "subject": "Re: Meeting"}]
        >>> group_emails(emails)
        {"travail::meeting": [{"category": "travail", "subject": "Re: Meeting"}]}
    """
    groups: dict[str, list[dict]] = {}

    for email in emails:
        category: str = email.get("category", "travail")
        sender: str = email.get("sender", "")
        subject: str = email.get("subject", "")

        if category == "travail":
            key = _normalize_subject(subject)
        elif category in ("notification", "newsletter", "associatif"):
            key = sender
        else:
            key = sender

        group_id = f"{category}::{key}"
        groups.setdefault(group_id, []).append(email)

    return groups
