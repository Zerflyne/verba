/** Il ponte verso il motore.
 *
 *  Dentro l'applicazione ogni funzione qui e' una `invoke` verso un comando di
 *  `verba-app`. Aperta in un browser — `npm run dev` senza Tauri — le stesse
 *  funzioni rispondono con dati finti: serve a lavorare sull'aspetto senza
 *  ricompilare il motore a ogni modifica del CSS, e a guardare la finestra
 *  anche su una macchina che non ha GTK e WebKit.
 *
 *  Il banco di prova non deve mai passare per vero: `finto` e' esportato, la
 *  barra di stato lo dice, e nessun comando finge di aver scritto un file. */

import type {
  Descrizione,
  EsitoExport,
  Evento,
  Famiglia,
  FormatiDisponibili,
  Impostazioni,
  Info,
  ParolaVista,
  Preset,
  Riepilogo,
  StatoModelli,
} from "./tipi";
import { bancoDiProva, fotogrammaFinto, presetFinti, scenaIniziale } from "./banco";

/** Vero quando la finestra non e' quella di Tauri. */
export const finto = !("__TAURI_INTERNALS__" in window);

async function chiama<T>(comando: string, argomenti?: Record<string, unknown>): Promise<T> {
  if (finto) return bancoDiProva<T>(comando, argomenti);
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(comando, argomenti);
}

/** Si mette in ascolto dell'avanzamento delle fasi. */
export async function ascolta(f: (e: Evento) => void): Promise<() => void> {
  if (finto) {
    (window as unknown as { __verbaEventi?: (e: Evento) => void }).__verbaEventi = f;
    return () => {};
  }
  const { listen } = await import("@tauri-apps/api/event");
  const stop = await listen<Evento>("verba://avanzamento", (m) => f(m.payload));
  return stop;
}

/** Il file trascinato sulla finestra. */
export async function ascoltaTrascinamento(
  sopra: (dentro: boolean) => void,
  lasciato: (percorsi: string[]) => void,
): Promise<() => void> {
  if (finto) return () => {};
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  const stop = await getCurrentWebview().onDragDropEvent((e) => {
    if (e.payload.type === "over") sopra(true);
    else if (e.payload.type === "leave") sopra(false);
    else if (e.payload.type === "drop") {
      sopra(false);
      lasciato(e.payload.paths);
    }
  });
  return stop;
}

/** Il selettore di file del sistema. */
export async function scegliFile(
  filtri: { name: string; extensions: string[] }[],
): Promise<string | null> {
  if (finto) return "/esempio/video_tutorial.mp4";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const scelto = await open({ multiple: false, filters: filtri });
  return typeof scelto === "string" ? scelto : null;
}

export async function scegliDoveSalvare(
  predefinito: string,
  filtri: { name: string; extensions: string[] }[],
): Promise<string | null> {
  if (finto) return predefinito;
  const { save } = await import("@tauri-apps/plugin-dialog");
  return await save({ defaultPath: predefinito, filters: filtri });
}

export async function scegliCartella(): Promise<string | null> {
  if (finto) return "/esempio/uscite";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const scelta = await open({ directory: true });
  return typeof scelta === "string" ? scelta : null;
}

// ---------------------------------------------------------------- comandi

export const informazioni = () => chiama<Info>("informazioni");
export const impostazioni = () => chiama<Impostazioni>("impostazioni");
export const salvaImpostazioni = (nuove: Impostazioni) =>
  chiama<void>("salva_impostazioni", { nuove });

export const statoModelli = () => chiama<StatoModelli>("stato_modelli");
export const scaricaModelli = () => chiama<StatoModelli>("scarica_modelli");

export const apri = (percorso: string) => chiama<Descrizione>("apri", { percorso });
export const chiudi = () => chiama<void>("chiudi");
export const trascrivi = () => chiama<Riepilogo>("trascrivi");
export const annulla = () => chiama<void>("annulla");

export const presetCorrente = () => chiama<Preset | null>("preset_corrente");
export const presetDiSerie = () => chiama<Preset[]>("preset_di_serie");
export const presetCarica = (percorso: string) => chiama<Preset>("preset_carica", { percorso });
export const presetSalva = (percorso: string, preset: Preset) =>
  chiama<void>("preset_salva", { percorso, preset });
export const applicaAspetto = (preset: Preset) =>
  chiama<string | null>("applica_aspetto", { preset });

export const dimensioni = () => chiama<[number, number] | null>("dimensioni");
export const onda = () => chiama<number[]>("onda");
export const parole = () => chiama<ParolaVista[]>("parole");
export const finestra = (t: number, quante: number) =>
  chiama<ParolaVista[]>("finestra", { t, quante });

export const caratteri = () => chiama<Famiglia[]>("caratteri");
export const aggiungiCarattere = (percorso: string) =>
  chiama<Famiglia[]>("aggiungi_carattere", { percorso });

export const formati = () => chiama<FormatiDisponibili>("formati");
export const nomeProposto = (formato: string) => chiama<string>("nome_proposto", { formato });
export const esporta = (formato: string, percorso: string, qualita: number) =>
  chiama<EsitoExport>("esporta", { formato, percorso, qualita });
export const esportaTesto = (percorso: string) => chiama<string>("esporta_testo", { percorso });
export const mostraNellaCartella = (percorso: string) =>
  chiama<void>("mostra_nella_cartella", { percorso });

/** Il fotogramma al tempo `t`, come pixel RGBA.
 *
 *  Arriva gia' disegnato dal motore — **lo stesso codice che produce
 *  l'export** — e qui viene solo messo su un canvas. Nessuna impaginazione
 *  avviene in JavaScript: se avvenisse, anteprima ed export finirebbero per
 *  non coincidere. */
export async function fotogramma(t: number, larghezza: number, altezza: number): Promise<ImageData> {
  if (finto) return fotogrammaFinto(t, larghezza, altezza);
  const { invoke } = await import("@tauri-apps/api/core");
  const byte = await invoke<ArrayBuffer>("fotogramma", { t });
  return new ImageData(new Uint8ClampedArray(byte), larghezza, altezza);
}

export { presetFinti };

/** Solo senza Tauri: la scena da aprire, se l'indirizzo ne chiede una. */
export function scenaDaMostrare() {
  return finto ? scenaIniziale() : null;
}
