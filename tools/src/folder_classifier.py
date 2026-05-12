from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

_EMAIL_TYPE_PATHS = {
    "direct": "Correspondance/Direct/Divers",
    "group": "Correspondance/Groupes/Divers",
    "mailing_list": "Listes/Divers/Divers",
}
_DEFAULT_PATH = "Divers/Divers/Divers"
_PATH_INVALID_CHARS = ['?', '*', '"', '<', '>', '|']


def propose_path(email: dict, config: dict, locked_levels: list[str] | None = None) -> str:
    locked = locked_levels or []

    if len(locked) >= 3:
        return "/".join(locked[:3])

    classify_cfg = config.get("classify", {})
    data_dir = Path(classify_cfg.get("data_dir", "data"))
    threshold: float = classify_cfg.get("confidence_threshold", 0.75)
    min_samples: int = classify_cfg.get("min_samples_before_ml", 20)
    cold_start_model: str = classify_cfg.get("cold_start_model", "qwen3:8b")

    corpus = _load_corpus(data_dir)

    use_ollama = len(corpus) < min_samples
    label: str | None = None

    if not use_ollama:
        model, vectorizer = _load_model(data_dir)
        if model is None or vectorizer is None:
            use_ollama = True
        else:
            label, confidence = _ml_propose_path(email, model, vectorizer)
            if confidence < threshold:
                use_ollama = True

    if use_ollama:
        known_classes = _load_known_classes(data_dir)
        expected_levels = max(1, 3 - len(locked))
        return _llm_propose_path(email, cold_start_model, known_classes, locked, expected_levels)

    # ML result: override first locked levels with IMAP classification
    if locked and label:
        parts = label.split("/")
        label = "/".join(locked + parts[len(locked):])
    return label or _rule_based_propose(email)


def record_decision(email: dict, path: str, config: dict) -> None:
    classify_cfg = config.get("classify", {})
    data_dir = Path(classify_cfg.get("data_dir", "data"))
    data_dir.mkdir(parents=True, exist_ok=True)

    corpus_path = data_dir / "corpus.jsonl"
    entry = {
        "subject": email.get("subject", ""),
        "sender": email.get("sender", ""),
        "email_type": email.get("email_type"),
        "label": path,
    }
    with corpus_path.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, ensure_ascii=False) + "\n")

    known_classes_path = data_dir / "known_classes.json"
    all_known_classes = _load_known_classes(data_dir)

    if path not in all_known_classes:
        all_known_classes.append(path)
        with known_classes_path.open("w", encoding="utf-8") as fh:
            json.dump(all_known_classes, fh, ensure_ascii=False)

    model, vectorizer = _load_model(data_dir)
    if model is None or vectorizer is None:
        logger.info("Modele non trouve, reconstruction depuis le corpus...")
        rebuild_model_from_corpus(data_dir)
        return

    features = _extract_features(entry)
    X = vectorizer.transform([features])

    existing_classes = list(model.classes_)
    if set(all_known_classes) != set(existing_classes):
        logger.info("Nouvelle classe detectee, reconstruction du modele...")
        rebuild_model_from_corpus(data_dir)
        model, vectorizer = _load_model(data_dir)
        if model is None or vectorizer is None:
            return
        X = vectorizer.transform([features])

    model.partial_fit(X, [path], classes=None)
    _save_model(model, vectorizer, data_dir)
    logger.info("Modele mis a jour avec la decision utilisateur : %s", path)


def rebuild_model_from_corpus(data_dir: Path) -> None:
    from sklearn.feature_extraction.text import TfidfVectorizer
    from sklearn.naive_bayes import BernoulliNB

    corpus = _load_corpus(data_dir)
    if not corpus:
        logger.warning("Corpus vide, impossible de reconstruire le modele.")
        return

    logger.info("Reconstruction du modele depuis %d exemples...", len(corpus))
    valid_entries = [e for e in corpus if "label" in e and "subject" in e and "sender" in e]
    if not valid_entries:
        logger.warning("Aucune entree valide dans le corpus, impossible de reconstruire le modele.")
        return

    texts = [_extract_features(e) for e in valid_entries]
    labels = [e["label"] for e in valid_entries]

    vectorizer = TfidfVectorizer()
    X = vectorizer.fit_transform(texts)

    model = BernoulliNB()
    model.fit(X, labels)

    _save_model(model, vectorizer, data_dir)
    logger.info("Modele reconstruit et sauvegarde avec succes.")


def prompt_user(
    email: dict,
    proposed_path: str,
    index: int | None = None,
    total: int | None = None,
    output_dir: Path | None = None,
    locked_levels: list[str] | None = None,
) -> str:
    from datetime import datetime

    date_val = email.get("date", "")
    if isinstance(date_val, datetime):
        date_str = date_val.strftime("%Y-%m-%d %H:%M")
    else:
        date_str = str(date_val)

    locked = locked_levels or []
    email_type = email.get("email_type") or "-"
    counter = f"[{index}/{total}]" if index is not None and total is not None else ""
    header = f"{counter}  {email_type}".strip()

    print()
    print("-" * 60)
    print(header)
    print()
    print(f"  {email.get('subject', '')}")
    print()
    print(f"  De   : {email.get('sender', '')}")
    print(f"  Date : {date_str}")
    if locked:
        print(f"  IMAP : {' / '.join(locked)}")
    print()
    print(f"  -> {proposed_path}")
    print()

    arbo_hint = "  a=arborescence  " if output_dir is not None else "  "
    response = input(
        f"[Entree]=accepter  s=skip  q=quitter{arbo_hint}ou chemin > "
    ).strip()

    if response in ("", "s", "q"):
        return response

    if response == "a" and output_dir is not None:
        return _pick_path_hierarchical(proposed_path, output_dir, locked)

    if response.count("/") != 2:
        print(f"  Avertissement : '{response}' doit etre au format Niveau1/Niveau2/Niveau3.")
    return response


def _pick_path_hierarchical(
    proposed_path: str,
    output_dir: Path,
    locked_levels: list[str] | None = None,
) -> str:
    proposed_parts = proposed_path.split("/")
    if len(proposed_parts) != 3:
        proposed_parts = (proposed_parts + ["", "", ""])[:3]

    locked = locked_levels or []
    chosen: list[str] = list(locked)
    current_dir = output_dir
    for part in locked:
        current_dir = current_dir / part

    if locked:
        print(f"\n  Niveaux verrouilles (IMAP) : {' / '.join(locked)}")

    for level_idx in range(len(locked), 3):
        proposed_part = proposed_parts[level_idx]
        level_num = level_idx + 1
        prefix = "/".join(chosen) + "/" if chosen else ""

        existing: list[str] = []
        if current_dir.exists():
            existing = sorted(d.name for d in current_dir.iterdir() if d.is_dir())

        print(f"\n  Niveau {level_num}  ({prefix or 'racine'})")
        for i, name in enumerate(existing, 1):
            marker = "  *" if name == proposed_part else ""
            print(f"    {i}. {name}{marker}")
        print("    n. nouveau dossier")
        if proposed_part:
            print(f"  (* recommande : {proposed_part})")

        raw = input("  Choix [Entree=*] > ").strip()

        if raw == "":
            chosen_part = proposed_part or "Divers"
        elif raw == "n":
            new_name = input("  Nom > ").strip()
            chosen_part = new_name if new_name else (proposed_part or "Divers")
        elif raw.isdigit():
            idx = int(raw) - 1
            chosen_part = existing[idx] if 0 <= idx < len(existing) else (proposed_part or "Divers")
        else:
            chosen_part = raw

        chosen.append(chosen_part)
        current_dir = current_dir / chosen_part

    return "/".join(chosen)


def _extract_features(email: dict) -> str:
    subject = email.get("subject") or ""
    sender = email.get("sender") or ""
    email_type = email.get("email_type") or ""
    return f"{subject} {sender} {email_type}"


def _llm_propose_path(
    email: dict,
    cold_start_model: str,
    known_classes: list[str] | None = None,
    locked_levels: list[str] | None = None,
    expected_levels: int = 3,
) -> str:
    locked = locked_levels or []
    known = known_classes or []
    expected_slashes = expected_levels - 1

    def _call(prompt: str) -> str:
        resp = ollama.chat(model=cold_start_model, messages=[{"role": "user", "content": prompt}])
        raw = resp["message"]["content"].strip()
        return raw.splitlines()[0].strip() if raw else ""

    def _validate(line: str) -> str | None:
        if line.count("/") != expected_slashes:
            return None
        if any(c in line for c in _PATH_INVALID_CHARS):
            return None
        parts = line.split("/")
        if not all(0 < len(p.strip()) <= 50 for p in parts):
            return None
        return "/".join(p.strip() for p in parts)  # D3: strip spaces from segments

    def _extract_unlocked_from_full(line: str) -> str | None:
        """D1: if Ollama returned a full 3-level path when fewer were expected, strip the locked prefix."""
        if not locked or line.count("/") != 2:
            return None
        parts = [p.strip() for p in line.split("/")]
        # Accept if the line starts with the locked prefix (case-insensitive)
        if [p.lower() for p in parts[:len(locked)]] == [l.lower() for l in locked]:
            candidate = "/".join(parts[len(locked):])
        else:
            candidate = "/".join(parts[len(locked):])  # strip anyway, best-effort
        return _validate(candidate)

    try:
        import ollama

        sections = [
            "Tu es un assistant de classement d'emails. "
            "Reponds UNIQUEMENT avec le chemin demande, sans explication, sans ponctuation, sans guillemets."
        ]

        if locked:
            sections.append(
                f"Niveaux deja imposes (IMAP) : {' / '.join(locked)}. "
                f"Propose UNIQUEMENT les {expected_levels} niveau(x) restant(s). "
                f"NE repete PAS les niveaux imposes dans ta reponse."
            )

        if known:
            labels_list = "\n".join(f"- {path}" for path in sorted(known)[:50])
            sections.append(f"Chemins deja utilises (a reutiliser si semantiquement adequat) :\n{labels_list}")
            sections.append(
                "Reutilise un chemin existant s'il convient semantiquement. "
                "Cree un NOUVEAU chemin uniquement si aucun existant ne convient."
            )
        else:
            sections.append("Premiere classification : sois coherent avec les categories que tu crees.")

        sections.append(
            "Pour les NOUVEAUX chemins : pluriel pour les categories generiques "
            "(Publicites, Investissements), PascalCase, noms propres tels quels "
            "(BNP, iPhone), accents conserves."
        )

        subject = email.get("subject", "")
        sender = email.get("sender", "")
        email_type = email.get("email_type", "")
        sections.append(
            f"Email a classer :\n  Sujet : {subject}\n  Expediteur : {sender}\n  Type : {email_type}"
        )

        example = "/".join(["Categorie"] * expected_levels)
        sections.append(
            f"Reponds avec exactement {expected_levels} niveau(x) separe(s) par '/', rien d'autre. "
            f"Exemple : {example}"
        )

        first_line = _call("\n\n".join(sections))
        logger.debug("Ollama tentative 1 : %s", first_line)

        validated = _validate(first_line)

        # D1: if Ollama returned a full 3-level path despite being asked for fewer, strip the locked prefix
        if validated is None and expected_levels < 3:
            validated = _extract_unlocked_from_full(first_line)
            if validated:
                logger.debug("Ollama tentative 1 : prefix IMAP retire -> %s", validated)

        if validated is None:
            logger.warning("Chemin LLM invalide (format) : %s", first_line)
            return _rule_based_propose(email)

        # Soft mapping: if first non-locked level ignores existing vocabulary, retry once
        if known:
            existing_at_level = {
                path.split("/")[len(locked)]
                for path in known
                if len(path.split("/")) > len(locked)
            }
            proposed_top = validated.split("/")[0]
            if existing_at_level and proposed_top.lower() not in {e.lower() for e in existing_at_level}:
                sample = ", ".join(sorted(existing_at_level)[:20])
                # D2: re-state locked context in retry prompt
                locked_ctx = (
                    f"Rappel : niveaux imposes (IMAP) : {' / '.join(locked)}. "
                    f"Propose uniquement les {expected_levels} niveau(x) restant(s).\n"
                    if locked else ""
                )
                retry_prompt = (
                    f"{locked_ctx}"
                    f"Ton choix '{proposed_top}' ne correspond a aucun niveau existant.\n"
                    f"Niveaux existants : {sample}\n"
                    f"Choisis parmi ces niveaux si l'un convient semantiquement, sinon cree un nouveau.\n"
                    f"Reponds avec exactement {expected_levels} niveau(x) separe(s) par '/', rien d'autre.\n"
                    f"Exemple : {example}"
                )
                retry_line = _call(retry_prompt)
                logger.debug("Ollama tentative 2 (soft map) : %s", retry_line)
                retry_validated = _validate(retry_line) or _extract_unlocked_from_full(retry_line)
                if retry_validated:
                    validated = retry_validated

        return "/".join(locked + validated.split("/"))

    except Exception as exc:
        logger.debug("LLM propose echoue: %s", exc)
        return _rule_based_propose(email)


def _load_known_classes(data_dir: Path) -> list[str]:
    known_classes_path = data_dir / "known_classes.json"
    if not known_classes_path.exists():
        return []
    with known_classes_path.open(encoding="utf-8") as fh:
        return json.load(fh)


def _rule_based_propose(email: dict) -> str:
    email_type = email.get("email_type")
    return _EMAIL_TYPE_PATHS.get(email_type, _DEFAULT_PATH)


def _ml_propose_path(email: dict, model: Any, vectorizer: Any) -> tuple[str, float]:
    import numpy as np

    features = _extract_features(email)
    X = vectorizer.transform([features])
    proba = model.predict_proba(X)[0]
    max_idx = int(np.argmax(proba))
    label = model.classes_[max_idx]
    confidence = float(proba[max_idx])
    return label, confidence


def _load_corpus(data_dir: Path) -> list[dict]:
    corpus_path = data_dir / "corpus.jsonl"
    if not corpus_path.exists():
        return []
    entries = []
    with corpus_path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                try:
                    entries.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    return entries


def _load_model(data_dir: Path) -> tuple[Any, Any]:
    import joblib

    model_path = data_dir / "model.pkl"
    vectorizer_path = data_dir / "vectorizer.pkl"
    if not model_path.exists() or not vectorizer_path.exists():
        return None, None
    model = joblib.load(model_path)
    vectorizer = joblib.load(vectorizer_path)
    return model, vectorizer


def _save_model(model: Any, vectorizer: Any, data_dir: Path) -> None:
    import joblib

    model_path = data_dir / "model.pkl"
    vectorizer_path = data_dir / "vectorizer.pkl"
    joblib.dump(model, model_path)
    joblib.dump(vectorizer, vectorizer_path)
