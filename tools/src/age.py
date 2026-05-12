from datetime import datetime, timezone


def age_in_days(email_date: datetime) -> int:
    """Return number of days since email_date."""
    now = datetime.now(timezone.utc)
    return (now - email_date).days
