"""Design tokens for draft 2 ("Mutig") and draft 3 ("Glas"), light and dark.

Every text colour below is checked against what it sits on by `check.py` (WCAG AA: 4.5:1 for body
text, 3:1 for large text and icons); for glass, against the panel composited over the brightest and
the most saturated part of the backdrop.
"""

AIS_BLUE = "#107FAD"


def shadow(color="#0B2A45", a1="12", a2="1A", y=12, r=32):
    return [{"type": "dropShadow", "color": color + a1, "offset": [0, 2], "radius": 6},
            {"type": "dropShadow", "color": color + a2, "offset": [0, y], "radius": r}]


def vgrad(top, bottom):
    return [{"type": "linearGradient", "stops": [top, bottom], "from": [0.5, 0], "to": [0.5, 1]}]


def dgrad(a, b):
    return [{"type": "linearGradient", "stops": [a, b], "from": [0, 0], "to": [1, 1]}]


# ----------------------------------------------------------------------------- Mutig

MUTIG = {
    "name": "Mutig", "glass": False, "dark": False,
    # sidebar: deep night blue with light text
    "side_fills": vgrad("#0C2E4E", "#081F37"),
    "side_ink": "#F3F8FC", "side_ink2": "#C9D8E6", "side_muted": "#8FAFCC", "side_line": "#1D3E60",
    "side_active": "#17507D", "side_active_ink": "#FFFFFF", "side_search": "#132F4E", "side_search_ink": "#A9C0D6",
    "side_new_fill": AIS_BLUE, "side_new_ink": "#FFFFFF",
    "side_w": 268, "side_radius": 0, "side_effects": None, "side_stroke": None,
    # content
    "app_fill": "#F3F6F9", "main_fill": "#F3F6F9", "main_effects": None, "main_stroke": None, "main_radius": 0,
    "panel_fill": "#FFFFFF", "panel_effects": [{"type": "dropShadow", "color": "#0B2A4514", "offset": [-8, 0], "radius": 24}],
    "panel_stroke": "#DCE4EC", "panel_radius": 0, "panel_w": 468,
    "surface": "#FFFFFF", "surface_stroke": "#DFE6EE", "card_effects": shadow(a1="0A", a2="10", y=6, r=18),
    "sunken": "#EAF0F5", "ink": "#0D1B2A", "ink2": "#33445A", "muted": "#566578", "line": "#DCE4EC",
    "accent": AIS_BLUE, "on_accent": "#FFFFFF", "accent_fills": None,
    "accent_soft": "#DFF0F8", "accent_ink": "#0A5C80", "link": "#0B6E98",
    "accent2": "#F0663F", "accent2_soft": "#FDE7DF", "accent2_ink": "#A93A18",
    "user_bubble": "#DCEEF8", "user_ink": "#0D1B2A",
    "th_fill": "#0C2E4E", "th_ink": "#FFFFFF", "row_line": "#E3E9EF",
    "code_bg": "#0C2238", "code_ink": "#E6EDF5", "code_muted": "#94A9BE", "code_kw": "#7DD3FC",
    "code_str": "#FDBA74", "code_num": "#C4B5FD", "code_head": "#14304C",
    "composer_fill": "#FFFFFF", "composer_stroke": "#D5DFE9", "composer_effects": shadow(a1="10", a2="1F", y=14, r=36),
    "chip_fill": "#EEF3F7", "chip_ink": "#24364B",
    "overlay": "#06182B8C", "modal_fill": "#FFFFFF", "modal_effects": shadow(a1="1F", a2="40", y=24, r=64),
    "r_card": 16, "r_panel": 0, "r_composer": 22, "r_bubble": 20,
    "green": "#1F9D6B",
}

MUTIG_DARK = dict(MUTIG, **{
    "dark": True,
    "side_fills": vgrad("#071B30", "#041222"),
    "side_ink": "#EEF4FA", "side_ink2": "#C2D2E2", "side_muted": "#87A3BF", "side_line": "#14304C",
    "side_active": "#123F63", "side_search": "#0D2741", "side_search_ink": "#9DB5CC",
    "side_new_fill": "#38A8DB", "side_new_ink": "#04131F",
    "app_fill": "#0D1723", "main_fill": "#0D1723",
    "panel_fill": "#132030", "panel_stroke": "#22344A",
    "surface": "#142234", "surface_stroke": "#22364D", "card_effects": None,
    "sunken": "#1A2A3E", "ink": "#E8EEF5", "ink2": "#BDCAD8", "muted": "#8E9FB2", "line": "#22364D",
    "accent": "#38A8DB", "on_accent": "#04131F",
    "accent_soft": "#113A53", "accent_ink": "#9AD7F2", "link": "#7CC8EC",
    "accent2": "#FF8A66", "accent2_soft": "#3D2219", "accent2_ink": "#FFB59C",
    "user_bubble": "#16405C", "user_ink": "#EEF5FB",
    "th_fill": "#1B3A57", "th_ink": "#FFFFFF", "row_line": "#22364D",
    "code_bg": "#081422", "code_head": "#0F2033",
    "composer_fill": "#142234", "composer_stroke": "#26405A", "composer_effects": shadow("#000000", "33", "4D", 14, 36),
    "chip_fill": "#1D2F44", "chip_ink": "#D4DEE8",
    "green": "#3CCB91",
})

# ----------------------------------------------------------------------------- Glas
# Panels are translucent white over a soft, already-blurred backdrop. Because the backdrop is soft,
# the panels read as frosted even without a real backdrop blur (which egui cannot do in-app).

GLAS_BLOBS = [  # x, y, diameter, colour (centre of a soft glow), layer blur radius
    (-160, -200, 620, "#6FC3EC", 150),
    (760, -260, 640, "#B9A8F4", 160),
    (900, 420, 620, "#8EE0D2", 160),
    (120, 470, 560, "#FFCBAE", 150),
    (430, 120, 420, "#9FD6F2", 140),
]

GLAS_BLOBS_DARK = [
    (-180, -220, 640, "#0F6A97", 160),
    (780, -240, 640, "#4A3AA0", 170),
    (920, 430, 600, "#126E66", 170),
    (80, 480, 560, "#6B3352", 160),
    (440, 160, 420, "#0E4F7A", 150),
]

GLAS = {
    "name": "Glas", "glass": True, "dark": False,
    "backdrop": dgrad("#E4EEF7", "#EEE9F7"), "blobs": GLAS_BLOBS,
    "side_fills": None, "side_fill": "#FFFFFF7A",
    "side_ink": "#0E1E2E", "side_ink2": "#2F4256", "side_muted": "#4A5D71", "side_line": "#FFFFFF99",
    "side_active": "#FFFFFFD9", "side_active_ink": "#0A4F70", "side_search": "#FFFFFF99", "side_search_ink": "#4A5D71",
    "side_new_fill": AIS_BLUE, "side_new_ink": "#FFFFFF",
    "side_w": 252, "side_radius": 24, "side_stroke": "#FFFFFFCC",
    "side_effects": [{"type": "backgroundBlur", "radius": 40},
                     {"type": "innerShadow", "color": "#FFFFFFB3", "offset": [0, 1], "radius": 0},
                     {"type": "dropShadow", "color": "#1B3A5A14", "offset": [0, 10], "radius": 30}],
    "app_fill": None,
    "main_fill": "#FFFFFFA6", "main_stroke": "#FFFFFFD9", "main_radius": 24,
    "main_effects": [{"type": "backgroundBlur", "radius": 48},
                     {"type": "innerShadow", "color": "#FFFFFFCC", "offset": [0, 1], "radius": 0},
                     {"type": "dropShadow", "color": "#1B3A5A1A", "offset": [0, 16], "radius": 40}],
    "panel_fill": "#FFFFFFC4", "panel_stroke": "#FFFFFFE6", "panel_radius": 24, "panel_w": 452,
    "panel_effects": [{"type": "backgroundBlur", "radius": 48},
                      {"type": "innerShadow", "color": "#FFFFFFCC", "offset": [0, 1], "radius": 0},
                      {"type": "dropShadow", "color": "#1B3A5A1F", "offset": [0, 16], "radius": 40}],
    "surface": "#FFFFFFB8", "surface_stroke": "#FFFFFFF2",
    "card_effects": [{"type": "innerShadow", "color": "#FFFFFF", "offset": [0, 1], "radius": 0},
                     {"type": "dropShadow", "color": "#1B3A5A12", "offset": [0, 6], "radius": 18}],
    "sunken": "#E9F0F7B3", "ink": "#0E1E2E", "ink2": "#2F4256", "muted": "#4A5D71", "line": "#1B3A5A1F",
    "accent": AIS_BLUE, "on_accent": "#FFFFFF",
    "accent_fills": [{"type": "linearGradient", "stops": ["#107FAD", "#0A5E86"], "from": [0, 0], "to": [1, 1]}],
    "accent_soft": "#D3ECF8CC", "accent_ink": "#0A5577", "link": "#0B6A93",
    "accent2": "#7B5CE5", "accent2_soft": "#E9E2FCCC", "accent2_ink": "#5434C4",
    "user_bubble": "#CFE8F6E6", "user_ink": "#0E1E2E",
    "th_fill": "#E2EEF7", "th_ink": "#0A4563", "row_line": "#1B3A5A17",
    "code_bg": "#0F2236E6", "code_ink": "#E6EDF5", "code_muted": "#9DB1C5", "code_kw": "#7DD3FC",
    "code_str": "#FDBA74", "code_num": "#C4B5FD", "code_head": "#FFFFFF14",
    "composer_fill": "#FFFFFFE6", "composer_stroke": "#FFFFFF",
    "composer_effects": [{"type": "backgroundBlur", "radius": 30},
                         {"type": "innerShadow", "color": "#FFFFFF", "offset": [0, 1], "radius": 0},
                         {"type": "dropShadow", "color": "#0B6E9814", "offset": [0, 4], "radius": 10},
                         {"type": "dropShadow", "color": "#1B3A5A29", "offset": [0, 20], "radius": 44}],
    "chip_fill": "#EEF4F9", "chip_ink": "#24364B",
    "overlay": "#C9DCEB66",
    "modal_fill": "#FFFFFFD1",
    "modal_effects": [{"type": "backgroundBlur", "radius": 50},
                      {"type": "innerShadow", "color": "#FFFFFF", "offset": [0, 1], "radius": 0},
                      {"type": "dropShadow", "color": "#1B3A5A33", "offset": [0, 30], "radius": 70}],
    "r_card": 18, "r_panel": 24, "r_composer": 24, "r_bubble": 20,
    "green": "#16875A",
}

GLAS_DARK = dict(GLAS, **{
    "dark": True,
    "backdrop": dgrad("#07111F", "#11132E"), "blobs": GLAS_BLOBS_DARK,
    "side_fill": "#0B1626A6",
    "side_ink": "#EEF4FA", "side_ink2": "#C7D3E0", "side_muted": "#9AABBD", "side_line": "#FFFFFF1A",
    "side_active": "#FFFFFF1F", "side_active_ink": "#FFFFFF", "side_search": "#FFFFFF14", "side_search_ink": "#A9B8C8",
    "side_new_fill": "#38A8DB", "side_new_ink": "#04131F", "side_stroke": "#FFFFFF24",
    "side_effects": [{"type": "backgroundBlur", "radius": 40},
                     {"type": "innerShadow", "color": "#FFFFFF1F", "offset": [0, 1], "radius": 0},
                     {"type": "dropShadow", "color": "#00000040", "offset": [0, 10], "radius": 30}],
    "main_fill": "#0D1A2BB8", "main_stroke": "#FFFFFF1F",
    "main_effects": [{"type": "backgroundBlur", "radius": 48},
                     {"type": "innerShadow", "color": "#FFFFFF24", "offset": [0, 1], "radius": 0},
                     {"type": "dropShadow", "color": "#00000052", "offset": [0, 16], "radius": 40}],
    "panel_fill": "#0D1A2BCC", "panel_stroke": "#FFFFFF1F",
    "surface": "#FFFFFF0F", "surface_stroke": "#FFFFFF1A",
    "card_effects": [{"type": "innerShadow", "color": "#FFFFFF14", "offset": [0, 1], "radius": 0}],
    "sunken": "#FFFFFF0D", "ink": "#EEF4FA", "ink2": "#C7D3E0", "muted": "#9AABBD", "line": "#FFFFFF1A",
    "accent": "#38A8DB", "on_accent": "#04131F",
    "accent_fills": [{"type": "linearGradient", "stops": ["#5CC3EE", "#2A93C6"], "from": [0, 0], "to": [1, 1]}],
    "accent_soft": "#38A8DB2E", "accent_ink": "#A6DDF5", "link": "#86CFF0",
    "accent2": "#A48BFF", "accent2_soft": "#A48BFF2E", "accent2_ink": "#CDBEFF",
    "user_bubble": "#2A7FB04D", "user_ink": "#F2F8FD",
    "th_fill": "#FFFFFF14", "th_ink": "#EEF4FA", "row_line": "#FFFFFF14",
    "code_bg": "#050C16B3", "code_head": "#FFFFFF0D",
    "composer_fill": "#13243ADB", "composer_stroke": "#FFFFFF2E",
    "composer_effects": [{"type": "backgroundBlur", "radius": 30},
                         {"type": "innerShadow", "color": "#FFFFFF29", "offset": [0, 1], "radius": 0},
                         {"type": "dropShadow", "color": "#0000005C", "offset": [0, 20], "radius": 44}],
    "chip_fill": "#FFFFFF14", "chip_ink": "#DCE6F0",
    "green": "#3CCB91",
})


def without_backdrop_blur(t):
    """The same tokens with every backgroundBlur removed: what egui draws on its own, without a
    compositor. The backdrop stays (it is a static, pre-blurred image egui can paint as a texture)."""
    out = dict(t)
    for k, v in t.items():
        if k.endswith("effects") and isinstance(v, list):
            out[k] = [e for e in v if e.get("type") != "backgroundBlur"]
    out["name"] = t["name"] + " ohne Unschärfe"
    return out


GLAS_EGUI = without_backdrop_blur(GLAS)
