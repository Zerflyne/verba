#!/usr/bin/env python3
"""Banco di prova per la webview: riproduce le condizioni del pacchetto.

Serve a non ripetere l'errore che e' costato tre tentativi sull'audio
dell'anteprima. Il difetto si vedeva solo nel `.deb`, e ogni verifica era
stata fatta in sviluppo — dove la pagina la serve Vite, senza CSP e senza
schema personalizzato, cioe' nell'unico ambiente in cui il difetto non
esiste. Ricostruire *quelle* condizioni senza compilare niente rende la
verifica una questione di secondi invece di un giro di CI.

Usa la stessa WebKitGTK 4.1 di Tauri v2, ed e' fedele al programma vero nei
punti che contano:

  * la pagina arriva da `tauri://localhost`, con lo schema registrato **come
    sicuro** (lo fa wry);
  * la riproduzione automatica e' permessa (wry imposta
    `AutoplayPolicy::Allow`; senza, ogni prova cade su `NotAllowedError` e non
    si misura l'audio ma la politica di autoplay);
  * la CSP arriva come header HTTP, non come `<meta>`;
  * lo script e' un file servito dallo stesso schema, perche' con una CSP
    senza `'unsafe-inline'` uno script inline non partirebbe affatto.

Prova quattro cose, e la terza e' quella che ha trovato il difetto:

  1. `blob:` con byte veri passato a un `<audio>` — funziona;
  2. gli stessi byte in WebAudio — funziona;
  3. un blob costruito da un **array di numeri** invece che da byte: pesa tre
     volte e mezzo tanto e da' `codice 4` con
     «NotSupportedError: The operation is not supported», che era esattamente
     l'errore riportato. E' quello che succede quando l'IPC ripiega su
     `postMessage`;
  4. una `fetch` verso `ipc://localhost`, per vedere se la CSP la blocca: se la
     blocca, Tauri ripiega e si ricade nel caso 3.

Uso:

    python3 scripts/banco_webview.py "<csp>" [secondi_di_audio]
    python3 scripts/banco_webview.py ""            # nessuna CSP, controllo

La CSP del programma sta in `crates/verba-app/tauri.conf.json`. Serve
`python3-gi` con `WebKit2 4.1` e un display.
"""

import base64, io, math, struct, sys, wave
import gi
gi.require_version("Soup", "3.0")
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gio, Gtk, Soup, WebKit2, GLib

CSP = sys.argv[1] if len(sys.argv) > 1 else ""
# La durata conta: un WAV piccolo la webview lo consegna in un colpo, uno
# grande lo consegna a pezzi con richieste Range — ed e' li' che si rompeva.
SECONDI = int(sys.argv[2]) if len(sys.argv) > 2 else 1

# Un WAV come quello che manda il motore: 16 kHz, mono, PCM a 16 bit.
buf = io.BytesIO()
with wave.open(buf, "wb") as w:
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
    w.writeframes(b"".join(
        struct.pack("<h", int(3000 * math.sin(2 * math.pi * 440 * i / 16000)))
        for i in range(16000 * SECONDI)))
WAV = base64.b64encode(buf.getvalue()).decode()

PAGINA = """<!doctype html><meta charset=utf-8><body><script src="/banco.js"></script></body>"""

# Nel programma vero gli script sono file serviti dallo stesso schema, non
# inline: con una CSP senza `'unsafe-inline'` uno script inline non parte
# nemmeno, e il banco misurerebbe quello invece dell'audio.
COPIONE = """
const di = (m) => window.webkit.messageHandlers.esito.postMessage(String(m));
window.onerror = (m) => di("ECCEZIONE: " + m);
document.addEventListener("securitypolicyviolation",
  (e) => di("CSP VIOLATA: " + e.violatedDirective + " su " + e.blockedURI));
const byte = Uint8Array.from(atob("%s"), (c) => c.charCodeAt(0));
di("origine della pagina: " + location.origin);
di("byte del WAV: " + byte.length);

let fatti = 0;
const finito = () => { if (++fatti === 4) setTimeout(() => di("FINE"), 400); };

// --- la causa vera: la CSP permette all'IPC di usare il proprio canale? ---
// `ipc:` qui non e' registrato, quindi la fetch fallisce in ogni caso: quello
// che conta e' *come*. Bloccata dalla CSP significa che Tauri ripiega su
// postMessage e i byte grezzi diventano numeri.
fetch("ipc://localhost/traccia_audio", {method: "POST"})
  .then(() => { di("IPC: fetch riuscita"); finito(); })
  .catch((e) => { di("IPC: fetch fallita (" + e.message + ")"); finito(); });

// --- il sospetto: i byte arrivati come Array di numeri, non come buffer ---
const comeArray = Array.from(byte);
const bTesto = new Blob([comeArray], {type: "audio/wav"});
di("ARRAY: il blob pesa " + bTesto.size + " byte invece di " + byte.length +
   " (" + (bTesto.size / byte.length).toFixed(2) + " volte)");
const t = new Audio();
t.src = URL.createObjectURL(bTesto);
t.onerror = () => { di("ARRAY: RIFIUTATO, codice " + (t.error && t.error.code)); finito(); };
t.onplaying = () => { di("ARRAY: sta suonando"); finito(); };
t.play().catch((e) => di("ARRAY: play() " + e.name + ": " + e.message));

// --- come era: un blob passato a un <audio> ---
const a = new Audio();
a.src = URL.createObjectURL(new Blob([byte], {type: "audio/wav"}));
di("indirizzo del blob: " + a.src);
a.onerror = () => { di("BLOB: RIFIUTATO, codice " + (a.error && a.error.code)); finito(); };
a.oncanplay = () => di("BLOB: sorgente accettata");
a.onplaying = () => { di("BLOB: sta suonando"); finito(); };
a.play().catch((e) => { di("BLOB: play() " + e.name + ": " + e.message); finito(); });

// --- come e' adesso: i byte in WebAudio, senza indirizzo ---
const ctx = new AudioContext();
ctx.decodeAudioData(byte.buffer.slice(0)).then(async (suono) => {
  di("WEBAUDIO: decodificato, " + suono.duration.toFixed(3) + " s");
  const g = ctx.createGain(); g.gain.value = 0.02; g.connect(ctx.destination);
  const s = ctx.createBufferSource(); s.buffer = suono; s.connect(g);
  s.start(0, 0);
  if (ctx.state === "suspended") await ctx.resume();
  const partenza = ctx.currentTime;
  di("WEBAUDIO: partito, stato " + ctx.state);
  setTimeout(() => {
    di("WEBAUDIO: orologio avanzato di " +
       (ctx.currentTime - partenza).toFixed(3) + " s in mezzo secondo");
    finito();
  }, 500);
}).catch((e) => { di("WEBAUDIO: RIFIUTATO: " + e); finito(); });
""" % WAV

def servi(richiesta, *_):
    js = richiesta.get_path().endswith(".js")
    d = GLib.Bytes.new((COPIONE if js else PAGINA).encode())
    flusso = Gio.MemoryInputStream.new_from_bytes(d)
    risposta = WebKit2.URISchemeResponse.new(flusso, d.get_size())
    risposta.set_content_type("text/javascript" if js else "text/html")
    if CSP:
        intestazioni = Soup.MessageHeaders.new(Soup.MessageHeadersType.RESPONSE)
        intestazioni.append("Content-Security-Policy", CSP)
        risposta.set_http_headers(intestazioni)
    richiesta.finish_with_response(risposta)

ctx = WebKit2.WebContext.get_default()
ctx.get_security_manager().register_uri_scheme_as_secure("tauri")
ctx.register_uri_scheme("tauri", servi)

# Le politiche del sito si danno alla costruzione, non dopo.
vista = WebKit2.WebView(
    web_context=ctx,
    website_policies=WebKit2.WebsitePolicies(autoplay=WebKit2.AutoplayPolicy.ALLOW))
cm = vista.get_user_content_manager()
cm.register_script_message_handler("esito")

def detto(_, risultato):
    testo = risultato.to_string() if hasattr(risultato, "to_string") else \
            risultato.get_js_value().to_string()
    print(testo, flush=True)
    if testo == "FINE":
        GLib.timeout_add(200, Gtk.main_quit)

cm.connect("script-message-received::esito", detto)

f = Gtk.Window(title="banco audio"); f.set_default_size(320, 120)
f.add(vista); f.show_all()
vista.load_uri("tauri://localhost/")
GLib.timeout_add_seconds(20, Gtk.main_quit)
Gtk.main()
