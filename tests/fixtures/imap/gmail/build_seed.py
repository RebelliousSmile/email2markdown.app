"""Generate the Gmail seed corpus: 12 messages in 4 conversations.

Deterministic, standard library only. Run from anywhere with:

    python -I tests/fixtures/imap/gmail/build_seed.py [output_dir]

Default output: tests/fixtures/imap/seed/gmail/. Every byte depends only on
this file: fixed boundaries, fixed dates, seeded noise images. The run label
is `fixture` and the addresses are fictitious; tests/common/gmail_seed.rs
substitutes them for a real run (phase 5 of the campaign plan).
"""

import base64
import quopri
import random
import struct
import sys
import textwrap
import zlib
from pathlib import Path

RUN = "fixture"
TAG = f"[email-to-markdown seed {RUN}]"
ALICE = "Alice Martin <alice@example.com>"
BOB = "Bob Durand <bob@example.com>"
ALICE_ATTR = "Alice Martin &lt;<a href=\"mailto:alice@example.com\">alice@example.com</a>&gt;"
BOB_ATTR = "Bob Durand &lt;<a href=\"mailto:bob@example.com\">bob@example.com</a>&gt;"


def mid(conv, msg):
    return f"<seed-c{conv}-m{msg}.{RUN}@email-to-markdown.test>"


# --------------------------------------------------------------------------
# Binary payloads: valid files of a target size, deterministic.
# --------------------------------------------------------------------------


def png(target, seed, width=256):
    """Noise PNG of roughly `target` bytes (noise does not compress)."""
    rng = random.Random(seed)
    row = width * 3 + 1
    height = max(1, target // row)
    raw = b"".join(b"\x00" + rng.randbytes(width * 3) for _ in range(height))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def _segment(marker, data):
    return b"\xff" + marker + struct.pack(">H", len(data) + 2) + data


def _comments(target, current, seed):
    """JPEG COM segments filling up to `target` bytes."""
    rng = random.Random(seed)
    out = b""
    while current + len(out) + 4 < target:
        size = min(65000, target - current - len(out) - 4)
        out += _segment(b"\xfe", bytes(rng.randrange(32, 127) for _ in range(size)))
    return out


def jpeg(target, seed):
    """Flat grey 8x8 baseline JPEG, padded with comments to `target` bytes."""
    head = b"\xff\xd8" + _segment(b"\xe0", b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00")
    tail = (
        _segment(b"\xdb", b"\x00" + b"\x01" * 64)
        + _segment(b"\xc0", b"\x08\x00\x08\x00\x08\x01\x01\x11\x00")
        # One-code Huffman tables: DC category 0 and AC end-of-block, both "0".
        + _segment(b"\xc4", b"\x00" + b"\x01" + b"\x00" * 15 + b"\x00")
        + _segment(b"\xc4", b"\x10" + b"\x01" + b"\x00" * 15 + b"\x00")
        + _segment(b"\xda", b"\x01\x01\x00\x00\x3f\x00")
        + b"\x3f"
        + b"\xff\xd9"
    )
    return head + _comments(target, len(head) + len(tail), seed) + tail


def gif(target, seed):
    """Animated 1x1 GIF (two frames, looping), padded with comment blocks."""
    rng = random.Random(seed)
    head = b"GIF89a" + struct.pack("<HHBBB", 1, 1, 0x80, 0, 0) + b"\x00\x00\x00\xff\xff\xff"
    loop = b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00"

    def frame(index):
        gce = b"\x21\xf9\x04\x00\x32\x00\x00\x00"
        desc = b"\x2c" + struct.pack("<HHHHB", 0, 0, 1, 1, 0)
        data = b"\x02\x02" + bytes([0x44 if index == 0 else 0x4C, 0x01]) + b"\x00"
        return gce + desc + data

    body = head + loop + frame(0) + frame(1)
    comment = b""
    room = target - len(body) - 1 - 3
    if room > 0:
        comment = b"\x21\xfe"
        while room > 0:
            size = min(255, room - 1) if room > 1 else 1
            comment += bytes([size]) + bytes(rng.randrange(32, 127) for _ in range(size))
            room -= size + 1
        comment += b"\x00"
    return body + comment + b"\x3b"


def pdf(title):
    """One-page PDF with a line of text."""
    text = title.encode("latin-1")
    stream = b"BT /F1 18 Tf 72 720 Td (" + text + b") Tj ET"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    out = b"%PDF-1.4\n"
    offsets = []
    for number, obj in enumerate(objects, start=1):
        offsets.append(len(out))
        out += str(number).encode() + b" 0 obj\n" + obj + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 " + str(len(objects) + 1).encode() + b"\n0000000000 65535 f \n"
    for offset in offsets:
        out += f"{offset:010d} 00000 n \n".encode()
    out += (
        b"trailer\n<< /Size " + str(len(objects) + 1).encode() + b" /Root 1 0 R >>\n"
        b"startxref\n" + str(xref).encode() + b"\n%%EOF\n"
    )
    return out


# --------------------------------------------------------------------------
# MIME assembly, Gmail style. Built with "\n", converted to CRLF at the end.
# --------------------------------------------------------------------------


def encode_subject(subject):
    """Keep `Re: [email-to-markdown seed <run>]` readable, Q-encode the rest."""
    if subject.isascii():
        return subject
    head, sep, tail = subject.partition(TAG + " ")
    encoded = "".join(
        ch if ch.isascii() and (ch.isalnum() or ch in "-.'") else
        "_" if ch == " " else "".join(f"={b:02X}" for b in ch.encode("utf-8"))
        for ch in tail
    )
    return f"{head}{sep}=?UTF-8?Q?{encoded}?="


def qp(text):
    return quopri.encodestring(text.encode("utf-8")).decode("ascii")


def b64(data):
    encoded = base64.b64encode(data).decode("ascii")
    return "\n".join(encoded[i : i + 76] for i in range(0, len(encoded), 76))


def text_part(subtype, text, params=""):
    return (
        f'Content-Type: text/{subtype}; charset="UTF-8"{params}\n'
        "Content-Transfer-Encoding: quoted-printable\n\n" + qp(text)
    )


def image_part(mimetype, name, data, cid=None, disposition="inline", filename_header=None):
    lines = [f'Content-Type: {mimetype}; name="{name}"']
    if disposition:
        lines.append(filename_header or f'Content-Disposition: {disposition}; filename="{name}"')
    lines.append("Content-Transfer-Encoding: base64")
    if cid:
        lines.append(f"Content-ID: <{cid}>")
        lines.append(f"X-Attachment-Id: {cid}")
    return "\n".join(lines) + "\n\n" + b64(data)


def multipart(subtype, boundary, parts):
    body = f'Content-Type: multipart/{subtype}; boundary="{boundary}"\n\n'
    for part in parts:
        body += f"--{boundary}\n{part.rstrip(chr(10))}\n"
    return body + f"--{boundary}--\n"


def message(conv, msg, date, sender, to, subject, body, reply_to=None, references=()):
    headers = ["MIME-Version: 1.0", f"Date: {date}"]
    if reply_to:
        refs = " ".join(mid(c, m) for c, m in references)
        headers += [f"References: {refs}", f"In-Reply-To: {mid(*reply_to)}"]
    headers += [
        f"Message-ID: {mid(conv, msg)}",
        f"Subject: {encode_subject(subject)}",
        f"From: {sender}",
        f"To: {to}",
    ]
    raw = "\n".join(headers) + "\n" + body
    assert "\r" not in raw
    return raw.replace("\n", "\r\n").encode("ascii")


def wrap(text, width=76):
    """Gmail-like hard wrap of each paragraph, blank lines kept."""
    return "\n".join(
        "\n".join(textwrap.wrap(line, width)) if line.strip() else line for line in text.split("\n")
    )


def flowed(text, width=76):
    """RFC 3676 format=flowed; delsp=yes: each soft break keeps the original
    space and adds the marker space that the reader deletes."""
    out = []
    for line in text.split("\n"):
        if line == "-- " or len(line) <= width:
            out.append(line)
            continue
        pieces = textwrap.wrap(line, width - 2, break_on_hyphens=False)
        out += [piece + "  " for piece in pieces[:-1]] + [pieces[-1]]
    return "\n".join(out)


SIGNATURE_A_HTML = (
    '<span class="gmail_signature_prefix">-- </span><br>'
    '<div dir="ltr" class="gmail_signature" data-smartmail="gmail_signature">'
    '<div dir="ltr"><div>Alice Martin</div><div>Responsable qualité</div>'
    '<div><img src="cid:ii_seed_c1m1_logo" alt="logo.png" width="96" height="32"></div>'
    '<div><a href="https://www.linkedin.com/company/example"><img src="cid:ii_seed_c1m1_in" '
    'alt="linkedin.png" width="16" height="16"></a> <a href="https://x.com/example">'
    '<img src="cid:ii_seed_c1m1_x" alt="x.png" width="16" height="16"></a></div></div></div>'
)
SIGNATURE_A_TEXT = "-- \nAlice Martin\nResponsable qualité\n[image: logo.png]\n[image: linkedin.png] <https://www.linkedin.com/company/example> [image: x.png] <https://x.com/example>"
SIGNATURE_B_HTML = (
    '<span class="gmail_signature_prefix">-- </span><br>'
    '<div dir="ltr" class="gmail_signature" data-smartmail="gmail_signature">'
    '<div dir="ltr">Bob Durand<br>Chef de projet</div></div>'
)
SIGNATURE_B_TEXT = "-- \nBob Durand\nChef de projet"

CAPTURE_ERROR = png(120 * 1024, 101)
CAPTURE_BUTTON = png(30 * 1024, 102)
LOGO = png(8 * 1024, 103)
ICON_IN = png(1024, 104)
ICON_X = png(1024, 105)


def quote_html(attribution, inner):
    return (
        '<br><div class="gmail_quote gmail_quote_container">'
        f'<div dir="ltr" class="gmail_attr">{attribution}<br></div>'
        '<blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex;'
        'border-left:1px solid rgb(204,204,204);padding-left:1ex">'
        f"{inner}</blockquote></div>"
    )


def quote_text(attribution, text):
    return attribution + "\n\n" + "\n".join(("> " + line).rstrip() for line in text.split("\n"))


# --------------------------------------------------------------------------
# Conversation 1: HTML, lists, links, screenshots, signature images, quotes.
# --------------------------------------------------------------------------

C1_SUBJECT = f"{TAG} Retours recette"
C1_DATES = [
    "Tue, 3 Nov 2026 09:00:00 +0100",
    "Tue, 3 Nov 2026 10:15:00 +0100",
    "Tue, 3 Nov 2026 11:30:00 +0100",
]
C1_ATTR_M1 = f"Le mar. 3 nov. 2026 à 09:00, {ALICE_ATTR} a écrit :"
C1_ATTR_M2 = f"Le mar. 3 nov. 2026 à 10:15, {BOB_ATTR} a écrit :"
C1_ATTR_M1_TEXT = f"Le mar. 3 nov. 2026 à 09:00, {ALICE} a écrit :"
C1_ATTR_M2_TEXT = f"Le mar. 3 nov. 2026 à 10:15, {BOB} a écrit :"

C1_M1_INNER = (
    "<div>Bonjour Bob,</div><div><br></div>"
    "<div>Voici mes retours sur la recette de la version 2.3 : les annonces sont "
    "disponibles dans votre secteur depuis lundi, mais trois points bloquent encore.</div>"
    "<div><br></div><div>Points bloquants :</div>"
    "<ol><li>La page de connexion renvoie une erreur 500 dès que le mot de passe contient "
    "un caractère accentué comme é ou ç, ce qui bloque une partie des testeurs depuis la "
    "mise en production de vendredi soir.</li>"
    "<li>Le bouton Exporter reste grisé.</li><li>Le tri par date est inversé.</li></ol>"
    "<div>Remarques :</div>"
    "<ul><li>Affichage mobile<ul><li>menu tronqué sur iPhone SE</li>"
    "<li>police trop petite</li></ul></li><li>Documentation à jour</li></ul>"
    "<div><br></div>"
    '<div>Le ticket est ici : <a href="https://tracker.example.com/browse/RECETTE-42">'
    "ticket RECETTE-42</a>, la spécification sur "
    '<a href="https://docs.example.com/spec/v2">https://docs.example.com/spec/v2</a> '
    "et la page "
    '<a href="https://fr.wikipedia.org/wiki/Recette_(informatique)">'
    "https://fr.wikipedia.org/wiki/Recette_(informatique)</a> explique le terme. "
    'Écrivez à <a href="mailto:qa@example.com">qa@example.com</a> pour les accès.</div>'
    "<div><br></div><div>Capture de l'erreur :</div>"
    '<div><img src="cid:{cap_error}" alt="image.png" width="562" height="316"><br></div>'
    "<div><br></div><div>Et le bouton grisé :</div>"
    '<div><img src="cid:{cap_button}" alt="image.png" width="300" height="120"><br></div>'
    "<div><br></div><div>Merci,</div>"
)
C1_M1_TEXT = wrap(
    "Bonjour Bob,\n\n"
    "Voici mes retours sur la recette de la version 2.3 : les annonces sont "
    "disponibles dans votre secteur depuis lundi, mais trois points bloquent encore.\n\n"
    "Points bloquants :\n\n"
    "   1. La page de connexion renvoie une erreur 500 dès que le mot de passe "
    "contient un caractère accentué comme é ou ç, ce qui bloque une partie des "
    "testeurs depuis la mise en production de vendredi soir.\n"
    "   2. Le bouton Exporter reste grisé.\n"
    "   3. Le tri par date est inversé.\n\n"
    "Remarques :\n\n"
    "   - Affichage mobile\n"
    "      - menu tronqué sur iPhone SE\n"
    "      - police trop petite\n"
    "   - Documentation à jour\n\n"
    "Le ticket est ici : ticket RECETTE-42 "
    "<https://tracker.example.com/browse/RECETTE-42>, la spécification sur "
    "https://docs.example.com/spec/v2 et la page "
    "https://fr.wikipedia.org/wiki/Recette_(informatique) explique le terme. "
    "Écrivez à qa@example.com pour les accès.\n\n"
    "Capture de l'erreur :\n[image: image.png]\n\n"
    "Et le bouton grisé :\n[image: image.png]\n\n"
    "Merci,"
)


def c1_m1():
    inner = C1_M1_INNER.format(cap_error="ii_seed_c1m1_a", cap_button="ii_seed_c1m1_b")
    html = f'<div dir="ltr">{inner}{SIGNATURE_A_HTML}</div>'
    text = f"{C1_M1_TEXT}\n{SIGNATURE_A_TEXT}\n"
    related = multipart(
        "related",
        "000000000000c1m1r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c1m1a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", CAPTURE_ERROR, "ii_seed_c1m1_a"),
            image_part("image/png", "image.png", CAPTURE_BUTTON, "ii_seed_c1m1_b"),
            image_part("image/png", "logo.png", LOGO, "ii_seed_c1m1_logo"),
            image_part("image/png", "linkedin.png", ICON_IN, "ii_seed_c1m1_in"),
            image_part("image/png", "x.png", ICON_X, "ii_seed_c1m1_x"),
        ],
    )
    body = multipart("mixed", "000000000000c1m1m00000000000", [related])
    return message(1, 1, C1_DATES[0], ALICE, BOB, C1_SUBJECT, body)


C1_M2_NEW_HTML = (
    "<div>Merci Alice, je regarde l'erreur 500 dès ce matin avec l'équipe.</div>"
    "<div>Le bouton grisé vient d'un droit manquant, je le corrige aujourd'hui.</div>"
)
C1_M2_NEW_TEXT = (
    "Merci Alice, je regarde l'erreur 500 dès ce matin avec l'équipe.\n"
    "Le bouton grisé vient d'un droit manquant, je le corrige aujourd'hui."
)


def c1_m2_quote_html():
    inner = C1_M1_INNER.format(cap_error="ii_seed_c1m2_q", cap_button="ii_seed_c1m2_q2")
    return quote_html(C1_ATTR_M1, f'<div dir="ltr">{inner}{SIGNATURE_A_HTML}</div>')


def c1_m2():
    # The quoted screenshots come back under new Content-IDs; Gmail drops the
    # signature images of a quoted message, so their cid stay unresolved.
    html = (
        f'<div dir="ltr">{C1_M2_NEW_HTML}<div><br></div>{SIGNATURE_B_HTML}</div>'
        + c1_m2_quote_html()
    )
    text = (
        f"{C1_M2_NEW_TEXT}\n\n{SIGNATURE_B_TEXT}\n\n"
        + quote_text(C1_ATTR_M1_TEXT, f"{C1_M1_TEXT}\n{SIGNATURE_A_TEXT}")
        + "\n"
    )
    related = multipart(
        "related",
        "000000000000c1m2r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c1m2a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", CAPTURE_ERROR, "ii_seed_c1m2_q"),
            image_part("image/png", "image.png", CAPTURE_BUTTON, "ii_seed_c1m2_q2"),
        ],
    )
    return message(
        1, 2, C1_DATES[1], BOB, ALICE, f"Re: {C1_SUBJECT}", related, (1, 1), [(1, 1)]
    )


CAPTURE_FIX = png(60 * 1024, 106)


def c1_m3():
    new_html = (
        "<div>Parfait, merci. Pour suivre :</div>"
        "<ol><li>Erreur 500 : corrigée en préproduction, voici l'écran après correctif "
        '<img src="cid:ii_seed_c1m3_a" alt="image.png" width="400" height="200"> '
        "et le même écran en mode sombre "
        '<img src="cid:ii_seed_c1m3_a" alt="image.png" width="400" height="200"></li>'
        "<li>Tri par date : toujours inversé.</li></ol>"
    )
    new_text = (
        "Parfait, merci. Pour suivre :\n\n"
        "   1. Erreur 500 : corrigée en préproduction, voici l'écran après correctif\n"
        "   [image: image.png]\n"
        "   et le même écran en mode sombre [image: image.png]\n"
        "   2. Tri par date : toujours inversé."
    )
    quoted_m2 = (
        f'<div dir="ltr">{C1_M2_NEW_HTML}<div><br></div>{SIGNATURE_B_HTML}</div>'
        + c1_m2_quote_html()
    )
    html = (
        f'<div dir="ltr">{new_html}<div><br></div>{SIGNATURE_A_HTML}</div>'
        + quote_html(C1_ATTR_M2, quoted_m2)
    )
    text = (
        f"{new_text}\n\n{SIGNATURE_A_TEXT}\n\n"
        + quote_text(
            C1_ATTR_M2_TEXT,
            f"{C1_M2_NEW_TEXT}\n\n{SIGNATURE_B_TEXT}\n\n"
            + quote_text(C1_ATTR_M1_TEXT, f"{C1_M1_TEXT}\n{SIGNATURE_A_TEXT}"),
        )
        + "\n"
    )
    related = multipart(
        "related",
        "000000000000c1m3r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c1m3a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", CAPTURE_FIX, "ii_seed_c1m3_a"),
            image_part("image/png", "logo.png", LOGO, "ii_seed_c1m1_logo"),
            image_part("image/png", "linkedin.png", ICON_IN, "ii_seed_c1m1_in"),
            image_part("image/png", "x.png", ICON_X, "ii_seed_c1m1_x"),
        ],
    )
    return message(
        1, 3, C1_DATES[2], ALICE, BOB, f"Re: {C1_SUBJECT}", related, (1, 2), [(1, 1), (1, 2)]
    )


# --------------------------------------------------------------------------
# Conversation 2: related at the root, attachments, accented name, GIF only.
# --------------------------------------------------------------------------

C2_SUBJECT = f"{TAG} QR code du salon"
QR_CODE = png(100 * 1024, 201)
CAPTURE_PLAN = png(40 * 1024, 202)
PHOTO = jpeg(150 * 1024, 203)
ACCENTED = png(150 * 1024, 204)
ANIMATION = gif(25 * 1024, 205)


def c2_m4():
    # Gmail mobile layout of issue #14: related at the root, image part with a
    # Content-ID but no Content-Disposition.
    html = (
        '<div dir="ltr"><div>Voici le QR code à imprimer pour le stand du salon.</div>'
        '<div><img src="cid:ii_seed_c2m4_a" alt="image.png" width="300" height="300"></div>'
        f"<div><br></div>{SIGNATURE_B_HTML}</div>"
    )
    text = (
        "Voici le QR code à imprimer pour le stand du salon.\n[image: image.png]\n\n"
        f"{SIGNATURE_B_TEXT}\n"
    )
    body = multipart(
        "related",
        "000000000000c2m4r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c2m4a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", QR_CODE, "ii_seed_c2m4_a", disposition=None),
        ],
    )
    return message(2, 4, "Wed, 4 Nov 2026 14:00:00 +0100", BOB, ALICE, C2_SUBJECT, body)


def c2_m5():
    html = (
        '<div dir="ltr"><div>Merci Bob. En pièces jointes :</div>'
        "<ul><li>le bon de commande signé (PDF)</li><li>la photo du stand</li>"
        "<li>la capture d'écran de l'été dernier</li></ul>"
        "<div>Et le plan d'accès collé ici :</div>"
        '<div><img src="cid:ii_seed_c2m5_a" alt="image.png" width="500" height="280"></div>'
        '<div>Le programme complet : <a href="https://salon.example.com/programme">'
        "programme du salon</a>.</div>"
        f"<div><br></div>{SIGNATURE_A_HTML.replace('cid:ii_seed_c1m1_', 'https://example.com/sig/')}</div>"
    )
    text = (
        "Merci Bob. En pièces jointes :\n\n"
        "   - le bon de commande signé (PDF)\n   - la photo du stand\n"
        "   - la capture d'écran de l'été dernier\n\n"
        "Et le plan d'accès collé ici :\n[image: image.png]\n"
        "Le programme complet : programme du salon <https://salon.example.com/programme>.\n\n"
        f"{SIGNATURE_A_TEXT}\n"
    )
    related = multipart(
        "related",
        "000000000000c2m5r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c2m5a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", CAPTURE_PLAN, "ii_seed_c2m5_a"),
        ],
    )
    accented = (
        'Content-Type: image/png; name="=?UTF-8?Q?capture_d=27=C3=A9cran_=C3=A9t=C3=A9.png?="\n'
        "Content-Disposition: attachment; "
        "filename*=UTF-8''capture%20d%27%C3%A9cran%20%C3%A9t%C3%A9.png\n"
        "Content-Transfer-Encoding: base64\nX-Attachment-Id: f_seed_c2m5_3\n"
        "Content-ID: <f_seed_c2m5_3>\n\n" + b64(ACCENTED)
    )
    body = multipart(
        "mixed",
        "000000000000c2m5m00000000000",
        [
            related,
            image_part(
                "application/pdf", "bon-de-commande.pdf", pdf("Bon de commande signe"),
                "f_seed_c2m5_1", disposition="attachment",
            ),
            image_part(
                "image/jpeg", "photo-stand.jpg", PHOTO, "f_seed_c2m5_2", disposition="attachment"
            ),
            accented,
        ],
    )
    return message(
        2, 5, "Wed, 4 Nov 2026 15:20:00 +0100", ALICE, BOB, f"Re: {C2_SUBJECT}", body,
        (2, 4), [(2, 4)],
    )


def c2_m6():
    html = (
        '<div dir="ltr"><div><img src="cid:ii_seed_c2m6_a" alt="image.gif" '
        f'width="320" height="180"><br></div><div><br></div>{SIGNATURE_B_HTML}</div>'
    )
    text = f"[image: image.gif]\n\n{SIGNATURE_B_TEXT}\n"
    body = multipart(
        "related",
        "000000000000c2m6r00000000000",
        [
            multipart(
                "alternative",
                "000000000000c2m6a00000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/gif", "image.gif", ANIMATION, "ii_seed_c2m6_a"),
        ],
    )
    return message(
        2, 6, "Wed, 4 Nov 2026 16:05:00 +0100", BOB, ALICE, f"Re: {C2_SUBJECT}", body,
        (2, 5), [(2, 4), (2, 5)],
    )


# --------------------------------------------------------------------------
# Conversation 3: plain text only (flowed, hard wrapped, dangling marker).
# --------------------------------------------------------------------------

C3_SUBJECT = f"{TAG} Notes de réunion"
WHITEBOARD = png(80 * 1024, 301)
SCAN_ONE = png(110 * 1024, 302)
SCAN_TWO = png(130 * 1024, 303)

C3_M7_TEXT = (
    "Bonjour Bob,\n\n"
    "Voici les notes de la réunion de lancement de ce matin. Les annonces restent "
    "disponibles dans votre secteur jusqu'à la fin du mois, et le client veut une "
    "démonstration avant le comité du 20 novembre.\n\n"
    "Décisions :\n"
    "1. Le périmètre de la première livraison couvre la recherche, la fiche annonce "
    "et le formulaire de contact, sans le paiement en ligne.\n"
    "2. Les maquettes sont validées par le client au plus tard vendredi prochain, "
    "faute de quoi le planning glisse d'une semaine.\n"
    "3. Démo le 18 novembre.\n\n"
    "Le client a résumé son attente par une formule qu'il a répétée deux fois : « une "
    "recherche qui répond avant que l'utilisateur ait fini de taper sa demande »\n\n"
    "Adresse du comité :\n"
    "Salle Garonne\n"
    "12 rue des Lilas\n"
    "31000 Toulouse\n\n"
    "Photo du tableau blanc en pièce jointe.\n\n"
    "-- \n"
    "Alice Martin\n"
    "Responsable qualité\n"
)


def c3_m7():
    plain = text_part("plain", flowed(C3_M7_TEXT), "; format=flowed; delsp=yes")
    body = multipart(
        "mixed",
        "000000000000c3m7m00000000000",
        [plain, image_part("image/png", "tableau-blanc.png", WHITEBOARD, "f_seed_c3m7_1",
                           disposition="attachment")],
    )
    return message(3, 7, "Thu, 5 Nov 2026 12:00:00 +0100", ALICE, BOB, C3_SUBJECT, body)


C3_M8_TEXT = (
    "Merci Alice, voici les deux pages du compte rendu signé par le client, scannées "
    "ce matin avant son départ pour le salon de Lyon :\n\n"
    "[image: image.png]\n"
    "[image: image.png]\n\n"
    "La seconde page porte les réserves sur le planning.\n\n"
    f"{SIGNATURE_B_TEXT}\n\n"
)


def c3_m8():
    text = textwrap_fill(C3_M8_TEXT, 72) + quote_text(
        f"Le jeu. 5 nov. 2026 à 12:00, {ALICE} a écrit :",
        "Bonjour Bob,\n\nVoici les notes de la réunion de lancement de ce matin.",
    ) + "\n"
    body = multipart(
        "mixed",
        "000000000000c3m8m00000000000",
        [
            text_part("plain", text),
            image_part("image/png", "image.png", SCAN_ONE, "f_seed_c3m8_1", disposition="attachment"),
            image_part("image/png", "image.png", SCAN_TWO, "f_seed_c3m8_2", disposition="attachment"),
        ],
    )
    return message(
        3, 8, "Thu, 5 Nov 2026 13:10:00 +0100", BOB, ALICE, f"Re: {C3_SUBJECT}", body,
        (3, 7), [(3, 7)],
    )


def textwrap_fill(text, width):
    return wrap(text, width)


def c3_m9():
    # Truncated message: the marker survives, its image part does not.
    text = (
        "Le plan de salle annoté, comme promis :\n\n[image: image.png]\n\n"
        f"{SIGNATURE_B_TEXT}\n"
    )
    body = text_part("plain", text)
    return message(
        3, 9, "Thu, 5 Nov 2026 13:40:00 +0100", BOB, ALICE, f"Re: {C3_SUBJECT}", body,
        (3, 8), [(3, 7), (3, 8)],
    )


# --------------------------------------------------------------------------
# Conversation 4: forwards (inline and attached), encoded subject, long URL.
# --------------------------------------------------------------------------

C4_SUBJECT = f"{TAG} Devis révisé 🚀"
CAPTURE_QUOTE = png(70 * 1024, 401)
CAPTURE_TOTAL = png(50 * 1024, 402)
CAPTURE_ATTACHED = png(90 * 1024, 403)
FORWARD_HEADER_HTML = (
    '<div class="gmail_quote gmail_quote_container"><div dir="ltr" class="gmail_attr">'
    "---------- Forwarded message ---------<br>De : <strong class=\"gmail_sendername\" "
    'dir="auto">Claire Petit</strong> <span dir="auto">&lt;<a href="mailto:claire@example.org">'
    "claire@example.org</a>&gt;</span><br>Date: lun. 2 nov. 2026 à 17:45<br>"
    "Subject: Devis révisé<br>To: " + BOB_ATTR + "<br></div><br><br>"
)
FORWARD_HEADER_TEXT = (
    "---------- Forwarded message ---------\n"
    "De : Claire Petit <claire@example.org>\n"
    "Date: lun. 2 nov. 2026 à 17:45\n"
    "Subject: Devis révisé\n"
    f"To: {BOB}\n"
)


def c4_m10():
    html = (
        '<div dir="ltr"><div>Alice, je te transfère le devis révisé de Claire. '
        "J'ai ajouté la capture du total recalculé :</div>"
        '<div><img src="cid:ii_seed_c4m10_b" alt="image.png" width="400" height="120"></div>'
        "<div><br></div>" + FORWARD_HEADER_HTML
        + '<div dir="ltr"><div>Bonjour Bob,</div><div>Voici le devis révisé :</div>'
        '<table border="1"><tr><th>Poste</th><th>Jours</th><th>Montant</th></tr>'
        "<tr><td>Développement</td><td>12</td><td>7 200 €</td></tr>"
        "<tr><td>Recette</td><td>3</td><td>1 500 €</td></tr></table>"
        "<div>Capture de l'ancien devis pour comparaison :</div>"
        '<div><img src="cid:ii_seed_c4m10_a" alt="image.png" width="500" height="300"></div>'
        "<div>Claire</div></div></div></div>"
    )
    text = (
        "Alice, je te transfère le devis révisé de Claire. J'ai ajouté la capture du\n"
        "total recalculé :\n[image: image.png]\n\n" + FORWARD_HEADER_TEXT
        + "\n\nBonjour Bob,\nVoici le devis révisé :\n"
        "Poste Jours Montant\nDéveloppement 12 7 200 €\nRecette 3 1 500 €\n"
        "Capture de l'ancien devis pour comparaison :\n[image: image.png]\nClaire\n"
    )
    body = multipart(
        "related",
        "000000000000c4m10r0000000000",
        [
            multipart(
                "alternative",
                "000000000000c4m10a0000000000",
                [text_part("plain", text), text_part("html", html)],
            ),
            image_part("image/png", "image.png", CAPTURE_TOTAL, "ii_seed_c4m10_b"),
            image_part("image/png", "image.png", CAPTURE_QUOTE, "ii_seed_c4m10_a"),
        ],
    )
    return message(4, 10, "Fri, 6 Nov 2026 09:30:00 +0100", BOB, ALICE, f"Fwd: {C4_SUBJECT}", body)


def c4_m11():
    attached = (
        "MIME-Version: 1.0\nDate: Mon, 2 Nov 2026 18:02:00 +0100\n"
        "Message-ID: <claire-annexe@example.org>\nSubject: Annexe du devis\n"
        f"From: Claire Petit <claire@example.org>\nTo: {BOB}\n"
        + multipart(
            "related",
            "000000000000c4m11inner000000",
            [
                multipart(
                    "alternative",
                    "000000000000c4m11innera00000",
                    [
                        text_part("plain", "L'annexe chiffrée :\n[image: image.png]\n"),
                        text_part(
                            "html",
                            '<div dir="ltr"><div>L\'annexe chiffrée :</div><div><img '
                            'src="cid:ii_seed_c4m11_inner" alt="image.png"></div></div>',
                        ),
                    ],
                ),
                image_part("image/png", "image.png", CAPTURE_ATTACHED, "ii_seed_c4m11_inner"),
            ],
        )
    )
    forwarded = (
        'Content-Type: message/rfc822; name="Annexe du devis.eml"\n'
        'Content-Disposition: attachment; filename="Annexe du devis.eml"\n'
        "X-Attachment-Id: f_seed_c4m11_1\nContent-ID: <f_seed_c4m11_1>\n\n" + attached
    )
    body = multipart(
        "mixed",
        "000000000000c4m11m0000000000",
        [
            multipart(
                "alternative",
                "000000000000c4m11a0000000000",
                [
                    text_part("plain", "Et l'annexe de Claire, transférée en pièce jointe.\n"),
                    text_part(
                        "html",
                        '<div dir="ltr">Et l\'annexe de Claire, transférée en pièce jointe.</div>',
                    ),
                ],
            ),
            forwarded,
        ],
    )
    return message(
        4, 11, "Fri, 6 Nov 2026 09:35:00 +0100", BOB, ALICE, f"Fwd: {C4_SUBJECT}", body,
        (4, 10), [(4, 10)],
    )


LONG_URL = (
    "https://compta.example.com/devis/2026/11/claire-petit/revision-2"
    "?format=pdf&signature=attendue&relance=non"
)


def c4_m12():
    text = (
        "Merci Bob. Je valide le *total* recalculé, mais le poste_recette doit rester à "
        "3 jours. Liste de contrôle :\n\n"
        "- [x] montants vérifiés\n- [ ] signature de Claire\n\n"
        "Le devis signé sera déposé ici :\n"
        f"<{LONG_URL}>\n\n"
        f"{SIGNATURE_A_TEXT}\n"
    )
    html = (
        '<div dir="ltr"><div>Merci Bob. Je valide le *total* recalculé, mais le '
        "poste_recette doit rester à 3 jours. Liste de contrôle :</div><div><br></div>"
        "<div>- [x] montants vérifiés</div><div>- [ ] signature de Claire</div><div><br></div>"
        f'<div>Le devis signé sera déposé ici :</div><div><a href="{LONG_URL.replace("&", "&amp;")}">'
        f'{LONG_URL.replace("&", "&amp;")}</a></div><div><br></div>'
        f"{SIGNATURE_A_HTML.replace('cid:ii_seed_c1m1_', 'https://example.com/sig/')}</div>"
    )
    body = multipart(
        "alternative",
        "000000000000c4m12a0000000000",
        [text_part("plain", wrap_keep_urls(text)), text_part("html", html)],
    )
    return message(
        4, 12, "Fri, 6 Nov 2026 11:00:00 +0100", ALICE, BOB, f"Re: Fwd: {C4_SUBJECT}", body,
        (4, 10), [(4, 10)],
    )


def wrap_keep_urls(text):
    """Hard wrap like Gmail, never inside a `<url>` line."""
    return "\n".join(line if line.startswith("<http") else wrap(line) for line in text.split("\n"))


MESSAGES = [
    ("c1-m1-retours-recette.eml", c1_m1),
    ("c1-m2-re-retours-recette.eml", c1_m2),
    ("c1-m3-re-re-retours-recette.eml", c1_m3),
    ("c2-m4-qr-code.eml", c2_m4),
    ("c2-m5-re-qr-code.eml", c2_m5),
    ("c2-m6-capture-seule.eml", c2_m6),
    ("c3-m7-notes-reunion.eml", c3_m7),
    ("c3-m8-re-notes-reunion.eml", c3_m8),
    ("c3-m9-image-absente.eml", c3_m9),
    ("c4-m10-fwd-devis.eml", c4_m10),
    ("c4-m11-fwd-piece-jointe.eml", c4_m11),
    ("c4-m12-re-fwd-devis.eml", c4_m12),
]


def main():
    default = Path(__file__).resolve().parent.parent / "seed" / "gmail"
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else default
    out.mkdir(parents=True, exist_ok=True)
    for name, build in MESSAGES:
        (out / name).write_bytes(build())
    print(f"{len(MESSAGES)} messages written to {out}")


if __name__ == "__main__":
    main()
