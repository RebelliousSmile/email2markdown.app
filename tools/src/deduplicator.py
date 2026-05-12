"""Deduplicate emails that share the same subject_hash and sender."""

from __future__ import annotations


def deduplicate(emails: list[dict]) -> list[dict]:
    """Return a deduplicated list of emails.

    Grouping key: (subject_hash, sender).
    Emails whose subject_hash is None are never deduplicated — each is kept as-is.
    When multiple emails share the same key, only the earliest by date is kept.
    """
    seen: dict[tuple[str, str], dict] = {}
    unique: list[dict] = []

    for email in emails:
        subject_hash: str | None = email.get("subject_hash")
        sender: str = email.get("sender", "")

        if subject_hash is None:
            # Treat as unique — never merge.
            unique.append(email)
            continue

        key = (subject_hash, sender)
        if key not in seen:
            seen[key] = email
        else:
            # Keep the earliest email by date.
            if email["date"] < seen[key]["date"]:
                seen[key] = email

    # Preserve original order: emit grouped emails in the order their first
    # representative appeared, interleaved with None-hash emails.
    result: list[dict] = []
    emitted_keys: set[tuple[str, str]] = set()

    for email in emails:
        subject_hash = email.get("subject_hash")
        sender = email.get("sender", "")

        if subject_hash is None:
            result.append(email)
        else:
            key = (subject_hash, sender)
            if key not in emitted_keys:
                result.append(seen[key])
                emitted_keys.add(key)

    return result
