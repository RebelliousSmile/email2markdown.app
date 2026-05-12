"""Categorize emails into: travail | notification | newsletter | associatif."""

from __future__ import annotations

import re

from src.llm import classify_email

# Patterns that indicate a notification sender (domain or local part).
_NOTIFICATION_PATTERNS = re.compile(
    r"(noreply|no-reply|no_reply|donotreply|do-not-reply|"
    r"notification|notifications|notify|alert|alerts|"
    r"mailer-daemon|postmaster|bounce|bounces|"
    r"firebase|sendgrid|mailchimp|mandrill|sparkpost|"
    r"automated|automatic|system|info@)",
    re.IGNORECASE,
)

# Keywords that indicate associatif content (French + English).
_ASSOCIATIF_KEYWORDS = re.compile(
    r"\b(don|dons|donation|donations|cotisation|cotisations|"
    r"association|associations|bénévol|benévol|benevolat|bénévolat|"
    r"action|actions|recours|petition|pétition|mobilisation|mobilization|"
    r"campagne|campaign|solidarit|solidarity|militant|militante|"
    r"adhérent|adherent|adhésion|adhesion|syndicat|syndic|"
    r"cause|causes|appel|appels)\b",
    re.IGNORECASE,
)


def _is_notification_sender(sender: str) -> bool:
    """Check if the sender address looks like an automated notification.
    
    Args:
        sender (str): The sender's email address.
        
    Returns:
        bool: True if the sender address matches notification patterns, False otherwise.
        
    Example:
        >>> _is_notification_sender("noreply@example.com")
        True
        >>> _is_notification_sender("john.doe@example.com")
        False
    """
    return bool(_NOTIFICATION_PATTERNS.search(sender))


def _body_has_associatif_keywords(body: str) -> bool:
    """Check if the body contains associatif or civic content.
    
    Args:
        body (str): The body content of the email.
        
    Returns:
        bool: True if the body contains associatif keywords, False otherwise.
        
    Example:
        >>> _body_has_associatif_keywords("Faites un don pour notre association")
        True
        >>> _body_has_associatif_keywords("Réunion de travail demain")
        False
    """
    return bool(_ASSOCIATIF_KEYWORDS.search(body))


def categorize(email: dict) -> str:
    """Categorize an email based on its type and content.
    
    Args:
        email (dict): A dictionary containing email data (e.g., sender, body, email_type).
        
    Returns:
        str: The category of the email (travail, notification, newsletter, or associatif).
        
    Example:
        >>> email = {"email_type": "mailing_list", "body": "Faites un don"}
        >>> categorize(email)
        "associatif"
    """
    email_type: str | None = email.get("email_type")
    sender: str = email.get("sender", "")
    body: str = email.get("body", "")
    subject: str = email.get("subject", "")

    if email_type == "mailing_list":
        if _body_has_associatif_keywords(body):
            return "associatif"
        return "newsletter"

    if email_type == "group":
        return "travail"

    if email_type == "direct":
        if _is_notification_sender(sender):
            return "notification"
        return "travail"

    # email_type is None — fall back to LLM
    return classify_email(subject, body[:200])
