"""Build screens in the running FigCraft (figcraft --control 7979) and render them.

    python3 run.py            # all screens
    python3 run.py d2-02 d3   # only screens whose name starts with one of these
"""
import json, os, sys
from fc import FC
import app
from kit import B
from themes import MUTIG, MUTIG_DARK, GLAS, GLAS_DARK, GLAS_EGUI

HERE = os.path.dirname(os.path.abspath(__file__))
DESIGN = os.path.dirname(HERE)
SHOTS = os.path.join(DESIGN, "screens")
FILE = os.path.join(DESIGN, "aiscraft-entwuerfe.figcraft")

SCREENS = []
for row, (d, light, dark) in enumerate([("d2", MUTIG, MUTIG_DARK), ("d3", GLAS, GLAS_DARK)]):
    for col, (slug, fn, tok) in enumerate([
        ("01-start", app.build_start, light),
        ("02-chat-hell", app.build_chat, light),
        ("03-chat-dunkel", app.build_chat, dark),
        ("04-dokument", app.build_doc, light),
        ("05-whiteboard-leer", app.build_board, light),
        ("06-einstellungen", app.build_settings, light),
    ]):
        SCREENS.append((f"{d}-{slug}", fn, tok, col * 1380, row * 1000 + 80))

SCREENS.append(("d3-07-ohne-unschaerfe", app.build_chat, GLAS_EGUI, 6 * 1380, 1080))

ids_path = os.path.join(HERE, "ids.json")
ids = json.load(open(ids_path)) if os.path.exists(ids_path) else {}
f = FC()
want = sys.argv[1:]
for name, fn, tok, x, y in SCREENS:
    if want and not any(name.startswith(w) for w in want):
        continue
    if name in ids:
        try:
            f.run("edit.delete", ids=[ids[name]])
        except RuntimeError:
            pass
    b = B(name.replace("-", "_") + "_")
    title = {"d2": "Entwurf 2 · Mutig", "d3": "Entwurf 3 · Glas"}[name[:2]]
    fn(b, x, y, tok, f"{title} · {name[3:]}")
    res = f.batch(b.cmds, label=name)
    sid = res[0]["id"]
    ids[name] = sid
    f.call("ui.render", ids=[sid], scale=2, path=os.path.join(SHOTS, f"{name}@2x.png"))
    f.call("ui.render", ids=[sid], scale=1, path=os.path.join(SHOTS, f"{name}.png"))
    print(name, len(b.cmds), "cmds, frame", sid)
# row titles on the canvas
for key, text, y in (("label-d2", "Entwurf 2 · Mutig", 0), ("label-d3", "Entwurf 3 · Glas", 1000)):
    if key in ids:
        try:
            f.run("edit.delete", ids=[ids[key]])
        except RuntimeError:
            pass
    r = f.batch([{"command": "node.create", "as": "t", "params": {
        "type": "text", "text": text, "x": 0, "y": y, "select": False, "name": text,
        "props": {"fontFamily": "Barlow", "fontStyle": "Bold", "fontSize": 40, "fill": "#E8EEF5"}}}])
    ids[key] = r[0]["id"]
json.dump(ids, open(ids_path, "w"), indent=1)
f.run("file.saveAs", path=FILE)
print("saved", FILE)
