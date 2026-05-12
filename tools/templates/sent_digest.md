---
title: Digest des envois {{ start_date }} → {{ end_date }}
generated: true
generated_at: {{ now }}
source_count: {{ count }}
email_type: digest
---

# Digest des envois {{ start_date }} → {{ end_date }}

> {{ count }} message(s) envoyé(s) sur la période, vers {{ recipients | length }} destinataire(s) unique(s).

## Aperçu

| Date | Sujet | Destinataires |
|------|-------|---------------|
{% for email in emails %}| {{ email.date or '—' }} | {{ email.subject or email.filename }} | {{ email.to | join(', ') if email.to else '—' }} |
{% endfor %}

## Détail

{% for email in emails %}
### {{ email.subject or email.filename }}

- **Envoyé le** : {{ email.date or '—' }}
- **À** : {{ email.to | join(', ') if email.to else '—' }}
{% if email.cc %}- **Cc** : {{ email.cc | join(', ') }}
{% endif %}

{{ email.body | trim }}

---
{% endfor %}
