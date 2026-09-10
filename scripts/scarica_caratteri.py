#!/usr/bin/env python3
"""Prepara i caratteri di serie di Verba in `assets/fonts`.

I caratteri vengono da Google Fonts (licenza OFL, ridistribuibile). Quelli che
Google pubblica solo in forma variabile vengono istanziati nei pesi che
servono: cosmic-text sceglie il carattere per peso dichiarato, e da un file
variabile leggerebbe un peso solo.

    pip install fonttools
    python3 scripts/scarica_caratteri.py assets/fonts
"""
import io
import sys
import pathlib
import urllib.request

GREZZO = "https://raw.githubusercontent.com/google/fonts/main/ofl"

# famiglia -> (cartella nel repository, file sorgente, pesi da produrre)
# Se `pesi` e' None il file e' gia' statico e si copia com'e'.
CARATTERI = [
    ("Inter",      "inter",      "Inter[opsz,wght].ttf",   [400, 700, 900]),
    ("Montserrat", "montserrat", "Montserrat[wght].ttf",   [400, 700, 900]),
    ("Oswald",     "oswald",     "Oswald[wght].ttf",       [400, 700]),
    ("Poppins",    "poppins",    "Poppins-Regular.ttf",    None),
    ("Poppins",    "poppins",    "Poppins-Bold.ttf",       None),
    ("Poppins",    "poppins",    "Poppins-Black.ttf",      None),
    ("Anton",      "anton",      "Anton-Regular.ttf",      None),
    ("Bebas Neue", "bebasneue",  "BebasNeue-Regular.ttf",  None),
]

NOMI_PESO = {100: "Thin", 200: "ExtraLight", 300: "Light", 400: "Regular",
             500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold",
             900: "Black"}


def scarica(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read()


def main(dest: pathlib.Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    for famiglia, cartella, file, pesi in CARATTERI:
        url = f"{GREZZO}/{cartella}/{urllib.parse.quote(file)}"
        dati = scarica(url)

        if pesi is None:
            (dest / file).write_bytes(dati)
            print(f"{file:<30} {len(dati):>8} byte")
            continue

        from fontTools import ttLib
        from fontTools.varLib import instancer
        senza_spazi = famiglia.replace(" ", "")
        for peso in pesi:
            font = ttLib.TTFont(io.BytesIO(dati))
            statico = instancer.instantiateVariableFont(
                font, {"wght": peso}, inplace=False, updateFontNames=True
            )
            nome = f"{senza_spazi}-{NOMI_PESO[peso]}.ttf"
            statico.save(dest / nome)
            print(f"{nome:<30} {(dest / nome).stat().st_size:>8} byte  (da variabile)")

    # Le licenze accompagnano i caratteri: OFL lo richiede.
    licenze = dest / "licenze"
    licenze.mkdir(exist_ok=True)
    for cartella in sorted({c for _, c, _, _ in CARATTERI}):
        (licenze / f"{cartella}-OFL.txt").write_bytes(
            scarica(f"{GREZZO}/{cartella}/OFL.txt")
        )
    print(f"licenze in {licenze}")


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "assets/fonts"))
