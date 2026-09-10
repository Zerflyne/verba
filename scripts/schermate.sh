#!/usr/bin/env bash
# Le schermate del README, senza toccare niente a mano.
#
# L'interfaccia viene aperta in Chrome headless con il banco di prova
# (`?banco=<sezione>`), che la porta gia' a lavoro fatto. Serve il server di
# sviluppo in ascolto:
#
#     npm run dev --prefix ui &
#     scripts/schermate.sh
set -euo pipefail

CHROME=${CHROME:-google-chrome}
URL=${URL:-http://localhost:5173}
USCITA=${USCITA:-assets/schermate}
TEMPO=${TEMPO:-1.7}

mkdir -p "$USCITA"
for sezione in carica modifica esporta impostazioni; do
    "$CHROME" --headless=new --disable-gpu --hide-scrollbars \
        --run-all-compositor-stages-before-draw \
        --window-size=1600,980 \
        --virtual-time-budget=20000 \
        --screenshot="$USCITA/$sezione.png" \
        "$URL/?banco=$sezione&t=$TEMPO" 2>/dev/null
    echo "$USCITA/$sezione.png"
done
