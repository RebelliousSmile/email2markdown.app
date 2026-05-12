"""Email parser module.

This module provides functionality to parse email files with YAML frontmatter
and extract relevant data into a structured format.
"""

from datetime import datetime
from pathlib import Path
from typing import TypedDict

import frontmatter


VALID_EMAIL_TYPES = {"direct", "group", "mailing_list"}


class EmailData(TypedDict):
    """Typed dictionary representing the structure of an email.
    
    Attributes:
        sender (str): The sender's email address.
        to (str): The recipient's email address.
        date (datetime): The date and time the email was sent.
        subject (str): The subject of the email.
        subject_hash (str | None): The hash of the subject, if available.
        email_type (str | None): The type of the email (e.g., direct, group, mailing_list).
        body (str): The body content of the email.
        filepath (Path): The path to the email file.
    """
    sender: str
    to: str
    date: datetime
    subject: str
    subject_hash: str | None
    email_type: str | None
    body: str
    filepath: Path


def parse_email(filepath: Path) -> EmailData:
    """Parse an email file with YAML frontmatter and extract relevant data.
    
    Args:
        filepath (Path): The path to the email file to parse.
        
    Returns:
        EmailData: A dictionary containing the parsed email data.
        
    Raises:
        ValueError: If required fields are missing or unparseable.
        
    Example:
        >>> email_data = parse_email(Path("email.md"))
        >>> print(email_data["subject"])
        "Test Subject"
    """
    """Parse a .md file with YAML frontmatter into an EmailData dict.

    Raises ValueError if required fields are missing or unparseable.
    """
    try:
        post = frontmatter.load(str(filepath))
    except Exception as exc:
        raise ValueError(f"Failed to parse frontmatter in {filepath.name}: {exc}") from exc

    metadata = post.metadata

    # Required fields
    raw_sender = metadata.get("from")
    if not raw_sender:
        raise ValueError(f"Missing required field 'from' in {filepath.name}")

    raw_to = metadata.get("to")
    if not raw_to:
        raise ValueError(f"Missing required field 'to' in {filepath.name}")

    raw_date = metadata.get("date")
    if not raw_date:
        raise ValueError(f"Missing required field 'date' in {filepath.name}")

    raw_subject = metadata.get("subject")
    if raw_subject is None:
        raise ValueError(f"Missing required field 'subject' in {filepath.name}")

    # Parse date — accept datetime objects (PyYAML may auto-parse) or ISO strings
    if isinstance(raw_date, datetime):
        parsed_date = raw_date
    else:
        try:
            parsed_date = datetime.fromisoformat(str(raw_date))
        except ValueError as exc:
            raise ValueError(
                f"Unparseable 'date' field in {filepath.name}: {raw_date!r}"
            ) from exc

    if parsed_date.tzinfo is None:
        raise ValueError(
            f"'date' field in {filepath.name} is not timezone-aware: {raw_date!r}"
        )

    # Optional fields
    subject_hash: str | None = metadata.get("subject_hash")
    if subject_hash is not None:
        subject_hash = str(subject_hash)

    raw_email_type = metadata.get("email_type")
    email_type: str | None = None
    if raw_email_type is not None:
        email_type = str(raw_email_type)
        if email_type not in VALID_EMAIL_TYPES:
            raise ValueError(
                f"Invalid 'email_type' value {email_type!r} in {filepath.name}. "
                f"Expected one of: {', '.join(sorted(VALID_EMAIL_TYPES))}"
            )

    return EmailData(
        sender=str(raw_sender),
        to=str(raw_to),
        date=parsed_date,
        subject=str(raw_subject),
        subject_hash=subject_hash,
        email_type=email_type,
        body=post.content,
        filepath=filepath,
    )
