#!/usr/bin/env python3
"""Disegna i diagrammi di `docs/tempi.md`.

Le finestre di accensione non sono inventate: questa funzione e' la traduzione
riga per riga di `calcola_finestre` in `crates/verba-core/src/layout.rs`. Se
quella cambia, va cambiata anche questa e le figure vanno rigenerate:

    python3 scripts/figure_tempi.py
"""

from pathlib import Path

USCITA = Path("assets/figure")

# Palette della spec.
BG = "#171717"
BORDO = "#2A2A2A"
TESTO = "#EDEDED"
DIM = "#8C8C8C"
ACCENT = "#A78BFA"
STRONG = "#8B5CF6"

# Un caso con tutto quello che serve: parole attaccate, un silenzio lungo, e
# una fine di riga.
PAROLE = [
    ("Ciao", 0.00, 0.35),
    ("sono", 0.40, 0.90),
    ("Federico", 0.95, 1.40),
    ("e", 2.60, 3.00),
    ("questo", 3.05, 3.60),
]
DURATA = 4.2


def finestre(parole, anticipo, pausa_max, coda, tenuta):
    """La traduzione di `calcola_finestre` piu' `assesta_tempi`."""
    start = max(parole[0][1] - anticipo, 0.0)
    end = parole[-1][2] + tenuta
    out = []
    for i, (_, ini, fin) in enumerate(parole):
        inizio = max(ini - anticipo, start)
        if i + 1 < len(parole):
            succ = parole[i + 1][1]
            fine = min(fin + pausa_max, max(succ - anticipo, inizio))
        else:
            fine = min(fin + coda, end)
        out.append((inizio, max(fine, inizio)))
    return start, end, out


def diagramma(nome, titolo, casi, evidenzia=None):
    """`casi` e' una lista di (etichetta, anticipo, pausa_max, coda, tenuta)."""
    larghezza = 760
    sinistra = 150
    destra = 24
    utile = larghezza - sinistra - destra
    alto = 46
    riga_h = 58
    altezza = alto + riga_h * len(casi) + 46

    def x(t):
        return sinistra + utile * t / DURATA

    p = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{larghezza}" height="{altezza}" '
        f'viewBox="0 0 {larghezza} {altezza}" font-family="Inter, system-ui, sans-serif">',
        f'<rect width="{larghezza}" height="{altezza}" fill="{BG}" rx="10"/>',
        f'<text x="20" y="26" fill="{TESTO}" font-size="14">{titolo}</text>',
    ]

    # La riga del parlato: dove ci sono parole e dove c'e' silenzio.
    y = alto
    p.append(f'<text x="20" y="{y + 14}" fill="{DIM}" font-size="11">parlato</text>')
    for testo, ini, fin in PAROLE:
        p.append(
            f'<rect x="{x(ini):.1f}" y="{y}" width="{x(fin) - x(ini):.1f}" height="18" '
            f'rx="3" fill="#2F2F2F"/>'
        )
        p.append(
            f'<text x="{(x(ini) + x(fin)) / 2:.1f}" y="{y + 13}" fill="{DIM}" '
            f'font-size="10" text-anchor="middle">{testo}</text>'
        )

    for k, (etichetta, anticipo, pausa_max, coda, tenuta) in enumerate(casi):
        y = alto + riga_h * (k + 1)
        start, end, fin_win = finestre(PAROLE, anticipo, pausa_max, coda, tenuta)
        p.append(f'<text x="20" y="{y + 14}" fill="{TESTO}" font-size="11">{etichetta}</text>')

        # La riga a schermo, sotto: dice fin dove il testo resta visibile.
        p.append(
            f'<rect x="{x(start):.1f}" y="{y + 22}" width="{x(end) - x(start):.1f}" height="5" '
            f'rx="2" fill="#3A3A3A"/>'
        )

        for i, (a, b) in enumerate(fin_win):
            colore = STRONG if (evidenzia is None or i in evidenzia) else "#4C3F70"
            p.append(
                f'<rect x="{x(a):.1f}" y="{y}" width="{max(x(b) - x(a), 1.5):.1f}" height="18" '
                f'rx="3" fill="{colore}"/>'
            )

    # L'asse dei tempi.
    y = altezza - 22
    p.append(f'<line x1="{sinistra}" y1="{y}" x2="{x(DURATA):.1f}" y2="{y}" stroke="{BORDO}"/>')
    for t in range(0, int(DURATA) + 1):
        p.append(f'<line x1="{x(t):.1f}" y1="{y - 3}" x2="{x(t):.1f}" y2="{y + 3}" stroke="{BORDO}"/>')
        p.append(
            f'<text x="{x(t):.1f}" y="{y + 16}" fill="{DIM}" font-size="10" '
            f'text-anchor="middle">{t}s</text>'
        )
    p.append(
        f'<text x="{larghezza - destra}" y="26" fill="{ACCENT}" font-size="10" '
        f'text-anchor="end">viola = evidenziazione accesa</text>'
    )
    p.append("</svg>")
    USCITA.mkdir(parents=True, exist_ok=True)
    (USCITA / f"{nome}.svg").write_text("\n".join(p), encoding="utf-8")
    print(f"{USCITA / (nome + '.svg')}")


D = dict(anticipo=0.06, pausa_max=0.60, coda=0.40, tenuta=0.30)

diagramma(
    "tempi-anticipo",
    "Anticipo — quanto l'evidenziazione precede la parola",
    [
        ("0 ms", 0.0, D["pausa_max"], D["coda"], D["tenuta"]),
        ("60 ms (default)", 0.06, D["pausa_max"], D["coda"], D["tenuta"]),
        ("200 ms", 0.20, D["pausa_max"], D["coda"], D["tenuta"]),
    ],
)

diagramma(
    "tempi-pausa",
    "Tetto alla pausa — quanto resta accesa dentro un silenzio",
    [
        ("0 ms", D["anticipo"], 0.0, D["coda"], D["tenuta"]),
        ("600 ms (default)", D["anticipo"], 0.60, D["coda"], D["tenuta"]),
        ("2000 ms", D["anticipo"], 2.0, D["coda"], D["tenuta"]),
    ],
    evidenzia={2},
)

diagramma(
    "tempi-coda",
    "Coda — quanto resta accesa dopo l'ultima parola della riga",
    [
        ("0 ms", D["anticipo"], D["pausa_max"], 0.0, D["tenuta"]),
        ("400 ms (default)", D["anticipo"], D["pausa_max"], 0.40, D["tenuta"]),
        ("2000 ms, oltre la tenuta", D["anticipo"], D["pausa_max"], 2.0, D["tenuta"]),
    ],
    evidenzia={4},
)
