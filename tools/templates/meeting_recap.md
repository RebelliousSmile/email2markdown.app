---
title: Récap réunion — {{ start_date if end_date == start_date else start_date ~ ' → ' ~ end_date }}
generated: true
generated_at: {{ now }}
source_count: {{ count }}
email_type: meeting_recap
---

# Récap réunion — {{ start_date if end_date == start_date else start_date ~ ' → ' ~ end_date }}

## Participants

{% for sender in senders %}- {{ sender }}
{% endfor %}
{% if recipients %}
Destinataires additionnels :

{% for r in recipients %}- {{ r }}
{% endfor %}
{% endif %}

## Fil chronologique

{% for email in emails %}
### {{ email.date or '—' }} — {{ email.subject or email.filename }}

> De : {{ email.sender or email.get('from') or '—' }}

{{ email.body | trim }}

---
{% endfor %}

## Décisions et actions

> À remplir manuellement après relecture.

- [ ] …
- [ ] …
