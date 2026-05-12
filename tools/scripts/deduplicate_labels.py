import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
    import io
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
if sys.stderr.encoding and sys.stderr.encoding.lower() != "utf-8":
    import io
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

import argparse
import json
from collections import Counter
from difflib import SequenceMatcher
from pathlib import Path

from src.config import load_config
from src.folder_classifier import rebuild_model_from_corpus


def _similarity(a: str, b: str) -> float:
    return SequenceMatcher(None, a.lower(), b.lower()).ratio()


def _cluster_labels(labels: list[str], threshold: float) -> list[list[str]]:
    """Regroupe les labels similaires par union-find."""
    parent = {label: label for label in labels}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(x, y):
        parent[find(x)] = find(y)

    for i, a in enumerate(labels):
        for b in labels[i + 1:]:
            if _similarity(a, b) >= threshold:
                union(a, b)

    clusters: dict[str, list[str]] = {}
    for label in labels:
        root = find(label)
        clusters.setdefault(root, []).append(label)

    return [members for members in clusters.values() if len(members) > 1]


def _pick_canonical(cluster: list[str], counts: Counter) -> str | None:
    """Demande à l'utilisateur de choisir le label canonique. Retourne None pour ignorer."""
    print("\n" + "─" * 60)
    print("Labels similaires détectés :")
    for i, label in enumerate(cluster):
        print(f"  [{i + 1}] {label}  ({counts[label]} ex.)")
    print("  [n] Saisir un nouveau libellé")
    print("  [s] Ignorer ce groupe")

    while True:
        response = input("Choix : ").strip()
        if response == "s":
            return None
        if response == "n":
            new_label = input("Nouveau libellé (Niveau1/Niveau2/Niveau3) : ").strip()
            if new_label.count("/") == 2:
                return new_label
            print("Format invalide — attendu : Niveau1/Niveau2/Niveau3")
            continue
        if response.isdigit():
            idx = int(response) - 1
            if 0 <= idx < len(cluster):
                return cluster[idx]
        print("Réponse invalide.")


def main() -> None:
    parser = argparse.ArgumentParser(description="Déduplique les labels du corpus de classification.")
    parser.add_argument("--config", default="config/config.yaml")
    parser.add_argument(
        "--threshold", type=float, default=0.82,
        help="Seuil de similarité (0-1, défaut 0.82)"
    )
    args = parser.parse_args()

    config = load_config(Path(args.config))
    data_dir = Path(config.get("classify", {}).get("data_dir", "data"))
    corpus_path = data_dir / "corpus.jsonl"

    if not corpus_path.exists():
        print("Corpus vide — rien à dédupliquer.")
        sys.exit(0)

    entries = []
    with corpus_path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                try:
                    entries.append(json.loads(line))
                except json.JSONDecodeError:
                    pass

    counts = Counter(e["label"] for e in entries)
    unique_labels = list(counts.keys())

    clusters = _cluster_labels(unique_labels, args.threshold)

    if not clusters:
        print(f"Aucun doublon détecté (seuil {args.threshold}).")
        sys.exit(0)

    print(f"{len(clusters)} groupe(s) similaire(s) détecté(s).")

    replacements: dict[str, str] = {}

    for cluster in clusters:
        canonical = _pick_canonical(cluster, counts)
        if canonical is None:
            continue
        for label in cluster:
            if label != canonical:
                replacements[label] = canonical

    if not replacements:
        print("\nAucune modification.")
        sys.exit(0)

    # Appliquer les remplacements
    for entry in entries:
        if entry["label"] in replacements:
            entry["label"] = replacements[entry["label"]]

    # Réécrire le corpus
    with corpus_path.open("w", encoding="utf-8") as fh:
        for entry in entries:
            fh.write(json.dumps(entry, ensure_ascii=False) + "\n")

    # Mettre à jour known_classes.json
    known_classes = sorted(set(e["label"] for e in entries))
    known_classes_path = data_dir / "known_classes.json"
    with known_classes_path.open("w", encoding="utf-8") as fh:
        json.dump(known_classes, fh, ensure_ascii=False)

    # Reconstruire le modèle
    print(f"\n{len(replacements)} label(s) remplacé(s). Reconstruction du modèle...")
    rebuild_model_from_corpus(data_dir)
    print("Modèle reconstruit.")

    print("\nRemplacements effectués :")
    for old, new in replacements.items():
        print(f"  {old!r} → {new!r}")


if __name__ == "__main__":
    main()
