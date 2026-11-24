"""WCAG contrast check for the text colours of both drafts.

For glass, every translucent layer is composited over each backdrop colour (the gradient ends and the
glow colours at full strength, which is darker/more saturated than anything the blurred backdrop
really shows), and the worst case is reported.

    python3 check.py
"""
from kit import contrast, over
from themes import MUTIG, MUTIG_DARK, GLAS, GLAS_DARK


def stops(t):
    """Opaque colours a glass layer can sit on."""
    if not t["glass"]:
        return [t["app_fill"]]
    return [s for s in t["backdrop"][0]["stops"]] + [b[3] for b in t["blobs"]]


def grounds(t, *layers):
    """Composite `layers` (bottom first) over each backdrop colour."""
    out = []
    for g in stops(t):
        c = g
        for l in layers:
            c = over(l, c) if len(l) == 9 else l
        out.append(c)
    return out


def side_grounds(t):
    if t.get("side_fills"):
        return t["side_fills"][0]["stops"]
    return grounds(t, t["side_fill"])


PAIRS = [
    # label, text key, layers (keys) the text sits on, minimum
    ("Text auf Hauptfläche", "ink", ["main_fill"], 4.5),
    ("Nebentext auf Hauptfläche", "muted", ["main_fill"], 4.5),
    ("Nebentext auf Karte", "muted", ["main_fill", "surface"], 4.5),
    ("Link auf Hauptfläche", "link", ["main_fill"], 4.5),
    ("Nutzertext in Blase", "user_ink", ["main_fill", "user_bubble"], 4.5),
    ("Akzenttext auf Akzent-hell", "accent_ink", ["main_fill", "accent_soft"], 4.5),
    ("Text auf Akzent (Knopf)", "on_accent", ["accent"], 4.5),
    ("Tabellenkopf", "th_ink", ["main_fill", "surface", "th_fill"], 4.5),
    ("Code-Kommentar", "code_muted", ["main_fill", "code_bg"], 4.5),
    ("Code-Text", "code_ink", ["main_fill", "code_bg"], 4.5),
    ("Zweitakzent-Text", "accent2_ink", ["main_fill", "accent2_soft"], 4.5),
    ("Nebentext im Panel", "muted", ["panel_fill"], 4.5),
    ("Platzhalter im Eingabefeld", "muted", ["main_fill", "composer_fill"], 4.5),
    ("Chip im Eingabefeld", "chip_ink", ["main_fill", "composer_fill", "chip_fill"], 4.5),
]


def report(name, t):
    rows = []
    for label, fg, layers, need in PAIRS:
        gs = grounds(t, *[t[k] for k in layers if t.get(k)])
        worst = min(contrast(t[fg], g) for g in gs)
        rows.append((label, worst, need))
    for label, key in (("Seitenleiste Text", "side_ink2"), ("Seitenleiste Nebentext", "side_muted")):
        worst = min(contrast(t[key], g) for g in side_grounds(t))
        rows.append((label, worst, 4.5))
    worst = min(contrast(t["side_active_ink"], over(t["side_active"], g) if len(t["side_active"]) == 9 else t["side_active"])
                for g in side_grounds(t))
    rows.append(("Seitenleiste aktiv", worst, 4.5))
    if t.get("accent_fills"):
        worst = min(contrast(t["on_accent"], s) for s in t["accent_fills"][0]["stops"])
        rows.append(("Text auf Akzent-Verlauf", worst, 4.5))
    print(f"\n{name}")
    ok = True
    for label, v, need in rows:
        flag = "ok" if v >= need else "ZU NIEDRIG"
        ok &= v >= need
        print(f"  {label:32s} {v:5.2f}:1  {flag}")
    return ok


if __name__ == "__main__":
    all_ok = True
    for n, t in (("Mutig hell", MUTIG), ("Mutig dunkel", MUTIG_DARK), ("Glas hell", GLAS), ("Glas dunkel", GLAS_DARK)):
        all_ok &= report(n, t)
    print("\nalles AA" if all_ok else "\nnicht alles AA")
