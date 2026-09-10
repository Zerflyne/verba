/** Sezione 2 — Modifica. A sinistra l'anteprima, a destra i controlli.
 *
 *  Nessuna modifica in questa sezione rilancia il modello: tutto si applica
 *  sull'anteprima entro un fotogramma. Il corpo del carattere e' in pixel
 *  riferiti all'altezza del **video sorgente**, non dell'anteprima, e accanto
 *  al cursore compare la percentuale sul fotogramma — e' il numero che conta
 *  davvero, perche' 64 px su un 4K e 64 px su un 1080p danno risultati molto
 *  diversi. */

import type { Famiglia, Preset } from "../tipi";
import { Anteprima } from "./Anteprima";
import { Trasporto } from "./Trasporto";
import { Colore, Cursore, Gruppo, Interruttore, Riga, Scelta } from "./controlli";

interface Props {
  preset: Preset;
  caratteri: Famiglia[];
  larghezza: number;
  altezza: number;
  tempo: number;
  durata: number;
  /** Il file di partenza e' solo audio. */
  soloAudio: boolean;
  /** L'indirizzo della traccia da suonare, se c'e'. */
  audio: string | null;
  inRiproduzione: boolean;
  avvisoCarattere: string | null;
  onPreset: (p: Preset) => void;
  onTempo: (t: number) => void;
  onRiproduzione: (v: boolean) => void;
  onSalvaPreset: () => void;
  onCaricaPreset: () => void;
  onPresetDiSerie: (nome: string) => void;
  onAggiungiCarattere: () => void;
}

/** Il corpo predefinito quando il preset non ne dichiara uno. */
function corpoEffettivo(preset: Preset, altezza: number, larghezza: number): number {
  if (preset.testo.corpo != null) return preset.testo.corpo;
  return Math.round(Math.min(larghezza, altezza) * 0.065);
}

export function Modifica(p: Props) {
  const { preset } = p;
  // Una modifica non muta il preset: ne costruisce uno nuovo. Cosi' React sa
  // sempre che qualcosa e' cambiato, e il confronto con il salvato e' banale.
  const cambia = (parte: Partial<Preset>) => p.onPreset({ ...preset, ...parte });
  const testo = (v: Partial<Preset["testo"]>) => cambia({ testo: { ...preset.testo, ...v } });
  const colori = (v: Partial<Preset["colori"]>) => cambia({ colori: { ...preset.colori, ...v } });
  const evid = (v: Partial<Preset["evidenziazione"]>) =>
    cambia({ evidenziazione: { ...preset.evidenziazione, ...v } });
  const pos = (v: Partial<Preset["posizione"]>) =>
    cambia({ posizione: { ...preset.posizione, ...v } });
  const tempi = (v: Partial<Preset["tempi"]>) => cambia({ tempi: { ...preset.tempi, ...v } });

  const corpo = corpoEffettivo(preset, p.altezza, p.larghezza);
  const percentuale = ((corpo / p.altezza) * 100).toFixed(1);
  const famiglia = p.caratteri.find((c) => c.nome === preset.testo.carattere);
  const pesi = famiglia?.pesi ?? [400, 700];

  return (
    <section className="sezione">
      <header className="testata">
        <h1 className="titolo-sezione">Modifica</h1>
        {p.avvisoCarattere && (
          <span className="targhetta" style={{ color: "var(--warn)" }}>
            {p.avvisoCarattere}
          </span>
        )}
      </header>

      <div className="corpo">
        <div className="due-colonne">
          <div style={{ display: "flex", flexDirection: "column", minWidth: 0 }}>
            <Anteprima
              tempo={p.tempo}
              larghezza={p.larghezza}
              altezza={p.altezza}
              baseline={preset.posizione.verticale}
              margine={preset.posizione.margine}
              soloAudio={p.soloAudio}
            />
            <Trasporto
              tempo={p.tempo}
              durata={p.durata}
              inRiproduzione={p.inRiproduzione}
              onTempo={p.onTempo}
              onRiproduzione={p.onRiproduzione}
              audio={p.audio}
            />
          </div>

          <div className="colonna-controlli">
            <Gruppo titolo="Testo">
              <Riga etichetta="Carattere">
                <select
                  value={preset.testo.carattere}
                  onChange={(e) => testo({ carattere: e.target.value })}
                >
                  {p.caratteri.map((c) => (
                    <option key={c.nome} value={c.nome}>
                      {c.nome}
                      {c.di_serie ? "" : " (sistema)"}
                    </option>
                  ))}
                </select>
              </Riga>
              <Riga etichetta="Peso">
                <Scelta
                  valore={preset.testo.peso}
                  opzioni={pesi.map((w) => ({ valore: w, etichetta: String(w) }))}
                  onChange={(w) => testo({ peso: w })}
                />
              </Riga>
              <Riga etichetta="Corpo">
                <Cursore
                  valore={corpo}
                  min={24}
                  max={160}
                  onChange={(v) => testo({ corpo: v })}
                  formato={(v) => `${v} px · ${percentuale} %`}
                />
              </Riga>
              <Riga etichetta="Maiuscole">
                <Interruttore
                  acceso={preset.testo.maiuscole}
                  onChange={(v) => testo({ maiuscole: v })}
                />
              </Riga>
              <Riga etichetta="Interlinea">
                <Cursore
                  valore={preset.testo.interlinea}
                  min={0.9}
                  max={2}
                  passo={0.01}
                  onChange={(v) => testo({ interlinea: v })}
                  formato={(v) => v.toFixed(2)}
                />
              </Riga>
              <div className="riga-controllo">
                <span className="etichetta" />
                <button className="pulsante" onClick={p.onAggiungiCarattere}>
                  Aggiungi un carattere…
                </button>
              </div>
              <p className="spiegazione">
                Un `.ttf` o `.otf` scaricato da qualsiasi parte. Il file resta dov'e': Verba se ne
                ricorda il percorso e lo ricarica a ogni avvio.
              </p>
            </Gruppo>

            <Gruppo titolo="Colori">
              <Riga etichetta="Testo">
                <Colore valore={preset.colori.testo} onChange={(v) => colori({ testo: v })} />
              </Riga>
              <Riga etichetta="Parola attiva">
                <Colore
                  valore={preset.colori.evidenziazione}
                  onChange={(v) => colori({ evidenziazione: v })}
                />
              </Riga>
              <Riga etichetta="Testo attivo">
                <Colore
                  valore={preset.colori.testo_attivo}
                  onChange={(v) => colori({ testo_attivo: v })}
                />
              </Riga>
              <Riga etichetta="Contorno">
                <Cursore
                  valore={preset.colori.bordo_px}
                  min={0}
                  max={16}
                  onChange={(v) => colori({ bordo_px: v })}
                  formato={(v) => `${v} px`}
                />
              </Riga>
              <Riga etichetta="Colore contorno">
                <Colore valore={preset.colori.bordo} onChange={(v) => colori({ bordo: v })} />
              </Riga>
              <Riga etichetta="Ombra">
                <Interruttore acceso={preset.colori.ombra} onChange={(v) => colori({ ombra: v })} />
              </Riga>
            </Gruppo>

            <Gruppo titolo="Evidenziazione">
              <Riga etichetta="Forma">
                <Scelta
                  valore={preset.evidenziazione.forma}
                  opzioni={[
                    { valore: "rettangolo" as const, etichetta: "rettangolo" },
                    { valore: "sottolineatura" as const, etichetta: "sottolineatura" },
                    { valore: "solo_colore" as const, etichetta: "solo colore" },
                    { valore: "nessuna" as const, etichetta: "nessuna" },
                  ]}
                  onChange={(v) => evid({ forma: v })}
                />
              </Riga>
              <Riga etichetta="Raggio angoli">
                <Cursore
                  valore={Math.round(preset.evidenziazione.raggio * corpo)}
                  min={0}
                  max={40}
                  onChange={(v) => evid({ raggio: v / corpo })}
                  formato={(v) => `${v} px`}
                />
              </Riga>
            </Gruppo>

            <Gruppo titolo="Posizione">
              <Riga etichetta="Formato">
                <Scelta
                  valore={preset.posizione.formato}
                  opzioni={[
                    { valore: "orizzontale" as const, etichetta: "16:9" },
                    { valore: "verticale" as const, etichetta: "9:16" },
                    { valore: "dal_sorgente" as const, etichetta: "dal sorgente" },
                  ]}
                  onChange={(v) => pos({ formato: v })}
                />
              </Riga>
              <Riga etichetta="Verticale">
                <Cursore
                  valore={Math.round(preset.posizione.verticale * 100)}
                  min={0}
                  max={100}
                  onChange={(v) => pos({ verticale: v / 100 })}
                  formato={(v) => `${v} %`}
                />
              </Riga>
              <Riga etichetta="Orizzontale">
                <Cursore
                  valore={Math.round(preset.posizione.orizzontale * 100)}
                  min={0}
                  max={100}
                  onChange={(v) => pos({ orizzontale: v / 100 })}
                  formato={(v) => `${v} %`}
                />
              </Riga>
              <Riga etichetta="Larghezza massima">
                <Cursore
                  valore={Math.round(preset.posizione.larghezza_max * 100)}
                  min={40}
                  max={100}
                  onChange={(v) => pos({ larghezza_max: v / 100 })}
                  formato={(v) => `${v} %`}
                />
              </Riga>
              <Riga etichetta="Righe massime">
                <Scelta
                  valore={preset.posizione.righe_max}
                  opzioni={[
                    { valore: 1, etichetta: "1" },
                    { valore: 2, etichetta: "2" },
                    { valore: 3, etichetta: "3" },
                  ]}
                  onChange={(v) => pos({ righe_max: v })}
                />
              </Riga>
              <Riga etichetta="Allineamento">
                <Scelta
                  valore={preset.posizione.allineamento}
                  opzioni={[
                    { valore: "sinistra" as const, etichetta: "sinistra" },
                    { valore: "centro" as const, etichetta: "centro" },
                    { valore: "destra" as const, etichetta: "destra" },
                  ]}
                  onChange={(v) => pos({ allineamento: v })}
                />
              </Riga>
              <Riga etichetta="Margine dai bordi">
                <Cursore
                  valore={Math.round(preset.posizione.margine * 100)}
                  min={0}
                  max={20}
                  onChange={(v) => pos({ margine: v / 100 })}
                  formato={(v) => `${v} %`}
                />
              </Riga>
            </Gruppo>

            <Gruppo titolo="Tempi">
              <Riga
                etichetta="Anticipo"
                spiegazione="Quanto l'evidenziazione arriva prima dell'inizio della parola. Un filo di anticipo la fa sembrare sincronizzata."
              >
                <Cursore
                  valore={preset.tempi.anticipo_ms}
                  min={0}
                  max={200}
                  passo={5}
                  onChange={(v) => tempi({ anticipo_ms: v })}
                  formato={(v) => `${v} ms`}
                />
              </Riga>
              <Riga
                etichetta="Tetto alla pausa"
                spiegazione="Impedisce all'evidenziazione di restare accesa per tutta la durata di un silenzio."
              >
                <Cursore
                  valore={preset.tempi.pausa_massima_ms}
                  min={0}
                  max={2000}
                  passo={10}
                  onChange={(v) => tempi({ pausa_massima_ms: v })}
                  formato={(v) => `${v} ms`}
                />
              </Riga>
              <Riga
                etichetta="Coda"
                spiegazione="Quanto l'evidenziazione resta accesa dopo l'ultima parola della riga."
              >
                <Cursore
                  valore={preset.tempi.coda_ms}
                  min={0}
                  max={2000}
                  passo={10}
                  onChange={(v) => tempi({ coda_ms: v })}
                  formato={(v) => `${v} ms`}
                />
              </Riga>
              <Riga
                etichetta="Durata minima parola"
                spiegazione="Sotto questa soglia l'evidenziazione lampeggerebbe invece di leggersi."
              >
                <Cursore
                  valore={preset.tempi.durata_minima_parola_ms}
                  min={20}
                  max={300}
                  passo={5}
                  onChange={(v) => tempi({ durata_minima_parola_ms: v })}
                  formato={(v) => `${v} ms`}
                />
              </Riga>
            </Gruppo>

            <Gruppo titolo="Preset">
              <div className="scelta" style={{ marginBottom: 12 }}>
                {["Verticale", "Orizzontale", "Sobrio"].map((n) => (
                  <button
                    key={n}
                    className={preset.nome === n ? "attiva" : ""}
                    onClick={() => p.onPresetDiSerie(n)}
                  >
                    {n}
                  </button>
                ))}
              </div>
              <div className="bottoni-in-fila" style={{ justifyContent: "flex-start" }}>
                <button className="pulsante" onClick={p.onCaricaPreset}>
                  Carica…
                </button>
                <button className="pulsante" onClick={p.onSalvaPreset}>
                  Salva…
                </button>
              </div>
              <p className="nota">
                Un preset contiene testo, colori, evidenziazione, posizione e tempi. Non contiene le
                impostazioni del modello ne' riferimenti a file: si puo' mandare a chiunque.
              </p>
            </Gruppo>
          </div>
        </div>
      </div>
    </section>
  );
}
