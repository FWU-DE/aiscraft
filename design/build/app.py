"""AIS.ChatCraft screens, built from one set of components for both drafts.

The draft only changes tokens (themes.py) and a few switches (`C["glass"]`): the layout, content and
component anatomy stay the same, so the two drafts can be compared screen by screen.
"""
from kit import B, C, use

W, H = 1280, 800
HEAD = "Barlow"

# ----------------------------------------------------------------------------- content

USER_NAME = "Julia Berger"

SIDEBAR = {
    "Projekte": [("folder", "Biologie 7b"), ("folder", "Mathe 6a"), ("folder", "Elternabend")],
    "Dokumente & Whiteboards": [("file", "Elternbrief Klassenfahrt"), ("sheet", "Arbeitsblatt Brüche"),
                                ("note", "Ideensammlung")],
    "Heute": [("chat", "Erkläre Photosynthese"), ("chat", "Bewertungsraster Referat"), ("chat", "Quiz: Zellbestandteile")],
    "Gestern": [("chat", "Elternbrief zur Klassenfahrt")],
}

PHOTO_USER = "Erkläre die Photosynthese für meine 7. Klasse – kurz, mit Formel und einer Tabelle."
PHOTO_INTRO = "Mit der Photosynthese gewinnen Pflanzen aus Sonnenlicht Energie – vor allem in ihren Blättern."
PHOTO_STEPS = ["Die Blätter nehmen Kohlenstoffdioxid aus der Luft auf.",
               "Die Wurzeln liefern Wasser.",
               "Mit Sonnenlicht entstehen daraus Traubenzucker und Sauerstoff."]
PHOTO_TABLE = [("Stoff", "Rolle bei der Photosynthese"),
               ("Chlorophyll", "fängt das Sonnenlicht ein"),
               ("Wasser", "liefert Elektronen und Wasserstoff"),
               ("Kohlenstoffdioxid", "Baustoff für den Traubenzucker")]
PHOTO_FOLLOW = "Soll ich daraus ein Arbeitsblatt mit Lückentext für Klasse 7 machen?"

SUGGESTIONS = [("book", "Stundenentwurf", "Photosynthese, Klasse 7"),
               ("sheet", "Arbeitsblatt", "Brüche addieren, Klasse 6"),
               ("mail", "Elternbrief", "Klassenfahrt Edersee"),
               ("quiz", "Quiz", "Zellbestandteile, 5 Fragen")]

RECENT = [("file", "Elternbrief Klassenfahrt", "Dokument · heute, 10:42"),
          ("sheet", "Arbeitsblatt Brüche", "Arbeitsblatt · gestern"),
          ("chat", "Erkläre Photosynthese", "Chat · heute, 08:12")]

FAVOURITES = [("pen", "Korrekturhilfe Deutsch", "Assistent", "#0E7C66"),
              ("pie", "Mathe-Coach Brüche", "Assistent", "#107FAD"),
              ("user", "Goethe im Gespräch", "Dialogpartner", "#7A4BC2"),
              ("drop", "Wasserkreislauf erforschen", "Lernszenario", "#B5541F")]


# ----------------------------------------------------------------------------- small components

def accent_fill(kw=None):
    """Fill kwargs for an accent surface: a gradient in Glas, the flat AIS blue in Mutig."""
    kw = {} if kw is None else kw
    if C.get("accent_fills"):
        kw["fills"] = C["accent_fills"]
    else:
        kw["fill"] = C["accent"]
    return kw


def pill(b, parent, label, kind="ghost", icon=None, size=14, pad=(8, 14, 8, 14), trailing=None, name=None,
         fillW=False, style="SemiBold"):
    """Pill button. kinds: primary, soft, ghost (outlined), chip (filled neutral), plain, accent2."""
    kw = {}
    if kind == "primary":
        accent_fill(kw)
        fg = C["on_accent"]
    elif kind == "soft":
        kw["fill"], fg = C["accent_soft"], C["accent_ink"]
    elif kind == "accent2":
        kw["fill"], fg = C["accent2_soft"], C["accent2_ink"]
    elif kind == "chip":
        kw["fill"], fg = C["chip_fill"], C["chip_ink"]
    elif kind == "plain":
        fg = C["ink2"]
    else:  # ghost
        kw["fill"], fg = C["surface"], C["ink2"]
        kw["stroke"] = C["surface_stroke"] if C["glass"] else C["line"]
    if fillW:
        kw["sizingH"] = "fill"
    p = b.row(parent, name or f"Button {label}", gap=7, pad=list(pad), radius=999, main="center", **kw)
    if icon:
        b.icon(p, icon, size + 3, fg)
    if label:
        b.txt(p, label, size, style, fg)
    if trailing:
        b.icon(p, trailing, size + 1, fg)
    return p


def icon_button(b, parent, icon, color=None, size=20, box=36, fill=None, stroke=None, name=None):
    f = b.row(parent, name or f"Icon button {icon}", w=box, h=box, radius=999, main="center", cross="center",
              fill=fill, stroke=stroke)
    b.icon(f, icon, size, color or C["ink2"])
    return f


def logo_tile(b, parent, d=34):
    t = b.row(parent, "Logo tile", w=d, h=d, radius=round(d * 0.28), fill="#FFFFFF", main="center", cross="center",
              effects=[{"type": "dropShadow", "color": "#0000001F", "offset": [0, 1], "radius": 3}])
    b.logo(t, round(d * 0.5))
    return t


def kbd(b, parent, s, fg, bg):
    k = b.row(parent, f"Key {s}", pad=[2, 7, 2, 7], radius=6, fill=bg)
    b.txt(k, s, 11.5, "SemiBold", fg)
    return k


def avatar(b, parent, initials, d=36, fills=None):
    a = b.row(parent, f"Avatar {initials}", w=d, h=d, radius=d / 2, main="center", cross="center",
              fills=fills or [{"type": "linearGradient", "stops": ["#F0663F", "#C2417A"], "from": [0, 0], "to": [1, 1]}])
    b.txt(a, initials, round(d * 0.38), "Bold", "#FFFFFF")
    return a


def segmented(b, parent, items, active, size=13.5):
    sg = b.row(parent, "Segmented", gap=2, pad=3, radius=999, fill=C["sunken"] if not C["glass"] else C["chip_fill"])
    for label, icon in items:
        on = label == active
        kw = {}
        if on:
            kw = {"fill": C["surface"] if not C["glass"] else ("#FFFFFF" if not C["dark"] else "#FFFFFF24"),
                  "effects": [{"type": "dropShadow", "color": "#0B2A451F", "offset": [0, 1], "radius": 3}]}
        s = b.row(sg, f"Tab {label}", gap=6, pad=[6, 13, 6, 13], radius=999, **kw)
        if icon:
            b.icon(s, icon, 16, C["ink"] if on else C["muted"])
        b.txt(s, label, size, "SemiBold", C["ink"] if on else C["muted"])
    return sg


def section_label(b, parent, label, color, plus=True):
    r = b.row(parent, f"Section {label}", pad=[0, 10, 0, 12], sizingH="fill", h=24)
    b.txt(r, label.upper(), 11.5, "SemiBold", color, ls=1.1)
    b.spacer(r)
    if plus:
        b.icon(r, "plus", 15, color)
    return r


# ----------------------------------------------------------------------------- sidebar

def sidebar(b, parent, active=None):
    glass = C["glass"]
    kw = {"fills": C["side_fills"]} if C.get("side_fills") else {"fill": C["side_fill"]}
    s = b.col(parent, "Sidebar", w=C["side_w"], sizingV="fill", pad=[18, 14, 14, 14], gap=6, radius=C["side_radius"],
              effects=C["side_effects"], stroke=C["side_stroke"], clip=True, **kw)
    # brand
    br = b.row(s, "Brand", gap=10, pad=[0, 4, 10, 4], sizingH="fill")
    logo_tile(b, br, 34)
    b.txt(br, "AIS.ChatCraft", 19, "Bold", C["side_ink"], ls=-0.2)
    b.spacer(br)
    b.icon(br, "panel", 19, C["side_muted"])
    # primary action + search
    nk = {"fills": C["accent_fills"]} if C.get("accent_fills") else {"fill": C["side_new_fill"]}
    n = b.row(s, "Neuer Chat", gap=8, pad=[10, 14, 10, 14], radius=999, sizingH="fill", main="center",
              effects=[{"type": "dropShadow", "color": "#00000026", "offset": [0, 3], "radius": 10}], **nk)
    b.icon(n, "plus", 18, C["side_new_ink"], weight=2.2)
    b.txt(n, "Neuer Chat", 15, "SemiBold", C["side_new_ink"])
    sr = b.row(s, "Suche", gap=8, pad=[8, 8, 8, 12], radius=999, fill=C["side_search"], sizingH="fill",
               stroke=C["side_line"] if glass else None)
    b.icon(sr, "search", 17, C["side_search_ink"])
    b.txt(sr, "Chats durchsuchen", 14.5, "Regular", C["side_search_ink"])
    b.spacer(sr)
    kbd(b, sr, "Strg K", C["side_search_ink"], C["side_line"] if not glass else "#FFFFFFB3" if not C["dark"] else "#FFFFFF14")

    lst = b.col(s, "Liste", gap=1, sizingH="fill", pad=[8, 0, 0, 0])
    for sec, items in SIDEBAR.items():
        b.spacer(lst, "v", 8)
        section_label(b, lst, sec, C["side_muted"], plus=sec in ("Projekte", "Dokumente & Whiteboards"))
        for icon, label in items:
            on = label == active
            r = b.row(lst, f"Item {label}", gap=10, pad=[7, 10, 7, 12], radius=12, sizingH="fill",
                      fill=C["side_active"] if on else None,
                      effects=[{"type": "dropShadow", "color": "#0B2A451A", "offset": [0, 2], "radius": 6}] if on and glass and not C["dark"] else None)
            if on and not glass:
                b.rect(r, 3, 18, C["accent2"], radius=2, name="Active bar", absolute=True, x=0, y=8)
            ic = C["side_active_ink"] if on else C["side_ink2"] if icon != "folder" else C["side_muted"]
            b.icon(r, icon, 17, ic)
            b.txt(r, label, 14.5, "SemiBold" if on else "Regular", C["side_active_ink"] if on else C["side_ink2"])
    b.spacer(s, "v")
    # account
    ft = b.row(s, "Konto", gap=10, pad=[10, 6, 2, 6], sizingH="fill", stroke=C["side_line"], sides=[1, 0, 0, 0])
    avatar(b, ft, "JB", 36)
    c = b.col(ft, "Name", gap=1)
    b.txt(c, USER_NAME, 14.5, "SemiBold", C["side_ink"])
    sub = b.row(c, "Status", gap=5)
    b.ellipse(sub, 7, C["green"])
    b.txt(sub, "AIS.chat · 2 Modelle", 12.5, "Regular", C["side_muted"])
    b.spacer(ft)
    icon_button(b, ft, "sliders", C["side_ink2"], 19, 34)
    return s


# ----------------------------------------------------------------------------- frame of a screen

def screen(b, name, x, y, tokens):
    use(tokens)
    root = b.free(None, name, W, H, x=x, y=y, clip=True, key="screen",
                  **({"fill": C["app_fill"]} if C.get("app_fill") else {}))
    if C["glass"]:
        bd = b.free(root, "Hintergrund (Verlauf + weiche Lichter)", W, H, x=0, y=0, clip=True)
        b.node("rectangle", bd, name="Verlauf", x=0, y=0, w=W, h=H, fills=C["backdrop"])
        for i, (bx, by, d, col, blur) in enumerate(C["blobs"]):
            b.node("ellipse", bd, name=f"Licht {i + 1}", x=bx, y=by, w=d, h=d, fill=col,
                   effects=[{"type": "layerBlur", "radius": blur}])
    pad = 12 if C["glass"] else 0
    app = b.row(root, "App", x=0, y=0, w=W, h=H, pad=pad, gap=12 if C["glass"] else 0, cross="min")
    return root, app


def main_area(b, app, name="Hauptbereich"):
    kw = {"fill": C["main_fill"]}
    return b.col(app, name, sizingH="fill", sizingV="fill", radius=C["main_radius"], effects=C["main_effects"],
                 stroke=C["main_stroke"], clip=True, **kw)


def chat_header(b, parent, title, project, meta, narrow=False):
    h = b.row(parent, "Kopfzeile", gap=12, pad=[16, 22, 12, 28], sizingH="fill")
    c = b.col(h, "Titel", gap=3)
    b.txt(c, title, 20, "Bold", C["ink"], ls=-0.2)
    m = b.row(c, "Meta", gap=8)
    chip = b.row(m, "Projekt", gap=5, pad=[2, 9, 2, 7], radius=999, fill=C["accent_soft"])
    b.icon(chip, "folder", 13, C["accent_ink"])
    b.txt(chip, project, 12.5, "SemiBold", C["accent_ink"])
    b.txt(m, meta, 13, "Regular", C["muted"])
    b.spacer(h)
    if narrow:
        icon_button(b, h, "share", C["ink2"], 19)
        icon_button(b, h, "more", C["ink2"], 19)
        return h
    ctx = b.row(h, "Kontext", gap=8, pad=[6, 12, 6, 12], radius=999,
                fill=C["surface"], stroke=C["surface_stroke"] if C["glass"] else C["line"])
    b.txt(ctx, "Kontext", 12.5, "SemiBold", C["muted"])
    track = b.free(ctx, "Kontext-Balken", 44, 6, fill=C["sunken"] if not C["glass"] else C["line"], cornerRadius=3, clip=True)
    b.rect(track, 6, 6, C["accent"], radius=3, name="Wert", x=0, y=0)
    b.txt(ctx, "1 %", 12.5, "SemiBold", C["ink2"])
    icon_button(b, h, "share", C["ink2"], 19)
    icon_button(b, h, "bell", C["ink2"], 19)
    icon_button(b, h, "more", C["ink2"], 19)
    return h


# ----------------------------------------------------------------------------- messages

def user_msg(b, parent, text, maxw):
    r = b.row(parent, "Nutzernachricht", sizingH="fill", main="max")
    bub = b.col(r, "Blase", pad=[12, 18, 12, 18], radius=[C["r_bubble"], 6, C["r_bubble"], C["r_bubble"]],
                fill=C["user_bubble"], w=maxw)
    b.txt(bub, text, 15.5, "Regular", C["user_ink"], fillW=True, lh=23)
    return r


def ai_head(b, parent, model="Demo Sonnet"):
    r = b.row(parent, "Absender", gap=9)
    t = b.row(r, "Marke", w=26, h=26, radius=8, fill="#FFFFFF", main="center", cross="center",
              stroke=C["line"] if not C["dark"] else None)
    b.logo(t, 13)
    b.txt(r, "AIS.chat", 14, "Bold", C["ink"])
    b.txt(r, model, 13.5, "Regular", C["muted"])
    return r


def para(b, parent, s, size=15.5, color=None, lh=25, style="Regular"):
    return b.txt(parent, s, size, style, color or C["ink"], fillW=True, lh=lh)


def heading(b, parent, s, size=23):
    return b.txt(parent, s, size, "Bold", C["ink"], fillW=True, ls=-0.2)


def numbered(b, parent, items):
    l = b.col(parent, "Liste", gap=9, sizingH="fill")
    for i, s in enumerate(items, 1):
        r = b.row(l, f"Punkt {i}", gap=12, sizingH="fill", cross="min")
        n = b.row(r, "Nummer", w=24, h=24, radius=12, fill=C["accent_soft"], main="center", cross="center")
        b.txt(n, str(i), 12.5, "Bold", C["accent_ink"])
        b.txt(r, s, 15.5, "Regular", C["ink"], fillW=True, lh=24)
    return l


def bullets(b, parent, items, size=15):
    l = b.col(parent, "Liste", gap=6, sizingH="fill")
    for s in items:
        r = b.row(l, "Punkt", gap=10, sizingH="fill", cross="min")
        dotbox = b.row(r, "Punktbox", w=8, h=22, cross="center")
        b.ellipse(dotbox, 6, C["accent"])
        b.txt(r, s, size, "Regular", C["ink"], fillW=True, lh=22)
    return l


def formula(b, parent, s="6 CO₂ + 6 H₂O  →  C₆H₁₂O₆ + 6 O₂", label="Formel"):
    f = b.row(parent, "Formel", gap=14, pad=[12, 16, 12, 16], radius=14, sizingH="fill", fill=C["sunken"])
    b.rect(f, 4, 24, C["accent"], radius=2, name="Akzent")
    b.txt(f, s, 16.5, "Regular", C["ink"], font="Inter")
    b.spacer(f)
    b.txt(f, label.upper(), 11, "SemiBold", C["muted"], ls=1.1)
    return f


def table(b, parent, rows, col1=170, total=False, size=14.5):
    t = b.col(parent, "Tabelle", sizingH="fill", radius=14, clip=True, fill=C["surface"],
              stroke=C["surface_stroke"] if C["glass"] else C["line"])
    for i, (a, v) in enumerate(rows):
        head = i == 0
        last_total = total and i == len(rows) - 1
        kw = {"fill": C["th_fill"]} if head else {}
        if not head and i < len(rows) - 1:
            kw.update(stroke=C["row_line"], sides=[0, 0, 1, 0])
        if last_total:
            kw = {"fill": C["accent_soft"]}
        r = b.row(t, f"Zeile {i}", pad=[9 if not head else 10, 16, 9 if not head else 10, 16], sizingH="fill", **kw)
        fg = C["th_ink"] if head else C["ink"]
        st = "Bold" if head or last_total else "Regular"
        c1 = b.row(r, "Zelle 1", w=col1)
        b.txt(c1, a, size, st if head or last_total else "SemiBold", fg)
        b.txt(r, v, size, st, fg, fillW=True)
    return t


def code_block(b, parent, lines, lang="Python"):
    k = b.col(parent, "Code", sizingH="fill", radius=14, clip=True, fill=C["code_bg"])
    hd = b.row(k, "Code-Kopf", pad=[7, 10, 7, 16], sizingH="fill", fill=C["code_head"])
    b.txt(hd, lang, 12, "SemiBold", C["code_muted"], ls=0.6)
    b.spacer(hd)
    cp = b.row(hd, "Kopieren", gap=5, pad=[3, 8, 3, 8], radius=999)
    b.icon(cp, "copy", 14, C["code_muted"])
    b.txt(cp, "Kopieren", 12, "SemiBold", C["code_muted"])
    body = b.col(k, "Zeilen", gap=3, pad=[10, 16, 12, 16])
    for spans in lines:
        r = b.row(body, "Zeile", gap=0)
        for s, kind in spans:
            b.txt(r, s.replace(" ", "\u00a0"), 13.5, "Regular", C[{"": "code_ink", "c": "code_muted", "k": "code_kw", "s": "code_str",
                                              "n": "code_num"}[kind]], font="JetBrainsMono Nerd Font")
    return k


PHOTO_CODE = [
    [("# Sauerstoff pro Stunde (vereinfacht)", "c")],
    [("blaetter = ", ""), ("120", "n")],
    [("print", "k"), ("(blaetter * ", ""), ("0.5", "n"), (", ", ""), ('"ml"', "s"), (")", "")],
]


def actions(b, parent, extra_refresh=True):
    r = b.row(parent, "Aktionen", gap=6, pad=[2, 0, 0, 0])
    for icon, label in (("copy", "Kopieren"), ("speaker", "Vorlesen"), ("file", "Als Dokument"), ("note", "Als Whiteboard")):
        pill(b, r, label, "ghost", icon=icon, size=13, pad=(6, 12, 6, 10), style="SemiBold")
    if extra_refresh:
        icon_button(b, r, "refresh", C["muted"], 17, 32)
    return r


def artifact_card(b, parent, icon, title, sub):
    c = b.row(parent, "Artefakt", gap=12, pad=[10, 14, 10, 10], radius=16, sizingH="fill", fill=C["surface"],
              stroke=C["accent"], sw=1.5, effects=C["card_effects"])
    t = b.row(c, "Symbol", w=40, h=40, radius=12, main="center", cross="center", **accent_fill())
    b.icon(t, icon, 20, C["on_accent"])
    tc = b.col(c, "Text", gap=1, sizingH="fill")
    b.txt(tc, title, 15, "Bold", C["ink"])
    b.txt(tc, sub, 13, "Regular", C["muted"])
    pill(b, c, "Geöffnet", "soft", icon="eye", size=12.5, pad=(5, 11, 5, 9))
    return c


# ----------------------------------------------------------------------------- composer

def composer(b, parent, w, placeholder="Schreib eine Nachricht …", big=False, compact=False):
    c = b.col(parent, "Eingabe", w=w, gap=10 if not big else 14, pad=[16, 14, 12, 18] if not big else [20, 16, 14, 22],
              radius=C["r_composer"], fill=C["composer_fill"], stroke=C["composer_stroke"],
              effects=C["composer_effects"])
    t = b.row(c, "Textfeld", sizingH="fill", gap=2)
    b.rect(t, 2, 20 if not big else 22, C["accent"], radius=1, name="Cursor")
    b.txt(t, placeholder, 16 if not big else 17.5, "Regular", C["muted"])
    if big:
        b.spacer(c, "v", 10)
    r = b.row(c, "Werkzeuge", gap=6, sizingH="fill")
    icon_button(b, r, "sparkle", C["ink2"], 19, 34)
    icon_button(b, r, "clip", C["ink2"], 19, 34)
    if not compact:
        icon_button(b, r, "globe", C["ink2"], 19, 34)
    b.spacer(r, "h", 2)
    mp = b.row(r, "Modell", gap=7, pad=[6, 10, 6, 8], radius=999, fill=C["chip_fill"])
    dot = b.row(mp, "Modellmarke", w=20, h=20, radius=10, main="center", cross="center", fill="#FFFFFF")
    b.logo(dot, 9)
    b.txt(mp, "Demo Sonnet", 13.5, "SemiBold", C["chip_ink"])
    b.icon(mp, "chev", 15, C["chip_ink"])
    pill(b, r, "Quellen", "chip", trailing="chev", size=13.5, pad=(6, 10, 6, 12))
    if not compact:
        pill(b, r, "Erstellen", "chip", trailing="chev", size=13.5, pad=(6, 10, 6, 12))
    b.spacer(r)
    icon_button(b, r, "mic", C["ink2"], 19, 36)
    send = b.row(r, "Senden", w=40, h=40, radius=20, main="center", cross="center",
                 effects=[{"type": "dropShadow", "color": "#107FAD59", "offset": [0, 4], "radius": 12}], **accent_fill())
    b.icon(send, "up", 20, C["on_accent"], weight=2.4)
    return c


def hint(b, parent):
    return b.txt(parent, "KI kann Fehler machen. Prüfe wichtige Angaben.", 12.5, "Regular", C["muted"])


# ----------------------------------------------------------------------------- chat views

def main_w(panel=False):
    """Width of the middle area."""
    w = W - C["side_w"] - (36 if C["glass"] else 0)
    if panel:
        w -= C["panel_w"] + (12 if C["glass"] else 0)
    return w


def fade(b, parent, w):
    """Soft edge where the history scrolls under the composer."""
    top = C["main_fill"][:7]
    fh = 36 if not C["glass"] else 22
    b.node("rectangle", parent, name="Ausblendung", absolute=True, x=0, y=-fh, w=w, h=fh,
           fills=[{"type": "linearGradient", "stops": [top + "00", top + ("FF" if not C["glass"] else "99")],
                   "from": [0.5, 0], "to": [0.5, 1]}])


def photo_chat(b, main, cw, scrolled=False):
    chat_header(b, main, "Erkläre Photosynthese", "Biologie 7b", "Demo Sonnet · heute, 08:12")
    sc = b.col(main, "Verlauf", sizingH="fill", sizingV="fill", cross="center", clip=True,
               pad=[6 if not C["glass"] else 0, 24, 22 if scrolled and not C["glass"] else 0, 24],
               main="max" if scrolled else "min")
    col = b.col(sc, "Nachrichten", w=cw, gap=18 if not C["glass"] else 12)
    if not scrolled:  # scrolled down, the question and the intro are above the top edge
        user_msg(b, col, PHOTO_USER, 470)
    a = b.col(col, "Antwort", gap=12, sizingH="fill")
    if not scrolled:
        ai_head(b, a)
        para(b, a, PHOTO_INTRO)
    heading(b, a, "Was passiert?")
    numbered(b, a, PHOTO_STEPS)
    formula(b, a)
    table(b, a, PHOTO_TABLE)
    if scrolled:  # the light screen shows the top of the answer, the dark one its end with the code
        code_block(b, a, PHOTO_CODE)
        para(b, a, PHOTO_FOLLOW)
    actions(b, a)
    bot = b.col(main, "Unten", sizingH="fill", cross="center", gap=8, pad=[10 if not C["glass"] else 4, 24, 12, 24])
    fade(b, bot, main_w())
    composer(b, bot, cw)
    hint(b, bot)


def build_chat(b, x, y, tokens, name):
    root, app = screen(b, name, x, y, tokens)
    sidebar(b, app, active="Erkläre Photosynthese")
    main = main_area(b, app)
    photo_chat(b, main, 740, scrolled=C["dark"])
    return b


# ----------------------------------------------------------------------------- start page

def start_content(b, main, cw, narrow=False):
    top = b.row(main, "Oben", sizingH="fill", pad=[16, 20, 0, 20], gap=8)
    b.spacer(top)
    if not narrow:
        pill(b, top, "Neu: Whiteboards mit KI", "accent2", icon="sparkle", size=12.5, pad=(5, 12, 5, 10))
    icon_button(b, top, "bell", C["ink2"], 19)
    body = b.col(main, "Start", sizingH="fill", sizingV="fill", cross="center", pad=[18 if not narrow else 30, 24, 24, 24])
    col = b.col(body, "Spalte", w=cw, gap=0)
    b.txt(col, "Dienstag, 24. November", 14, "SemiBold", C["accent_ink"] if not C["dark"] else C["link"], ls=0.4)
    b.spacer(col, "v", 4)
    g = b.row(col, "Gruß", gap=0, cross="max")
    b.txt(g, "Guten Morgen, Julia.", 44 if not narrow else 36, "Bold", C["ink"], ls=-0.8)
    b.txt(col, "Woran arbeiten wir heute?", 22 if not narrow else 19, "Medium", C["ink2"])
    b.spacer(col, "v", 22)
    composer(b, col, cw, "Frag etwas oder beschreib, was du brauchst …", big=True, compact=narrow)
    b.spacer(col, "v", 22)
    # suggestions
    grid = b.row(col, "Vorschläge", gap=12, sizingH="fill", wrap=narrow, crossGap=12) if narrow else \
        b.row(col, "Vorschläge", gap=12, sizingH="fill")
    for icon, title, sub in SUGGESTIONS:
        card = b.col(grid, f"Vorschlag {title}", gap=10, pad=[14, 14, 14, 14], radius=C["r_card"], h=118,
                     fill=C["surface"], stroke=C["surface_stroke"], effects=C["card_effects"],
                     **({"w": (cw - 12) / 2} if narrow else {"sizingH": "fill"}))
        hr = b.row(card, "Kopf", sizingH="fill")
        t = b.row(hr, "Symbol", w=36, h=36, radius=11, fill=C["accent_soft"], main="center", cross="center")
        b.icon(t, icon, 19, C["accent_ink"])
        b.spacer(hr)
        b.icon(hr, "chevr", 16, C["muted"])
        tc = b.col(card, "Text", gap=1, sizingH="fill")
        b.txt(tc, title, 16, "Bold", C["ink"])
        b.txt(tc, sub, 13.5, "Regular", C["muted"], fillW=True)
    if narrow:
        return col
    b.spacer(col, "v", 30)
    fh = b.row(col, "Favoriten-Kopf", sizingH="fill", gap=8)
    b.icon(fh, "star", 18, C["accent2"], weight=2)
    b.txt(fh, "Favoriten aus AIS.chat", 18, "Bold", C["ink"])
    b.spacer(fh)
    b.txt(fh, "Alle anzeigen", 14, "SemiBold", C["link"])
    b.icon(fh, "chevr", 15, C["link"])
    b.spacer(col, "v", 12)
    fr = b.row(col, "Favoriten", gap=12, sizingH="fill")
    for icon, title, kind, color in FAVOURITES:
        card = b.row(fr, f"Favorit {title}", gap=12, pad=[12, 12, 12, 12], radius=C["r_card"], sizingH="fill",
                     fill=C["surface"], stroke=C["surface_stroke"], effects=C["card_effects"], cross="center")
        t = b.row(card, "Bild", w=42, h=42, radius=21, main="center", cross="center",
                  fills=[{"type": "linearGradient", "stops": [color + "D9", color], "from": [0, 0], "to": [1, 1]}])
        b.icon(t, icon, 20, "#FFFFFF")
        tc = b.col(card, "Text", gap=2, sizingH="fill")
        b.txt(tc, title, 14.5, "Bold", C["ink"], fillW=True, lh=18)
        b.txt(tc, kind, 12.5, "Regular", C["muted"])
    b.spacer(col, "v", 26)
    rh = b.row(col, "Zuletzt-Kopf", sizingH="fill", gap=8)
    b.icon(rh, "history", 18, C["ink2"], weight=2)
    b.txt(rh, "Zuletzt bearbeitet", 18, "Bold", C["ink"])
    b.spacer(col, "v", 10)
    rr = b.row(col, "Zuletzt", gap=12, sizingH="fill")
    for icon, title, sub in RECENT:
        it = b.row(rr, f"Zuletzt {title}", gap=11, pad=[9, 12, 9, 9], radius=14, sizingH="fill",
                   fill=C["surface"] if C["glass"] else None, stroke=C["surface_stroke"] if C["glass"] else C["line"])
        t = b.row(it, "Symbol", w=34, h=34, radius=10, fill=C["sunken"] if not C["glass"] else C["accent_soft"],
                  main="center", cross="center")
        b.icon(t, icon, 17, C["ink2"] if not C["glass"] else C["accent_ink"])
        tc = b.col(it, "Text", gap=1, sizingH="fill")
        b.txt(tc, title, 14, "SemiBold", C["ink"])
        b.txt(tc, sub, 12.5, "Regular", C["muted"])
    return col


def build_start(b, x, y, tokens, name):
    root, app = screen(b, name, x, y, tokens)
    sidebar(b, app, active=None)
    main = main_area(b, app)
    start_content(b, main, 820)
    return b


# ----------------------------------------------------------------------------- artifact panel

def panel(b, app, icon, title):
    p = b.col(app, "Seitenleiste rechts", w=C["panel_w"], sizingV="fill", fill=C["panel_fill"],
              radius=C["panel_radius"], effects=C["panel_effects"], clip=True,
              stroke=C["panel_stroke"], sides=None if C["glass"] else [0, 0, 0, 1])
    h = b.row(p, "Kopf", gap=10, pad=[16, 14, 8, 20], sizingH="fill")
    t = b.row(h, "Symbol", w=32, h=32, radius=10, main="center", cross="center", **accent_fill())
    b.icon(t, icon, 17, C["on_accent"])
    b.txt(h, title, 18, "Bold", C["ink"], ls=-0.2)
    b.spacer(h)
    pill(b, h, "Teilen", "primary", icon="share", size=13.5, pad=(7, 14, 7, 11))
    icon_button(b, h, "x", C["ink2"], 19, 34)
    return p


def doc_panel(b, app):
    p = panel(b, app, "file", "Elternbrief Klassenfahrt")
    tb = b.row(p, "Werkzeugleiste", gap=8, pad=[4, 14, 12, 20], sizingH="fill",
               stroke=C["line"], sides=[0, 0, 1, 0])
    segmented(b, tb, [("Bearbeiten", "pen"), ("Vorschau", "eye")], "Vorschau")
    pill(b, tb, "Im Chat", "ghost", size=13, pad=(6, 13, 6, 13))
    b.spacer(tb)
    v = b.row(tb, "Versionen", gap=5, pad=[5, 10, 5, 8], radius=999, fill=C["chip_fill"])
    b.icon(v, "history", 16, C["chip_ink"])
    b.txt(v, "V2", 13, "Bold", C["chip_ink"])
    icon_button(b, tb, "download", C["ink2"], 18, 32)
    # page
    area = b.col(p, "Dokumentfläche", sizingH="fill", sizingV="fill", pad=[18, 18, 0, 18], clip=True,
                 fill=None if C["glass"] else C["sunken"])
    page = b.col(area, "Seite", sizingH="fill", sizingV="fill", gap=11, pad=[26, 28, 24, 28], radius=[14, 14, 0, 0],
                 fill="#FFFFFF" if not C["dark"] else C["surface"],
                 effects=[{"type": "dropShadow", "color": "#0B2A4514", "offset": [0, 2], "radius": 10}])
    b.txt(page, "KLASSE 7B · VERSION 2", 11.5, "SemiBold", C["accent_ink"], ls=1.2)
    b.txt(page, "Elternbrief Klassenfahrt", 26, "Bold", "#0D1B2A" if not C["dark"] else C["ink"], ls=-0.4, fillW=True)
    ink = "#0D1B2A" if not C["dark"] else C["ink"]
    b.txt(page, "Liebe Eltern der Klasse 7b,", 14.5, "Regular", ink, fillW=True, lh=22)
    b.txt(page, "vom 12. bis 14. Mai fahren wir gemeinsam ins Schullandheim am Edersee. Wir freuen uns auf drei Tage mit "
                "Wanderungen, einer Kanutour und viel Zeit als Klassengemeinschaft.", 14.5, "Regular", ink, fillW=True, lh=22)
    b.txt(page, "Kosten", 18, "Bold", ink)
    table(b, page, [("Posten", "Betrag"), ("Unterkunft und Verpflegung", "96 €"), ("Busfahrt", "38 €"),
                    ("Programm (Kanutour)", "21 €"), ("Gesamt", "155 €")], col1=250, total=True, size=13.5)
    b.txt(page, "Der Förderverein übernimmt auf Antrag 30 € pro Kind. Bitte überweisen Sie den Betrag bis zum 15. April.",
          14.5, "Regular", ink, fillW=True, lh=22)
    b.txt(page, "Herzliche Grüße\nJulia Berger, Klassenleitung 7b", 14.5, "Regular", ink, fillW=True, lh=22)
    # revise
    rv = b.col(p, "Überarbeiten", sizingH="fill", pad=[12, 14, 14, 14], stroke=C["line"], sides=[1, 0, 0, 0],
               fill=C["panel_fill"] if not C["glass"] else None)
    f = b.row(rv, "Feld", gap=8, pad=[6, 6, 6, 14], radius=999, sizingH="fill", fill=C["surface"],
              stroke=C["accent"] if not C["glass"] else C["surface_stroke"], sw=1.5 if not C["glass"] else 1,
              effects=C["card_effects"] if C["glass"] else None)
    b.icon(f, "sparkle", 18, C["accent"])
    b.txt(f, "Mit KI überarbeiten, z. B. „kürzer“", 14, "Regular", C["muted"])
    b.spacer(f)
    pill(b, f, "Überarbeiten", "primary", size=13.5, pad=(8, 16, 8, 16))
    return p


def eltern_chat(b, main, cw):
    chat_header(b, main, "Elternbrief zur Klassenfahrt", "Elternabend", "Demo Sonnet · 10:40", narrow=True)
    sc = b.col(main, "Verlauf", sizingH="fill", sizingV="fill", cross="center", pad=[6, 22, 0, 22], clip=True)
    col = b.col(sc, "Nachrichten", w=cw, gap=20)
    user_msg(b, col, "Schreib einen Elternbrief zur Klassenfahrt der 7b an den Edersee, 12.–14. Mai.", 360)
    a = b.col(col, "Antwort", gap=12, sizingH="fill")
    ai_head(b, a)
    para(b, a, "Gern! Ich habe den Brief als Dokument angelegt. Du kannst ihn rechts bearbeiten oder mit der KI überarbeiten.")
    artifact_card(b, a, "file", "Elternbrief Klassenfahrt", "Dokument · Termin, Kosten, Rückmeldung")
    user_msg(b, col, "Bitte noch freundlicher und erwähne den Zuschuss vom Förderverein.", 340)
    a2 = b.col(col, "Antwort 2", gap=12, sizingH="fill")
    ai_head(b, a2)
    para(b, a2, "Erledigt: Version 2 klingt wärmer und nennt den Zuschuss von 30 € im Abschnitt „Kosten“.")
    r = b.row(a2, "Aktionen", gap=6)
    pill(b, r, "Kopieren", "ghost", icon="copy", size=13, pad=(6, 12, 6, 10))
    pill(b, r, "Vorlesen", "ghost", icon="speaker", size=13, pad=(6, 12, 6, 10))
    icon_button(b, r, "refresh", C["muted"], 17, 32)
    bot = b.col(main, "Unten", sizingH="fill", cross="center", gap=8, pad=[10, 18, 12, 18])
    fade(b, bot, main_w(True))
    composer(b, bot, cw, compact=True)
    hint(b, bot)


def build_doc(b, x, y, tokens, name):
    root, app = screen(b, name, x, y, tokens)
    sidebar(b, app, active="Elternbrief Klassenfahrt")
    main = main_area(b, app)
    cw = W - C["side_w"] - C["panel_w"] - (2 * 12 + 2 * 12 if C["glass"] else 0) - 44
    eltern_chat(b, main, cw)
    doc_panel(b, app)
    return b


# ----------------------------------------------------------------------------- whiteboard

def board_panel(b, app):
    p = panel(b, app, "note", "Ideensammlung")
    tb = b.row(p, "Werkzeugleiste", gap=8, pad=[4, 14, 12, 20], sizingH="fill", stroke=C["line"], sides=[0, 0, 1, 0])
    pill(b, tb, "Im Chat", "ghost", icon="chat", size=13, pad=(6, 12, 6, 10))
    b.spacer(tb)
    icon_button(b, tb, "history", C["ink2"], 18, 32)
    icon_button(b, tb, "download", C["ink2"], 18, 32)
    icon_button(b, tb, "more", C["ink2"], 18, 32)
    pw = C["panel_w"]
    ch = H - 2 * (12 if C["glass"] else 0) - 116
    cv = b.free(p, "Leinwand", pw, ch, clip=True, sizingH="fill",
                fill=None if C["glass"] else ("#FBFCFD" if not C["dark"] else C["panel_fill"]))
    dot = C["line"] if not C["glass"] else ("#1B3A5A2E" if not C["dark"] else "#FFFFFF1F")
    for gy in range(14, ch, 22):
        for gx in range(14, pw, 22):
            b.node("ellipse", cv, name="Raster", x=gx, y=gy, w=2, h=2, fill=dot)
    # floating tool bar
    tools = b.row(cv, "Werkzeuge", gap=2, pad=[5, 6, 5, 6], radius=999, x=(pw - 404) / 2, y=14,
                  fill=C["surface"] if not C["glass"] else C["composer_fill"], stroke=C["surface_stroke"],
                  effects=C["composer_effects"] if C["glass"] else C["card_effects"])
    for i, name in enumerate(["cursor", "hand", "note", "rect", "ellipse", "arrow", "pen", "eraser"]):
        on = i == 0
        f = b.row(tools, f"Werkzeug {name}", w=34, h=34, radius=17, main="center", cross="center",
                  **(accent_fill() if on else {}))
        b.icon(f, name, 18, C["on_accent"] if on else C["ink2"])
    b.rect(tools, 1, 20, C["line"], name="Trenner")
    for name in ("undo", "redo"):
        f = b.row(tools, f"Werkzeug {name}", w=34, h=34, radius=17, main="center", cross="center")
        b.icon(f, name, 18, C["muted"])
    # empty state
    es = b.col(cv, "Leer", gap=10, cross="center", w=330, x=(pw - 330) / 2, y=ch / 2 - 150)
    halo = b.row(es, "Symbol", w=72, h=72, radius=36, main="center", cross="center", fill=C["accent_soft"])
    inner = b.row(halo, "Innen", w=48, h=48, radius=24, main="center", cross="center", **accent_fill())
    b.icon(inner, "note", 24, C["on_accent"])
    b.spacer(es, "v", 2)
    b.txt(es, "Noch leer", 22, "Bold", C["ink"])
    b.txt(es, "Doppelklick legt eine Notiz an. Oder lass die KI aus einem Chat eine Mindmap bauen.", 14.5, "Regular",
          C["ink2"], w=330, align="center", lh=21)
    b.spacer(es, "v", 4)
    r = b.row(es, "Start", gap=8)
    pill(b, r, "Notiz anlegen", "soft", icon="plus", size=13.5, pad=(7, 14, 7, 11))
    pill(b, r, "Mindmap aus Chat", "ghost", icon="sparkle", size=13.5, pad=(7, 14, 7, 11))
    b.txt(es, "F1 zeigt alle Tastenkürzel", 12.5, "Regular", C["muted"])
    # colour bar
    cb = b.row(cv, "Farben", gap=8, pad=[7, 14, 7, 10], radius=999, x=(pw - 330) / 2, y=ch - 64,
               fill=C["surface"] if not C["glass"] else C["composer_fill"], stroke=C["surface_stroke"],
               effects=C["composer_effects"] if C["glass"] else C["card_effects"])
    for i, col in enumerate(["#1B2733", "#F5C84B", "#F59E5B", "#EF6F6C", "#E58FC4", "#A58BF0", "#6FB6EE", "#6CCB8F"]):
        if i == 6:
            ring = b.row(cb, "Auswahl", w=26, h=26, radius=13, main="center", cross="center", stroke=C["accent"], sw=2)
            b.ellipse(ring, 18, col)
        else:
            b.ellipse(cb, 20, col)
    b.rect(cb, 1, 18, C["line"], name="Trenner")
    b.txt(cb, "100 %", 13, "SemiBold", C["ink2"])
    return p


def build_board(b, x, y, tokens, name):
    root, app = screen(b, name, x, y, tokens)
    sidebar(b, app, active="Ideensammlung")
    main = main_area(b, app)
    cw = W - C["side_w"] - C["panel_w"] - (2 * 12 + 2 * 12 if C["glass"] else 0) - 56
    start_content(b, main, cw, narrow=True)
    board_panel(b, app)
    return b


# ----------------------------------------------------------------------------- settings

RAIL = [("cloud", "Dienste"), ("plug", "Konnektoren"), ("calendar", "Zeitpläne"), ("bulb", "Fähigkeiten"),
        ("lock", "Tresor"), ("chart", "Nutzung"), ("memory", "Gedächtnis"), ("palette", "Darstellung"), ("info", "Über")]


def theme_card(b, parent, label, icon, on, dark_preview=False, system=False):
    c = b.col(parent, f"Schema {label}", gap=10, pad=8, radius=16, sizingH="fill",
              fill=C["surface"], stroke=C["accent"] if on else (C["surface_stroke"] if C["glass"] else C["line"]),
              sw=2 if on else 1)
    pv = b.free(c, "Vorschau", 160, 92, cornerRadius=10, clip=True, sizingH="fill",
                fill="#F3F6F9" if not dark_preview else "#0D1723")
    if C["glass"]:
        b.node("rectangle", pv, name="Verlauf", x=0, y=0, w=200, h=92,
               fills=[{"type": "linearGradient", "stops": ["#BFE3F5", "#E1D8F8"] if not dark_preview else ["#0B3350", "#2A2160"],
                       "from": [0, 0], "to": [1, 1]}])
    sb = ("#0C2E4E" if not dark_preview else "#061626") if not C["glass"] else ("#FFFFFF99" if not dark_preview else "#FFFFFF1F")
    b.rect(pv, 42, 92 if not C["glass"] else 80, sb, radius=0 if not C["glass"] else 8, name="Leiste",
           x=0 if not C["glass"] else 6, y=0 if not C["glass"] else 6)
    if system:
        b.node("rectangle", pv, name="Dunkle Hälfte", x=100, y=0, w=100, h=92, fill="#0D1723" if not C["glass"] else "#14214A")
    for i, wv in enumerate([70, 52, 82]):
        b.rect(pv, wv, 7, "#0D1B2A33" if not dark_preview else "#FFFFFF33", radius=4, name="Zeile", x=58, y=16 + i * 15)
    b.rect(pv, 92, 18, "#FFFFFF" if not dark_preview else "#1C2B3E", radius=9, name="Eingabe", x=54, y=64)
    b.ellipse(pv, 12, C["accent"], x=132, y=67)
    r = b.row(c, "Beschriftung", gap=7, pad=[0, 4, 2, 4], sizingH="fill")
    b.icon(r, icon, 17, C["ink2"])
    b.txt(r, label, 14.5, "SemiBold", C["ink"])
    b.spacer(r)
    if on:
        ck = b.row(r, "Gewählt", w=20, h=20, radius=10, main="center", cross="center", **accent_fill())
        b.icon(ck, "check", 13, C["on_accent"], weight=2.4)
    return c


def setting_row(b, parent, title, sub, control, last=False):
    r = b.row(parent, f"Einstellung {title}", gap=16, pad=[14, 0, 14, 0], sizingH="fill",
              stroke=None if last else C["line"], sides=None if last else [0, 0, 1, 0])
    c = b.col(r, "Text", gap=2, sizingH="fill")
    b.txt(c, title, 15, "SemiBold", C["ink"])
    b.txt(c, sub, 13, "Regular", C["muted"], fillW=True, lh=18)
    control(r)
    return r


def toggle(b, parent, on=True):
    t = b.row(parent, "Schalter", w=46, h=26, radius=13, pad=3, main="max" if on else "min", cross="center",
              **(accent_fill() if on else {"fill": C["line"] if not C["glass"] else "#1B3A5A33"}))
    b.ellipse(t, 20, "#FFFFFF", effects=[{"type": "dropShadow", "color": "#0000002E", "offset": [0, 1], "radius": 2}])
    return t


def build_settings(b, x, y, tokens, name):
    root, app = screen(b, name, x, y, tokens)
    sidebar(b, app, active=None)
    main = main_area(b, app)
    start_content(b, main, 820)
    # dim + modal
    b.node("rectangle", root, name="Abdunkeln", x=0, y=0, w=W, h=H, fill=C["overlay"],
           **({"effects": [{"type": "backgroundBlur", "radius": 6}]} if C["glass"] else {}))
    mw, mh = 940, 640
    m = b.row(root, "Einstellungen", x=(W - mw) / 2, y=(H - mh) / 2, w=mw, h=mh, radius=24, cross="min",
              fill=C["modal_fill"], effects=C["modal_effects"], clip=True,
              stroke="#FFFFFF" if C["glass"] else None)
    rail = b.col(m, "Bereiche", w=232, sizingV="fill", gap=2, pad=[22, 12, 16, 14],
                 fill=C["sunken"] if not C["glass"] else "#FFFFFF66", stroke=C["line"], sides=[0, 1, 0, 0])
    b.txt(rail, "Einstellungen", 22, "Bold", C["ink"], ls=-0.3)
    b.spacer(rail, "v", 14)
    for icon, label in RAIL:
        on = label == "Darstellung"
        r = b.row(rail, f"Bereich {label}", gap=11, pad=[8, 12, 8, 12], radius=12, sizingH="fill",
                  **(accent_fill() if on else {}))
        b.icon(r, icon, 18, C["on_accent"] if on else C["ink2"])
        b.txt(r, label, 15, "SemiBold" if on else "Regular", C["on_accent"] if on else C["ink2"])
    b.spacer(rail, "v")
    hb = b.row(rail, "Handbuch", gap=7, pad=[8, 12, 0, 12])
    b.icon(hb, "help", 17, C["link"])
    b.txt(hb, "Handbuch: Darstellung", 13.5, "SemiBold", C["link"])

    body = b.col(m, "Inhalt", sizingH="fill", sizingV="fill", pad=[20, 22, 22, 32], gap=0)
    hr = b.row(body, "Kopf", sizingH="fill")
    hc = b.col(hr, "Titel", gap=2)
    b.txt(hc, "Darstellung", 28, "Bold", C["ink"], ls=-0.4)
    b.txt(hc, "So sieht AIS.ChatCraft auf diesem Gerät aus.", 14.5, "Regular", C["muted"])
    b.spacer(hr)
    icon_button(b, hr, "x", C["ink2"], 20, 36, fill=C["chip_fill"])
    b.spacer(body, "v", 18)
    b.txt(body, "FARBSCHEMA", 11.5, "SemiBold", C["muted"], ls=1.1)
    b.spacer(body, "v", 10)
    tr = b.row(body, "Schemata", gap=12, sizingH="fill")
    theme_card(b, tr, "Hell", "sun", True)
    theme_card(b, tr, "Dunkel", "moon", False, dark_preview=True)
    theme_card(b, tr, "Wie das System", "monitor", False, system=True)
    b.spacer(body, "v", 8)
    lst = b.col(body, "Liste", sizingH="fill")

    def swatches(r):
        s = b.row(r, "Farben", gap=8)
        for i, col in enumerate(["#107FAD", "#0E7C66", "#6D4FD8", "#D0532B"]):
            if i == 0:
                ring = b.row(s, "Gewählt", w=30, h=30, radius=15, main="center", cross="center", stroke=col, sw=2)
                b.ellipse(ring, 22, col)
            else:
                b.ellipse(s, 22, col)

    def size_slider(r):
        s = b.row(r, "Regler", gap=10)
        b.txt(s, "A", 13, "SemiBold", C["muted"])
        tr_ = b.free(s, "Spur", 150, 20)
        b.rect(tr_, 150, 4, C["line"] if not C["glass"] else "#1B3A5A26", radius=2, name="Spur", x=0, y=8)
        b.node("rectangle", tr_, name="Wert", x=0, y=8, w=62, h=4, cornerRadius=2, **accent_fill())
        b.ellipse(tr_, 18, "#FFFFFF", x=54, y=1, stroke=C["accent"], strokeWeight=2,
                  effects=[{"type": "dropShadow", "color": "#0000002E", "offset": [0, 1], "radius": 3}])
        b.txt(s, "A", 19, "SemiBold", C["muted"])
        b.txt(s, "16 px", 13, "SemiBold", C["ink2"])

    if not C["glass"]:
        setting_row(b, lst, "Seitenleiste", "Nachtblau setzt die Navigation deutlich vom Inhalt ab.",
                    lambda r: segmented(b, r, [("Nachtblau", None), ("Hell", None)], "Nachtblau", 13))
    if C["glass"]:
        setting_row(b, lst, "Transparenz", "Glas über dem Verlauf. „Fenster“ lässt den Desktop durchscheinen "
                    "(braucht Hyprland, macOS oder Windows 11).",
                    lambda r: segmented(b, r, [("Aus", None), ("Verlauf", None), ("Fenster", None)], "Verlauf", 13))
    setting_row(b, lst, "Akzentfarbe", "Standard ist das AIS-Blau. Gilt für Knöpfe, Auswahl und Links.", swatches)
    setting_row(b, lst, "Schriftgröße", "Für Chats, Dokumente und die Seitenleiste.", size_slider)
    setting_row(b, lst, "Dichte", "Wie viel Luft zwischen Nachrichten und Listen ist.",
                lambda r: segmented(b, r, [("Kompakt", None), ("Normal", None), ("Luftig", None)], "Normal", 13))
    setting_row(b, lst, "Bewegung reduzieren", "Keine Übergänge und kein Tipp-Effekt beim Antworten.",
                lambda r: toggle(b, r, False), last=True)
    return b
