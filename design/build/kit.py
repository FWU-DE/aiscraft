"""Layout DSL on top of FigCraft's node.create, collected into one command.batch per screen.

Adapted from ~/code/meeting-companion/design/build/kit.py: the same builder, plus vector icons
(Lucide-style, drawn on a 24-unit grid and scaled here), the AIS.chat logo and a contrast check.
"""
import re

# The active token set. Screens call `use(TOKENS)` before building.
C = {}


def use(tokens):
    C.clear()
    C.update(tokens)


# --------------------------------------------------------------------------- contrast (WCAG 2.x)

def _rgba(h):
    h = h.lstrip("#")
    v = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return v + ([int(h[6:8], 16) / 255] if len(h) == 8 else [1.0])


def over(fg, bg):
    """Colour of a translucent `fg` composited over an opaque `bg`."""
    f, b = _rgba(fg), _rgba(bg)
    a = f[3]
    return "#" + "".join("%02X" % round((f[i] * a + b[i] * (1 - a)) * 255) for i in range(3))


def _lum(h):
    c = [x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4 for x in _rgba(h)[:3]]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


def contrast(a, b):
    la, lb = sorted([_lum(a), _lum(b)], reverse=True)
    return (la + 0.05) / (lb + 0.05)


# --------------------------------------------------------------------------- paths

_TOK = re.compile(r"[A-Za-z]|-?\d*\.?\d+(?:e-?\d+)?")


def scale_path(d, s):
    """Scale every coordinate of an SVG path (no arcs) by s. Absolute and relative commands both
    scale by s alone: FigCraft places the vector at the node's x/y plus the path's own offset."""
    out = []
    for t in _TOK.findall(d):
        if t.isalpha():
            if t in "Aa":
                raise ValueError("arcs are not supported in icon paths")
            out.append(t)
        else:
            out.append(f"{float(t) * s:.3f}".rstrip("0").rstrip("."))
    return " ".join(out)


# Icons on a 24 grid: a list of path strings and circles ("c", cx, cy, r) / dots ("d", cx, cy, r).
ICONS = {
    "plus": ["M12 5 L12 19 M5 12 L19 12"],
    "search": [("c", 11, 11, 6.5), "M20.5 20.5 L15.8 15.8"],
    "folder": ["M3.5 6.5 C3.5 5.7 4.2 5 5 5 L9.2 5 L11.2 7 L19 7 C19.8 7 20.5 7.7 20.5 8.5 L20.5 17.5 C20.5 18.3 19.8 19 19 19 L5 19 C4.2 19 3.5 18.3 3.5 17.5 Z"],
    "file": ["M6 3 L14 3 L19 8 L19 20 C19 20.6 18.6 21 18 21 L6 21 C5.4 21 5 20.6 5 20 L5 4 C5 3.4 5.4 3 6 3 Z M14 3 L14 8 L19 8 M8.5 13 L15.5 13 M8.5 16.5 L13.5 16.5"],
    "sheet": ["M5 4 L19 4 L19 20 L5 20 Z M5 9 L19 9 M5 14.5 L19 14.5 M10.5 9 L10.5 20"],
    "note": ["M5 4 L19 4 L19 14 L14 19 L5 19 Z M14 19 L14 14 L19 14"],
    "chat": ["M4.5 5.5 C4.5 4.7 5.2 4 6 4 L18 4 C18.8 4 19.5 4.7 19.5 5.5 L19.5 14.5 C19.5 15.3 18.8 16 18 16 L9.5 16 L5.5 20 L5.5 16 C5 16 4.5 15.4 4.5 14.8 Z"],
    "sliders": ["M4 7 L13 7 M19 7 L20 7 M4 17 L6 17 M12 17 L20 17", ("c", 16, 7, 2.5), ("c", 9, 17, 2.5)],
    "chev": ["M6.5 9.5 L12 15 L17.5 9.5"],
    "chevr": ["M9.5 6.5 L15 12 L9.5 17.5"],
    "up": ["M12 19 L12 5.5 M6 11.5 L12 5.5 L18 11.5"],
    "copy": ["M9 9 L19 9 C19.6 9 20 9.4 20 10 L20 19 C20 19.6 19.6 20 19 20 L10 20 C9.4 20 9 19.6 9 19 Z M5.5 15 L5 15 C4.4 15 4 14.6 4 14 L4 5 C4 4.4 4.4 4 5 4 L14 4 C14.6 4 15 4.4 15 5 L15 5.5"],
    "speaker": ["M11 5 L6.5 9 L3.5 9 L3.5 15 L6.5 15 L11 19 Z M15.5 9 C16.8 10.6 16.8 13.4 15.5 15 M18.5 6 C21.2 9.2 21.2 14.8 18.5 18"],
    "refresh": ["M20 12 C20 16.4 16.4 20 12 20 C7.6 20 4 16.4 4 12 C4 7.6 7.6 4 12 4 C14.6 4 16.9 5.2 18.4 7.2 M18.8 3.2 L18.8 7.6 L14.4 7.6"],
    "sparkle": ["M11 4 L12.6 9.4 L18 11 L12.6 12.6 L11 18 L9.4 12.6 L4 11 L9.4 9.4 Z M18.5 3 L18.5 7 M16.5 5 L20.5 5 M18 17 L18 20 M16.5 18.5 L19.5 18.5"],
    "clip": ["M20.5 11.5 L12.4 19.6 C10.5 21.5 7.5 21.5 5.6 19.6 C3.7 17.7 3.7 14.7 5.6 12.8 L13.4 5 C14.7 3.7 16.8 3.7 18.1 5 C19.4 6.3 19.4 8.4 18.1 9.7 L10.4 17.4 C9.7 18.1 8.6 18.1 7.9 17.4 C7.2 16.7 7.2 15.6 7.9 14.9 L15 7.8"],
    "globe": [("c", 12, 12, 8.5), "M3.5 12 L20.5 12 M12 3.5 C14.3 5.8 15.5 8.8 15.5 12 C15.5 15.2 14.3 18.2 12 20.5 C9.7 18.2 8.5 15.2 8.5 12 C8.5 8.8 9.7 5.8 12 3.5"],
    "mic": ["M12 3 C10.3 3 9 4.3 9 6 L9 11.5 C9 13.2 10.3 14.5 12 14.5 C13.7 14.5 15 13.2 15 11.5 L15 6 C15 4.3 13.7 3 12 3 Z M5.5 11 C5.5 14.6 8.4 17.5 12 17.5 C15.6 17.5 18.5 14.6 18.5 11 M12 17.5 L12 21"],
    "x": ["M6.5 6.5 L17.5 17.5 M17.5 6.5 L6.5 17.5"],
    "history": ["M4 12 C4 16.4 7.6 20 12 20 C16.4 20 20 16.4 20 12 C20 7.6 16.4 4 12 4 C9.2 4 6.8 5.4 5.4 7.6 M5 3.4 L5 7.8 L9.4 7.8 M12 8 L12 12 L14.8 13.8"],
    "download": ["M12 4 L12 15 M7 10.5 L12 15.5 L17 10.5 M5 19.5 L19 19.5"],
    "share": ["M12 15 L12 4 M7.5 8 L12 3.5 L16.5 8 M6 12 L5 12 L5 20 L19 20 L19 12 L18 12"],
    "more": [("d", 6, 12, 1.6), ("d", 12, 12, 1.6), ("d", 18, 12, 1.6)],
    "sun": [("c", 12, 12, 4), "M12 2.5 L12 4.5 M12 19.5 L12 21.5 M2.5 12 L4.5 12 M19.5 12 L21.5 12 M5.3 5.3 L6.7 6.7 M17.3 17.3 L18.7 18.7 M5.3 18.7 L6.7 17.3 M17.3 6.7 L18.7 5.3"],
    "moon": ["M19.5 14.5 C18.5 14.9 17.4 15.1 16.3 15.1 C11.7 15.1 8 11.4 8 6.8 C8 5.7 8.2 4.6 8.6 3.6 C5.4 4.8 3.2 7.9 3.2 11.5 C3.2 16.2 7 20 11.7 20 C15.3 20 18.3 17.7 19.5 14.5 Z"],
    "monitor": ["M3.5 5 C3.5 4.4 3.9 4 4.5 4 L19.5 4 C20.1 4 20.5 4.4 20.5 5 L20.5 15 C20.5 15.6 20.1 16 19.5 16 L4.5 16 C3.9 16 3.5 15.6 3.5 15 Z M8 20 L16 20 M12 16 L12 20"],
    "star": ["M12 3.5 L14.6 8.9 L20.5 9.7 L16.2 13.8 L17.3 19.6 L12 16.8 L6.7 19.6 L7.8 13.8 L3.5 9.7 L9.4 8.9 Z"],
    "bell": ["M6 16 L6 11 C6 7.7 8.7 5 12 5 C15.3 5 18 7.7 18 11 L18 16 L19.5 17.5 L4.5 17.5 Z M10 20.5 L14 20.5"],
    "pen": ["M4 20 L4.8 16.2 L15.8 5.2 C16.6 4.4 17.8 4.4 18.6 5.2 C19.4 6 19.4 7.2 18.6 8 L7.6 19 Z M14 7 L17 10"],
    "eye": ["M2.5 12 C4.8 7.6 8.1 5.5 12 5.5 C15.9 5.5 19.2 7.6 21.5 12 C19.2 16.4 15.9 18.5 12 18.5 C8.1 18.5 4.8 16.4 2.5 12 Z", ("c", 12, 12, 3)],
    "book": ["M3.5 5.5 C5.8 4.5 9 4.5 12 6.5 C15 4.5 18.2 4.5 20.5 5.5 L20.5 19 C18.2 18 15 18 12 20 C9 18 5.8 18 3.5 19 Z M12 6.5 L12 20"],
    "quiz": ["M9 3.5 L15 3.5 L15 6.5 L9 6.5 Z M8 5 L6 5 C5.4 5 5 5.4 5 6 L5 20 C5 20.6 5.4 21 6 21 L18 21 C18.6 21 19 20.6 19 20 L19 6 C19 5.4 18.6 5 18 5 L16 5 M8.5 13.5 L11 16 L15.5 11.5"],
    "mail": ["M3.5 6.5 C3.5 5.9 3.9 5.5 4.5 5.5 L19.5 5.5 C20.1 5.5 20.5 5.9 20.5 6.5 L20.5 17.5 C20.5 18.1 20.1 18.5 19.5 18.5 L4.5 18.5 C3.9 18.5 3.5 18.1 3.5 17.5 Z M4 6.5 L12 12.5 L20 6.5"],
    "user": [("c", 12, 8, 4), "M4.5 20.5 C4.5 16.4 7.9 13.5 12 13.5 C16.1 13.5 19.5 16.4 19.5 20.5"],
    "cursor": ["M5.5 3.5 L18.5 11.5 L12.3 13 L9.5 19.5 Z"],
    "hand": ["M8 13 L8 5.5 C8 4.7 8.7 4 9.5 4 C10.3 4 11 4.7 11 5.5 L11 11 M11 10 L11 4 C11 3.2 11.7 2.5 12.5 2.5 C13.3 2.5 14 3.2 14 4 L14 10 M14 5.5 C14 4.7 14.7 4 15.5 4 C16.3 4 17 4.7 17 5.5 L17 11 M17 8 C17 7.2 17.7 6.5 18.5 6.5 C19.3 6.5 20 7.2 20 8 L20 14 C20 18 17.3 21 13.5 21 C10.8 21 9.3 19.9 7.8 18 L4.6 13.9 C4.1 13.2 4.3 12.3 5 11.8 C5.7 11.4 6.6 11.5 7.1 12.1 L8 13"],
    "rect": ["M4 6 C4 5.4 4.4 5 5 5 L19 5 C19.6 5 20 5.4 20 6 L20 18 C20 18.6 19.6 19 19 19 L5 19 C4.4 19 4 18.6 4 18 Z"],
    "ellipse": [("c", 12, 12, 8)],
    "arrow": ["M5 19 L19 5 M10 5 L19 5 L19 14"],
    "line": ["M5 19 L19 5"],
    "eraser": ["M8 20.5 L4.2 16.7 C3.4 15.9 3.4 14.7 4.2 13.9 L13.4 4.7 C14.2 3.9 15.4 3.9 16.2 4.7 L19.8 8.3 C20.6 9.1 20.6 10.3 19.8 11.1 L10.4 20.5 Z M9.5 9 L15.5 15 M8 20.5 L20 20.5"],
    "undo": ["M9 14.5 L4.5 10 L9 5.5 M4.5 10 L14.5 10 C17.8 10 20.5 12.7 20.5 16 C20.5 18.2 19.3 19.5 18 20"],
    "redo": ["M15 14.5 L19.5 10 L15 5.5 M19.5 10 L9.5 10 C6.2 10 3.5 12.7 3.5 16 C3.5 18.2 4.7 19.5 6 20"],
    "frame": ["M4 8 L4 4 L8 4 M16 4 L20 4 L20 8 M20 16 L20 20 L16 20 M8 20 L4 20 L4 16", ("c", 12, 12, 3)],
    "help": [("c", 12, 12, 8.5), "M9.6 9.6 C9.6 8.2 10.7 7.2 12.1 7.2 C13.5 7.2 14.5 8.2 14.5 9.5 C14.5 11.4 12 11.6 12 13.6", ("d", 12, 16.6, 1.1)],
    "cloud": ["M7 18.5 L17 18.5 C19.2 18.5 21 16.7 21 14.5 C21 12.3 19.3 10.6 17.2 10.5 C16.8 7.5 14.4 5.5 11.5 5.5 C8.6 5.5 6.3 7.7 6 10.5 C4.3 10.9 3 12.5 3 14.4 C3 16.7 4.8 18.5 7 18.5 Z"],
    "plug": ["M9 2.5 L9 7 M15 2.5 L15 7 M6 7 L18 7 L18 10.5 C18 13.8 15.3 16.5 12 16.5 C8.7 16.5 6 13.8 6 10.5 Z M12 16.5 L12 21.5"],
    "calendar": ["M4 6 C4 5.4 4.4 5 5 5 L19 5 C19.6 5 20 5.4 20 6 L20 19 C20 19.6 19.6 20 19 20 L5 20 C4.4 20 4 19.6 4 19 Z M4 10 L20 10 M8.5 3 L8.5 7 M15.5 3 L15.5 7"],
    "bulb": ["M9.5 18 L14.5 18 M10.5 21 L13.5 21 M12 3 C8.7 3 6 5.7 6 9 C6 11.2 7.2 12.9 9 13.9 L9 15.5 L15 15.5 L15 13.9 C16.8 12.9 18 11.2 18 9 C18 5.7 15.3 3 12 3 Z"],
    "lock": ["M5.5 11 L18.5 11 L18.5 20.5 L5.5 20.5 Z M8.5 11 L8.5 7.5 C8.5 5.6 10.1 4 12 4 C13.9 4 15.5 5.6 15.5 7.5 L15.5 11"],
    "chart": ["M4 20 L20 20 M7.5 16.5 L7.5 11 M12 16.5 L12 6 M16.5 16.5 L16.5 13"],
    "memory": ["M6 3.5 L18 3.5 L18 20.5 L6 20.5 Z M9.5 3.5 L9.5 20.5 M12.5 8.5 L15.5 8.5 M12.5 12 L15.5 12"],
    "palette": ["M12 3.5 C7.3 3.5 3.5 7.3 3.5 12 C3.5 16.7 7.3 20.5 12 20.5 C13 20.5 13.6 19.8 13.6 19 C13.6 18.1 13 17.7 13 16.8 C13 15.9 13.7 15.2 14.6 15.2 L16.6 15.2 C18.8 15.2 20.5 13.4 20.5 11.3 C20.5 7 16.7 3.5 12 3.5 Z",
                ("d", 8, 11, 1.2), ("d", 10.5, 7.5, 1.2), ("d", 15, 7.8, 1.2)],
    "info": [("c", 12, 12, 8.5), "M12 11 L12 16.5", ("d", 12, 7.8, 1.1)],
    "check": ["M5 12.5 L10 17.5 L19 7"],
    "panel": ["M4 5 L20 5 L20 19 L4 19 Z M9.5 5 L9.5 19"],
    "flask": ["M9.5 3.5 L14.5 3.5 M10.5 3.5 L10.5 9.5 L5 18.5 C4.5 19.4 5.1 20.5 6.2 20.5 L17.8 20.5 C18.9 20.5 19.5 19.4 19 18.5 L13.5 9.5 L13.5 3.5 M7.5 15 L16.5 15"],
    "pie": ["M12 3.5 L12 12 L20.5 12 C20.5 16.7 16.7 20.5 12 20.5 C7.3 20.5 3.5 16.7 3.5 12 C3.5 7.3 7.3 3.5 12 3.5 Z M15 3.9 C17.6 4.8 19.6 6.9 20.2 9.5 L15 9.5 Z"],
    "drop": ["M12 3.5 C12 3.5 5.5 10.5 5.5 14.5 C5.5 18 8.4 20.5 12 20.5 C15.6 20.5 18.5 18 18.5 14.5 C18.5 10.5 12 3.5 12 3.5 Z"],
    "leaf": ["M5 19 C5 11 10 5 20 4.5 C19.5 14 14 19 6.5 19 Z M5 19 L13 11"],
    "masks": ["M4 5 L13 5 L13 11 C13 14 11 16 8.5 16 C6 16 4 14 4 11 Z M11 9.5 L20 9.5 L20 15.5 C20 18.5 18 20.5 15.5 20.5 C14 20.5 12.6 19.8 11.8 18.6"],
}


def logo_paths():
    """The AIS.chat mark (FWU-DE/ais-chat logo.svg, also packaging/icons/aischatcraft.svg)."""
    return [
        "M202.2,2.6H62C28.8,2.6,2,29.5,2,62.6h0c0,33.1,26.9,60,60,60h140.2c33.1,0,60-26.9,60-60h0c0-33.1-26.9-60-60-60h0ZM202.2,43.1c10.8,0,19.5,8.8,19.5,19.5s-8.8,19.5-19.5,19.5H62c-10.8,0-19.5-8.8-19.5-19.5s8.8-19.5,19.5-19.5h140.2",
        "M236.7,138.6h-55.2c-11.3,0-20.4,9.1-20.4,20.4s9.1,20.4,20.4,20.4h55.2c11.3,0,20.4-9.1,20.4-20.4s-9.1-20.4-20.4-20.4h0Z",
        "M237.2,220.4c-3.3-3.3-4.9-7.4-4.9-12.5s1.6-9.2,4.9-12.5c3.3-3.3,7.4-4.9,12.5-4.9s9.2,1.6,12.5,4.9c3.3,3.3,4.9,7.4,4.9,12.5s-1.6,9.2-4.9,12.5c-3.3,3.3-7.4,4.9-12.5,4.9s-9.2-1.6-12.5-4.9h0Z",
    ]


# The logo's own box (viewBox units of the paths above) and where each path starts, so the three
# pieces keep their relative position when FigCraft normalises each vector to its own bounds.
LOGO_BOX = (2.0, 2.6, 267.1, 225.3)  # min x, min y, max x, max y
LOGO_PARTS = [(2.0, 2.6), (161.1, 138.6), (232.3, 190.5)]


# --------------------------------------------------------------------------- builder

class B:
    def __init__(self, prefix):
        self.cmds = []
        self.k = 0
        self.prefix = prefix
        self.named = {}

    def _ref(self):
        self.k += 1
        return f"{self.prefix}{self.k}"

    def node(self, type, parent=None, name=None, text=None, path=None, x=None, y=None, w=None, h=None, key=None,
             **props):
        ref = self._ref()
        p = {"type": type, "select": False}
        if parent is not None:
            p["parent"] = {"$ref": parent}
        if name:
            p["name"] = name
        if text is not None:
            p["text"] = text
        if path is not None:
            p["path"] = path
        for k, v in (("x", x), ("y", y), ("width", w), ("height", h)):
            if v is not None:
                p[k] = v
        if props:
            p["props"] = props
        self.cmds.append({"command": "node.create", "as": ref, "params": p})
        if key:
            self.named[key] = ref
        return ref

    # --- layout ---------------------------------------------------------
    def box(self, parent, name="Box", dir="vertical", gap=0, pad=0, fill=None, radius=0, w=None, h=None,
            sizingH=None, sizingV=None, main="min", cross="min", stroke=None, sw=1, effects=None, x=None, y=None,
            key=None, sides=None, fills=None, **extra):
        props = {"layout": dir, "gap": gap, "padding": pad, "mainAlign": main, "crossAlign": cross,
                 "cornerRadius": radius}
        if fills is not None:
            props["fills"] = fills
        else:
            props["fill"] = fill or "none"
        if stroke:
            props["stroke"] = stroke
            props["strokeWeight"] = sw
            props["strokeAlign"] = "inside"
            if sides:
                props["strokeSides"] = sides
        if effects:
            props["effects"] = effects
        props["sizingH"] = sizingH or ("fixed" if w else "hug")
        props["sizingV"] = sizingV or ("fixed" if h else "hug")
        props["clip"] = False  # FigCraft frames clip by default, which cuts off shadows
        props.update(extra)
        return self.node("frame", parent, name=name, x=x, y=y, w=w, h=h, key=key, **props)

    def row(self, parent, name="Row", **kw):
        kw.setdefault("cross", "center")
        return self.box(parent, name, dir="horizontal", **kw)

    def col(self, parent, name="Col", **kw):
        return self.box(parent, name, dir="vertical", **kw)

    def free(self, parent, name, w, h, fill=None, x=None, y=None, **extra):
        """A frame without auto layout: children sit at their own x/y."""
        props = {"fill": fill or "none", "clip": False}
        props.update(extra)
        return self.node("frame", parent, name=name, w=w, h=h, x=x, y=y, **props)

    def txt(self, parent, s, size=15, style="Regular", color=None, fillW=False, w=None, lh=None, name=None,
            align=None, ls=None, key=None, font="Barlow", **extra):
        props = {"fontFamily": font, "fontStyle": style, "fontSize": size, "fill": color or C["ink"]}
        if lh:
            props["lineHeight"] = lh
        if ls is not None:
            props["letterSpacing"] = ls
        if align:
            props["textAlign"] = align
        if fillW:
            props["sizingH"] = "fill"
            props["textAutoResize"] = "height"
        elif w:
            props["textAutoResize"] = "height"
        props.update(extra)
        return self.node("text", parent, name=name or s[:28], text=s, w=w, key=key, **props)

    def spacer(self, parent, dir="h", size=None):
        if size is not None:
            return self.node("frame", parent, name="Gap", w=size if dir == "h" else 1, h=size if dir == "v" else 1,
                             fill="none")
        return self.node("frame", parent, name="Spacer", w=1, h=1, fill="none",
                         sizingH="fill" if dir == "h" else "fixed", sizingV="fill" if dir == "v" else "fixed")

    def rect(self, parent, w, h, fill, radius=0, name="Rect", **extra):
        return self.node("rectangle", parent, name=name, w=w, h=h, fill=fill, cornerRadius=radius, **extra)

    def ellipse(self, parent, d, fill, name="Dot", x=None, y=None, **extra):
        return self.node("ellipse", parent, name=name, w=d, h=d, x=x, y=y, fill=fill, **extra)

    # --- icons ------------------------------------------------------------
    def icon(self, parent, name, size=20, color=None, weight=None, box=None):
        """Lucide-style line icon, `size` px, in a fixed frame of `box` px (default: size)."""
        color = color or C["ink2"]
        box = box or size
        s = size / 24
        weight = weight or max(1.4, round(1.75 * s * 1.05, 2))
        off = (box - size) / 2
        f = self.free(parent, f"Icon {name}", box, box)
        for part in ICONS[name]:
            if isinstance(part, tuple):
                kind, cx, cy, r = part
                d = 2 * r * s
                if kind == "c":
                    self.node("ellipse", f, name="Circle", x=off + (cx - r) * s, y=off + (cy - r) * s, w=d, h=d,
                              fill="none", stroke=color, strokeWeight=weight)
                else:
                    self.node("ellipse", f, name="Dot", x=off + (cx - r) * s, y=off + (cy - r) * s, w=d, h=d,
                              fill=color)
            else:
                self.node("vector", f, name="Path", path=scale_path(part, s), x=off, y=off, fill="none",
                          stroke=color, strokeWeight=weight, strokeCap="round", strokeJoin="round")
        return f

    def logo(self, parent, height, color="#107FAD"):
        """The AIS.chat mark, `height` px tall, in a fixed frame."""
        x0, y0, x1, y1 = LOGO_BOX
        s = height / (y1 - y0)
        f = self.free(parent, "AIS.chat logo", round((x1 - x0) * s, 2), height)
        for d, (px, py) in zip(logo_paths(), LOGO_PARTS):
            self.node("vector", f, name="Logo part", path=scale_path(d.replace(",", " "), s),
                      x=(px - x0) * s - px * s, y=(py - y0) * s - py * s, fill=color)
        return f
