---
title: Activité {{ start_date }} → {{ end_date }}
generated: true
source_count: {{ emails | length }}
---

# Activité {{ start_date }} → {{ end_date }}

{% for email in emails %}
## {{ email.subject or email.filename }}

- **Date** : {{ email.date }}
- **De** : {{ email.sender or email.get('from') or '—' }}
- **À** : {{ email.to | join(', ') if email.to else '—' }}

{{ email.body | trim }}

---
{% endfor %}
