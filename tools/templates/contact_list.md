---
title: Contacts — {{ start_date }} → {{ end_date }}
generated: true
source_count: {{ emails | length }}
---

# Contacts touchés ({{ start_date }} → {{ end_date }})

{% set seen = namespace(addrs=[]) %}
{% for email in emails %}
{% for addr in (email.to or []) %}
{% if addr and addr not in seen.addrs %}
{% set _ = seen.addrs.append(addr) %}
- **{{ addr }}** — premier contact : {{ email.date }} (sujet : *{{ email.subject or email.filename }}*)
{% endif %}
{% endfor %}
{% endfor %}

_Total destinataires uniques : {{ seen.addrs | length }}_
