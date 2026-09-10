/** Il banco di prova: cosa risponde il ponte quando Tauri non c'e'.
 *
 *  Serve a due cose e a nessun'altra: lavorare sull'aspetto della finestra
 *  senza ricompilare il motore, e poter aprire l'interfaccia in un browser su
 *  una macchina che non ha GTK e WebKit installati.
 *
 *  Tutto quello che sta qui e' finto e non tocca nessun file. In particolare
 *  il fotogramma d'anteprima e' **disegnato in JavaScript**: nell'applicazione
 *  vera non lo e', e non deve mai diventarlo — l'anteprima esce dallo stesso
 *  codice dell'export, altrimenti le due immagini divergono. */

import type {
  Descrizione,
  EsitoExport,
  Evento,
  Famiglia,
  FormatiDisponibili,
  Impostazioni,
  Info,
  NomeFase,
  ParolaVista,
  Preset,
  Riepilogo,
  SchedaVista,
  StatoModelli,
  TerminiVisti,
} from "./tipi";

const TESTO =
  "Ciao sono Federico Costantini e questo è un audio di prova per verificare che i sottotitoli funzionino correttamente, iscriviti al canale.";

const PAROLE: ParolaVista[] = (() => {
  const parole = TESTO.split(" ");
  const inizio = 0.37;
  const fine = 8.71;
  const passo = (fine - inizio) / parole.length;
  return parole.map((testo, i) => ({
    id: i + 1,
    testo,
    inizio: inizio + i * passo,
    fine: inizio + (i + 1) * passo - 0.02,
    // Qualche parola incerta, per vedere come si segnalano.
    confidenza: i === 3 || i === 15 ? 0.31 : 0.86,
    incerta: i === 3 || i === 15,
  }));
})();

const DESCRIZIONE: Descrizione = {
  percorso: "/esempio/video_tutorial.mp4",
  nome: "video_tutorial.mp4",
  modalita: "video",
  larghezza: 1920,
  altezza: 1080,
  fps: 25,
  durata: 9.66,
  codec_video: "h264",
  codec_audio: "aac",
  riassunto: "1920x1080 · 25.00 fps · 00:09",
};

export const presetFinti: Preset[] = [
  preset("Verticale", "verticale", 1),
  preset("Orizzontale", "orizzontale", 2),
  { ...preset("Sobrio", "dal_sorgente", 2), evidenziazione: { forma: "nessuna", padding: 0.18, altezza: 1.12, raggio: 0.2, spessore_sottolineatura: 0.1 } },
];

function preset(nome: string, formato: Preset["posizione"]["formato"], righe: number): Preset {
  return {
    versione: 1,
    nome,
    testo: { carattere: "Inter", peso: 700, corpo: null, maiuscole: false, interlinea: 1.18 },
    colori: {
      testo: "#FFFFFF",
      testo_attivo: "#FFFFFF",
      evidenziazione: "#8B5CF6",
      bordo: "#000000",
      bordo_px: 3,
      ombra: true,
      colore_ombra: "#000000A0",
      ombra_spostamento: 0.05,
      ombra_sfocatura: 0.08,
    },
    evidenziazione: {
      forma: "rettangolo",
      padding: 0.18,
      altezza: 1.12,
      raggio: 0.2,
      spessore_sottolineatura: 0.1,
    },
    posizione: {
      formato,
      verticale: 0.82,
      orizzontale: 0.5,
      larghezza_max: 0.8,
      margine: 0.05,
      righe_max: righe,
      allineamento: "centro",
    },
    tempi: {
      anticipo_ms: 60,
      pausa_massima_ms: 600,
      coda_ms: 400,
      durata_minima_parola_ms: 80,
      durata_blocco: 5,
      pausa_blocco: 0.7,
      tenuta: 0.3,
    },
  };
}

let impostazioniFinte: Impostazioni = {
  versione: 1,
  modello: "large-v3",
  lingua: "it",
  dispositivo: "automatico",
  gpu: null,
  termini: null,
  soglia: 0.5,
  cartella_export: null,
  cartella_modelli: null,
  caratteri_aggiunti: [],
  caratteri_di_sistema: false,
  ultimo_formato: null,
};

let terminiFinti: TerminiVisti = {
  percorso: "~/.local/share/verba/termini.csv",
  esiste: true,
  termini: ["Federico Costantini", "Anthropic", "Zerflyne", "whisper.cpp", "wav2vec2"],
};

const CARATTERI: Famiglia[] = [
  { nome: "Anton", pesi: [400], di_serie: true },
  { nome: "Bebas Neue", pesi: [400], di_serie: true },
  { nome: "Inter", pesi: [400, 700, 900], di_serie: true },
  { nome: "Montserrat", pesi: [400, 700, 900], di_serie: true },
  { nome: "Oswald", pesi: [400, 700], di_serie: true },
  { nome: "Poppins", pesi: [400, 700, 900], di_serie: true },
];

const MODELLI: StatoModelli = {
  cartella: "~/.local/share/verba/models",
  pronto: true,
  da_scaricare: 0,
  da_scaricare_leggibile: "0 B",
  a_mano: [],
  modelli: [
    m("large-v3", "Whisper large-v3", "2.9 GB", true, true, "Il modello di trascrizione. Formato GGML per whisper.cpp."),
    m("medium", "Whisper medium", "1.4 GB", false, false, "Trascrizione a meta' strada fra qualita' e velocita'."),
    m("small", "Whisper small", "465 MB", false, false, "Trascrizione rapida, per una bozza o per una macchina modesta."),
    m("segmentazione", "pyannote segmentation 3.0", "5.7 MB", true, true, "Trova dove c'e' parlato: e' quello che divide l'audio in segmenti."),
    m("vocabolario", "Vocabolario wav2vec2-italian", "410 B", true, true, "Da qui il programma deduce blank CTC e delimitatore di parola."),
    m("allineamento", "wav2vec2-italian (CTC)", "1.2 GB", true, true, "Da' il tempo esatto di ogni parola."),
  ],
};

/** Lo stesso elenco, ma con i modelli non ancora scaricati.
 *
 *  Si chiede con `?modelli=mancanti`, ed e' l'unico modo di guardare la
 *  scheda d'avviso senza cancellare per davvero tre gigabyte di file. */
const MODELLI_MANCANTI: StatoModelli = {
  cartella: "~/.local/share/verba/models",
  pronto: false,
  da_scaricare: 3_101_000_000,
  da_scaricare_leggibile: "2,9 GB",
  a_mano: [
    "wav2vec2-italian (CTC) va esportato a mano:\n" +
      "  python scripts/export_models.py --w2v",
  ],
  modelli: [
    m("large-v3", "Whisper large-v3", "2.9 GB", false, true, "Il modello di trascrizione. Formato GGML per whisper.cpp."),
    m("medium", "Whisper medium", "1.4 GB", false, false, "Trascrizione a meta' strada fra qualita' e velocita'."),
    m("small", "Whisper small", "465 MB", false, false, "Trascrizione rapida, per una bozza o per una macchina modesta."),
    m("segmentazione", "pyannote segmentation 3.0", "5.7 MB", false, true, "Trova dove c'e' parlato: e' quello che divide l'audio in segmenti."),
    m("vocabolario", "Vocabolario wav2vec2-italian", "410 B", true, true, "Da qui il programma deduce blank CTC e delimitatore di parola."),
    m("allineamento", "wav2vec2-italian (CTC)", "1.2 GB", false, true, "Da' il tempo esatto di ogni parola."),
  ],
};

/** Quale dei due elenchi mostrare, secondo l'indirizzo. */
function modelliDaMostrare(): StatoModelli {
  return new URLSearchParams(window.location.search).get("modelli") === "mancanti"
    ? MODELLI_MANCANTI
    : MODELLI;
}

function m(
  id: string,
  nome: string,
  leggibile: string,
  presente: boolean,
  in_uso: boolean,
  spiegazione: string,
) {
  return {
    id,
    nome,
    file: `${id}.bin`,
    spiegazione,
    byte: 0,
    leggibile,
    presente,
    ripresa: 0,
    si_scarica: id !== "allineamento",
    comando: id === "allineamento" ? "python scripts/export_models.py --w2v" : null,
    in_uso,
  };
}

const FORMATI: FormatiDisponibili = {
  video: [
    { id: "h264", etichetta: "Video sottotitolato", descrizione: "H.264 CRF 18: i sottotitoli impressi, si riproduce ovunque", estensione: "mp4", alfa: false },
    { id: "prores422", etichetta: "Video sottotitolato senza perdita", descrizione: "ProRes 422 HQ: i sottotitoli impressi, senza perdita, per rimontare", estensione: "mov", alfa: false },
    { id: "prores4444", etichetta: "Overlay trasparente", descrizione: "ProRes 4444 con canale alfa: solo i sottotitoli, da sovrapporre in montaggio", estensione: "mov", alfa: true },
    { id: "vp9", etichetta: "Overlay trasparente compatto", descrizione: "VP9 con alfa: come l'overlay ProRes ma centinaia di volte piu' leggero", estensione: "webm", alfa: true },
  ],
  testo: [
    { id: "srt", etichetta: "Sottotitoli", descrizione: "Un blocco per riga mostrata. Il formato che legge chiunque.", estensione: "srt" },
    { id: "vtt", etichetta: "Sottotitoli WebVTT", descrizione: "Come l'SRT, nel formato che vogliono i lettori video del web.", estensione: "vtt" },
    { id: "json", etichetta: "Parola per parola", descrizione: "Testo, inizio, fine e confidenza di ogni singola parola.", estensione: "json" },
    { id: "txt", etichetta: "Solo testo", descrizione: "La trascrizione senza tempi, una battuta per riga.", estensione: "txt" },
  ],
};

/** L'onda: rumore riproducibile, con le pause dove ci sono davvero. */
const ONDA: number[] = (() => {
  const n = 1600;
  const v: number[] = [];
  let seme = 7;
  for (let i = 0; i < n; i++) {
    seme = (seme * 1103515245 + 12345) & 0x7fffffff;
    const casuale = (seme / 0x7fffffff) * 0.55 + 0.2;
    const t = (i / n) * DESCRIZIONE.durata;
    const parla = PAROLE.some((p) => t >= p.inizio - 0.05 && t <= p.fine + 0.05);
    v.push(parla ? Math.min(1, casuale * 1.5) : casuale * 0.12);
  }
  return v;
})();

function emetti(e: Evento) {
  const f = (window as unknown as { __verbaEventi?: (e: Evento) => void }).__verbaEventi;
  if (f) f(e);
}

/** Finge una fase che dura `ms`, con l'avanzamento che sale. */
function fase(nome: NomeFase, ms: number): Promise<void> {
  return new Promise((risolvi) => {
    emetti({ evento: "iniziata", fase: nome });
    const avvio = performance.now();
    // Un intervallo e non `requestAnimationFrame`: il banco di prova deve
    // avanzare anche quando la finestra non e' in primo piano, altrimenti
    // guardare l'interfaccia da fuori la lascia ferma a meta'.
    const id = setInterval(() => {
      const q = Math.min(1, (performance.now() - avvio) / ms);
      emetti({ evento: "avanzamento", fase: nome, frazione: q });
      if (q >= 1) {
        clearInterval(id);
        emetti({ evento: "conclusa", fase: nome, secondi: ms / 1000 });
        risolvi();
      }
    }, 60);
  });
}

/** Lo stato in cui aprire l'interfaccia, letto dall'indirizzo.
 *
 *  `?banco=carica&t=1.2` apre la sezione Carica con il file gia' trascritto e
 *  il cursore a 1,2 secondi. Serve a fare gli scatti per il README senza
 *  cliccare a mano, e vale **solo** senza Tauri. */
/** Vero con `?audio=1`: il banco finge un file di solo audio. */
export function soloAudioFinto(): boolean {
  return new URLSearchParams(window.location.search).get("audio") === "1";
}

export function scenaIniziale(): { sezione: string; tempo: number } | null {
  const q = new URLSearchParams(window.location.search);
  const sezione = q.get("banco");
  if (!sezione) return null;
  return { sezione, tempo: Number(q.get("t") ?? "1.2") };
}

export async function bancoDiProva<T>(
  comando: string,
  argomenti?: Record<string, unknown>,
): Promise<T> {
  const q = (x: unknown) => x as T;
  switch (comando) {
    case "informazioni":
      return q({
        versione: "0.1.0",
        repository: "https://github.com/zerflyne/verba",
        licenza: "MIT",
        provider: "GPU NVIDIA (CUDA)",
        dispositivo: "CUDA:0 (NVIDIA GeForce RTX 4060, 8188 MiB totali)",
        cartella_modelli: "~/.local/share/verba/models",
        cartella_dati: "~/.local/share/verba",
      } satisfies Info);
    case "impostazioni":
      return q(impostazioniFinte);
    case "salva_impostazioni":
      impostazioniFinte = argomenti!.nuove as Impostazioni;
      return q(undefined);
    case "stato_modelli":
      return q(modelliDaMostrare());
    case "gpu_disponibili":
      return q([
        {
          indice: 0,
          nome: "NVIDIA GeForce RTX 4060",
          totale_mib: 8188,
          libera_mib: 7421,
          etichetta: "CUDA:0 — NVIDIA GeForce RTX 4060, 8188 MiB",
        },
        {
          indice: 1,
          nome: "Tesla P40",
          totale_mib: 23040,
          libera_mib: 22901,
          etichetta: "CUDA:1 — Tesla P40, 23040 MiB",
        },
      ] satisfies SchedaVista[]);
    case "termini":
      return q(terminiFinti);
    case "termini_salva":
      terminiFinti = {
        percorso: terminiFinti.percorso,
        esiste: true,
        termini: argomenti!.elenco as string[],
      };
      return q(terminiFinti);
    case "scarica_modelli":
      await fase("scaricamento", 1400);
      return q(MODELLI);
    case "traccia_audio":
      // Il banco non ha un file da suonare: `sorgenteAudio` non arriva
      // nemmeno qui, ma se ci arrivasse deve dirlo invece di mentire.
      throw new Error("il banco di prova non ha una traccia da riprodurre");
    case "file_da_aprire":
      return q(null);
    case "apri":
      if (soloAudioFinto()) {
        await fase("preparazione", 350);
        return q({
          ...DESCRIZIONE,
          nome: "intervista.mp3",
          modalita: "audio" as const,
          larghezza: 0,
          altezza: 0,
          riassunto: "audio mp3 · 00:09",
        });
      }
      await fase("preparazione", 350);
      return q(DESCRIZIONE);
    case "chiudi":
      return q(undefined);
    case "trascrivi":
      await fase("segmentazione", 500);
      await fase("trascrizione", 1600);
      await fase("allineamento", 700);
      await fase("pulizia", 200);
      await fase("impaginazione", 200);
      return q({
        parole: PAROLE.length,
        incerte: PAROLE.filter((p) => p.incerta).length,
        blocchi: 3,
        secondi: 3.2,
        dispositivo: "CUDA:0 (NVIDIA GeForce RTX 4060)",
        modello: "ggml-large-v3.bin",
      } satisfies Riepilogo);
    case "annulla":
      emetti({ evento: "annullata" });
      return q(undefined);
    case "preset_corrente":
      return q(presetFinti[1]);
    case "preset_di_serie":
      return q(presetFinti);
    case "preset_carica":
      return q(presetFinti[0]);
    case "preset_salva":
      return q(undefined);
    case "applica_aspetto":
      return q(null);
    case "dimensioni":
      // `?audio=1` finge un file di solo audio: la scena prende le misure del
      // preset verticale invece di quelle del filmato. Serve a guardare
      // l'anteprima in quel caso senza dover trascrivere un mp3 per davvero.
      return q(soloAudioFinto() ? [1080, 1920] : [DESCRIZIONE.larghezza, DESCRIZIONE.altezza]);
    case "onda":
      return q(ONDA);
    case "parole":
      return q(PAROLE);
    case "finestra": {
      const t = argomenti!.t as number;
      const quante = argomenti!.quante as number;
      let centro = PAROLE.findIndex((p) => t < p.fine);
      if (centro < 0) centro = PAROLE.length - 1;
      const da = Math.max(0, Math.min(centro - Math.floor(quante / 2), PAROLE.length - quante));
      return q(PAROLE.slice(da, da + quante));
    }
    case "caratteri":
      return q(CARATTERI);
    case "aggiungi_carattere":
      return q(CARATTERI);
    case "formati":
      return q(FORMATI);
    case "nome_proposto":
      return q(`/esempio/video_tutorial_sub.${argomenti!.formato === "h264" ? "mp4" : "mov"}`);
    case "esporta":
      await fase("codifica", 1800);
      return q({
        percorso: argomenti!.percorso as string,
        fotogrammi: 242,
        secondi: 3.04,
      } satisfies EsitoExport);
    case "esporta_testo":
      return q(argomenti!.percorso as string);
    case "mostra_nella_cartella":
      return q(undefined);
    default:
      throw new Error(`comando «${comando}» non previsto dal banco di prova`);
  }
}

/** Il fotogramma finto: uno sfondo che somiglia a un video, e i sottotitoli.
 *
 *  Ripeto quello che dice l'intestazione, perche' e' il punto in cui e' piu'
 *  facile sbagliarsi: **nell'applicazione vera questo disegno non esiste**. Il
 *  motore manda i pixel gia' pronti. */
export function fotogrammaFinto(t: number, larghezza: number, altezza: number): ImageData {
  const c = document.createElement("canvas");
  c.width = larghezza;
  c.height = altezza;
  const g = c.getContext("2d")!;

  // Un paesaggio sfumato: basta a capire se i sottotitoli si leggono sopra.
  const cielo = g.createLinearGradient(0, 0, 0, altezza);
  cielo.addColorStop(0, "#5d6a75");
  cielo.addColorStop(0.55, "#8d9aa2");
  cielo.addColorStop(1, "#3c4650");
  g.fillStyle = cielo;
  g.fillRect(0, 0, larghezza, altezza);

  g.fillStyle = "rgba(30,38,46,0.75)";
  for (const [x, y, w] of [
    [0.05, 0.62, 0.4],
    [0.3, 0.55, 0.5],
    [0.62, 0.6, 0.45],
  ] as const) {
    g.beginPath();
    g.moveTo(x * larghezza, altezza);
    g.lineTo((x + w / 2) * larghezza, y * altezza);
    g.lineTo((x + w) * larghezza, altezza);
    g.closePath();
    g.fill();
  }
  g.fillStyle = "rgba(22,28,34,0.9)";
  g.fillRect(0, altezza * 0.78, larghezza, altezza * 0.22);

  // I sottotitoli, con la parola in corso sotto il rettangolo violetto.
  const corpo = Math.round(altezza * 0.065);
  g.font = `700 ${corpo}px Inter, sans-serif`;
  g.textBaseline = "middle";

  const attiva = PAROLE.find((p) => t >= p.inizio && t < p.fine);
  const blocco = PAROLE.filter((p) => p.inizio < t + 2.2 && p.fine > t - 1.0).slice(0, 6);
  const mostrate = blocco.length ? blocco : PAROLE.slice(0, 5);

  const spazio = g.measureText(" ").width;
  const larghezze = mostrate.map((p) => g.measureText(p.testo).width);
  const totale = larghezze.reduce((a, b) => a + b, 0) + spazio * (mostrate.length - 1);
  let x = (larghezza - totale) / 2;
  const y = altezza * 0.82;

  mostrate.forEach((p, i) => {
    const w = larghezze[i];
    if (attiva && p.id === attiva.id) {
      const px = corpo * 0.18;
      const h = corpo * 1.12;
      g.fillStyle = "#8B5CF6";
      const r = corpo * 0.2;
      const rx = x - px;
      const ry = y - h / 2;
      const rw = w + px * 2;
      g.beginPath();
      g.moveTo(rx + r, ry);
      g.arcTo(rx + rw, ry, rx + rw, ry + h, r);
      g.arcTo(rx + rw, ry + h, rx, ry + h, r);
      g.arcTo(rx, ry + h, rx, ry, r);
      g.arcTo(rx, ry, rx + rw, ry, r);
      g.fill();
    }
    g.shadowColor = "rgba(0,0,0,0.63)";
    g.shadowBlur = corpo * 0.16;
    g.shadowOffsetY = corpo * 0.05;
    g.fillStyle = "#FFFFFF";
    g.fillText(p.testo, x, y);
    g.shadowColor = "transparent";
    g.shadowBlur = 0;
    g.shadowOffsetY = 0;
    x += w + spazio;
  });

  return g.getImageData(0, 0, larghezza, altezza);
}
