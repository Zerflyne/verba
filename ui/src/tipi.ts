/** I tipi che attraversano il ponte fra la finestra e il motore.
 *
 *  Sono gli stessi campi che `verba-core` serializza: quando cambia una
 *  struttura la' va cambiata anche qui, e il compilatore di TypeScript non
 *  puo' accorgersene da solo. */

export type Sezione = "carica" | "modifica" | "esporta" | "impostazioni";

export interface Descrizione {
  percorso: string;
  nome: string;
  modalita: "audio" | "video";
  larghezza: number;
  altezza: number;
  fps: number;
  durata: number;
  codec_video: string | null;
  codec_audio: string | null;
  riassunto: string;
}

export interface ParolaVista {
  id: number;
  testo: string;
  inizio: number;
  fine: number;
  confidenza: number;
  incerta: boolean;
}

export interface Riepilogo {
  parole: number;
  incerte: number;
  blocchi: number;
  secondi: number;
  dispositivo: string;
  modello: string;
}

export type NomeFase =
  | "scaricamento"
  | "preparazione"
  | "segmentazione"
  | "trascrizione"
  | "allineamento"
  | "pulizia"
  | "impaginazione"
  | "codifica";

export type Evento =
  | { evento: "iniziata"; fase: NomeFase }
  | { evento: "avanzamento"; fase: NomeFase; frazione: number }
  | { evento: "conclusa"; fase: NomeFase; secondi: number }
  | { evento: "avviso"; messaggio: string }
  | { evento: "annullata" };

export type FormatoPreset = "verticale" | "orizzontale" | "dal_sorgente";
export type Allineamento = "sinistra" | "centro" | "destra";
export type Forma = "rettangolo" | "sottolineatura" | "solo_colore" | "nessuna";

export interface Preset {
  versione: number;
  nome: string;
  testo: {
    carattere: string;
    peso: number;
    corpo: number | null;
    maiuscole: boolean;
    interlinea: number;
  };
  colori: {
    testo: string;
    testo_attivo: string;
    evidenziazione: string;
    bordo: string;
    bordo_px: number;
    ombra: boolean;
    colore_ombra: string;
    ombra_spostamento: number;
    ombra_sfocatura: number;
  };
  evidenziazione: {
    forma: Forma;
    padding: number;
    altezza: number;
    raggio: number;
    spessore_sottolineatura: number;
  };
  posizione: {
    formato: FormatoPreset;
    verticale: number;
    orizzontale: number;
    larghezza_max: number;
    margine: number;
    righe_max: number;
    allineamento: Allineamento;
  };
  tempi: {
    anticipo_ms: number;
    pausa_massima_ms: number;
    coda_ms: number;
    durata_minima_parola_ms: number;
    durata_blocco: number;
    pausa_blocco: number;
    tenuta: number;
  };
}

export interface Famiglia {
  nome: string;
  pesi: number[];
  di_serie: boolean;
}

export type Dimensione = "small" | "medium" | "large-v3";
export type Dispositivo = "automatico" | "gpu" | "cpu";

export interface Impostazioni {
  versione: number;
  modello: Dimensione;
  lingua: string;
  dispositivo: Dispositivo;
  termini: string | null;
  soglia: number;
  cartella_export: string | null;
  cartella_modelli: string | null;
  caratteri_aggiunti: string[];
  caratteri_di_sistema: boolean;
  ultimo_formato: string | null;
}

export interface ModelloVisto {
  id: string;
  nome: string;
  file: string;
  spiegazione: string;
  byte: number;
  leggibile: string;
  presente: boolean;
  ripresa: number;
  si_scarica: boolean;
  comando: string | null;
  in_uso: boolean;
}

export interface StatoModelli {
  cartella: string;
  modelli: ModelloVisto[];
  pronto: boolean;
  da_scaricare: number;
  da_scaricare_leggibile: string;
  a_mano: string[];
}

export interface FormatoVisto {
  id: string;
  etichetta: string;
  descrizione: string;
  estensione: string;
  alfa: boolean;
}

export interface FormatoTesto {
  id: string;
  etichetta: string;
  descrizione: string;
  estensione: string;
}

export interface FormatiDisponibili {
  video: FormatoVisto[];
  testo: FormatoTesto[];
}

export interface Info {
  versione: string;
  repository: string;
  licenza: string;
  provider: string;
  dispositivo: string;
  cartella_modelli: string;
  cartella_dati: string;
}

export interface EsitoExport {
  percorso: string;
  fotogrammi: number;
  secondi: number;
}
