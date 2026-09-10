/** Sezione 4 — Impostazioni. Una colonna centrata, gruppi separati da
 *  intestazioni testuali.
 *
 *  E' l'unico posto in cui una modifica richiede di rilanciare il modello:
 *  quando ce ne sono di pendenti compare in fondo la barra che lo dice, con il
 *  pulsante che riporta alla sezione 1. */

import type { Impostazioni as Dati, Info, SchedaVista, StatoModelli } from "../tipi";
import { Cursore, Gruppo, Interruttore, Riga, Scelta } from "./controlli";
import * as Icone from "./icone";

interface Props {
  dati: Dati;
  info: Info | null;
  modelli: StatoModelli | null;
  scaricando: boolean;
  frazioneScarico: number;
  pendenti: boolean;
  /** Le GPU fra cui scegliere. Vuoto = nessuna scheda NVIDIA su questa macchina. */
  gpu: SchedaVista[];
  terminiLetti: { quanti: number; primi: string[] } | null;
  onCambia: (d: Dati) => void;
  onTermini: () => void;
  onScegliTermini: () => void;
  onScegliCartellaExport: () => void;
  onScegliCartellaModelli: () => void;
  onScarica: () => void;
  onRitrascrivi: () => void;
}

const LINGUE: { valore: string; etichetta: string }[] = [
  { valore: "it", etichetta: "Italiano" },
  { valore: "en", etichetta: "Inglese" },
  { valore: "fr", etichetta: "Francese" },
  { valore: "de", etichetta: "Tedesco" },
  { valore: "es", etichetta: "Spagnolo" },
  { valore: "auto", etichetta: "Rilevamento automatico" },
];

const COMPROMESSO: Record<Dati["modello"], string> = {
  "large-v3": "2,9 GB su disco, ~4,3 GB di memoria. La qualita' di riferimento; su CPU e' lento.",
  medium: "1,4 GB su disco, ~2,2 GB di memoria. Circa due volte piu' veloce, qualche nome proprio in meno.",
  small: "465 MB su disco, ~1 GB di memoria. Quattro-cinque volte piu' veloce; va bene per una bozza.",
};

export function Impostazioni(p: Props) {
  const cambia = (v: Partial<Dati>) => p.onCambia({ ...p.dati, ...v });

  return (
    <section className="sezione">
      <header className="testata">
        <h1 className="titolo-sezione">Impostazioni</h1>
      </header>

      <div className="corpo">
        <div className="colonna-centrata">
          <Gruppo titolo="Modello">
            <Riga etichetta="Dimensione" spiegazione={COMPROMESSO[p.dati.modello]}>
              <Scelta
                valore={p.dati.modello}
                opzioni={[
                  { valore: "large-v3" as const, etichetta: "large-v3" },
                  { valore: "medium" as const, etichetta: "medium" },
                  { valore: "small" as const, etichetta: "small" },
                ]}
                onChange={(v) => cambia({ modello: v })}
              />
            </Riga>
            <Riga etichetta="Lingua">
              <select value={p.dati.lingua} onChange={(e) => cambia({ lingua: e.target.value })}>
                {LINGUE.map((l) => (
                  <option key={l.valore} value={l.valore}>
                    {l.etichetta}
                  </option>
                ))}
              </select>
            </Riga>
            <Riga
              etichetta="Dispositivo"
              spiegazione="Automatico prova la GPU e, se non si registra, continua su CPU senza fermarsi."
            >
              <Scelta
                valore={p.dati.dispositivo}
                opzioni={[
                  { valore: "automatico" as const, etichetta: "automatico" },
                  { valore: "gpu" as const, etichetta: "GPU" },
                  { valore: "cpu" as const, etichetta: "CPU" },
                ]}
                onChange={(v) => cambia({ dispositivo: v })}
              />
            </Riga>

            {/* Con piu' schede la scelta automatica prende quella con piu'
              * VRAM, che non e' sempre quella che si vuole: la piu' capiente
              * puo' essere anche la piu' vecchia e la piu' lenta. */}
            {p.dati.dispositivo !== "cpu" && (
              <>
                <Riga
                  etichetta="Scheda"
                  spiegazione={
                    p.gpu.length === 0
                      ? "Nessuna GPU NVIDIA visibile su questa macchina: si lavora su CPU."
                      : "Automatica prende quella con piu' memoria, che non e' sempre la piu' veloce."
                  }
                >
                  <select
                    value={p.dati.gpu ?? ""}
                    disabled={p.gpu.length === 0}
                    onChange={(e) =>
                      cambia({ gpu: e.target.value === "" ? null : Number(e.target.value) })
                    }
                  >
                    <option value="">Automatica</option>
                    {p.gpu.map((g) => (
                      <option key={g.indice} value={g.indice}>
                        {g.etichetta}
                      </option>
                    ))}
                  </select>
                </Riga>
                {(() => {
                  // Una riga sola, e sulla scheda che verra' usata davvero:
                  // ripetere l'elenco sotto il menu a tendina non aggiunge
                  // niente a chi lo ha appena letto.
                  const scelta =
                    p.dati.gpu === null
                      ? [...p.gpu].sort((a, b) => b.totale_mib - a.totale_mib)[0]
                      : p.gpu.find((g) => g.indice === p.dati.gpu);
                  if (!scelta) return null;
                  return (
                    <p className="spiegazione">
                      {p.dati.gpu === null ? "Verrebbe usata: " : "In uso: "}
                      {scelta.nome} · {scelta.libera_mib} MiB liberi su {scelta.totale_mib}
                    </p>
                  );
                })()}
              </>
            )}

            <Riga
              etichetta="Termini noti"
              spiegazione="Nomi propri, sigle e parole tecniche suggerite al modello prima della trascrizione. Cambiarli richiede una nuova trascrizione."
            >
              <button className="pulsante" onClick={p.onTermini}>
                <Icone.File />
                {p.terminiLetti && p.terminiLetti.quanti > 0
                  ? `Modifica (${p.terminiLetti.quanti})`
                  : "Aggiungi termini"}
              </button>
            </Riga>
            {p.terminiLetti && p.terminiLetti.quanti > 0 && (
              <p className="spiegazione">
                {p.terminiLetti.primi.join(", ")}
                {p.terminiLetti.quanti > p.terminiLetti.primi.length ? "…" : ""}
              </p>
            )}
            <Riga
              etichetta="Soglia di segnalazione"
              spiegazione="Sotto questa confidenza una parola viene segnalata nella striscia: e' li' che conviene guardare."
            >
              <Cursore
                valore={Math.round(p.dati.soglia * 100)}
                min={0}
                max={100}
                onChange={(v) => cambia({ soglia: v / 100 })}
                formato={(v) => (v / 100).toFixed(2).replace(".", ",")}
              />
            </Riga>
          </Gruppo>

          <Gruppo titolo="Modelli scaricati">
            {p.modelli ? (
              <>
                {p.modelli.modelli.map((m) => (
                  <div key={m.id} className="riga-modello">
                    <span style={{ color: m.presente ? "var(--ok)" : "var(--dim)", width: 14 }}>
                      {m.presente ? "✓" : "·"}
                    </span>
                    <span style={{ color: m.in_uso ? "var(--text)" : "var(--dim)" }}>{m.nome}</span>
                    {m.in_uso && <span style={{ color: "var(--dim)", fontSize: 11 }}>in uso</span>}
                    <span className="peso">
                      {m.ripresa > 0 && !m.presente ? "ripreso a meta'" : m.leggibile}
                    </span>
                  </div>
                ))}
                {p.scaricando ? (
                  <>
                    <div className="barra" style={{ marginTop: 16 }}>
                      <i style={{ width: `${(p.frazioneScarico * 100).toFixed(1)}%` }} />
                    </div>
                    <p className="nota">
                      Uno scaricamento interrotto riprende: non ricomincia da capo.
                    </p>
                  </>
                ) : (
                  !p.modelli.pronto && (
                    <div style={{ marginTop: 16 }}>
                      {p.modelli.da_scaricare > 0 && (
                        <button className="pulsante primario" onClick={p.onScarica}>
                          Scarica quelli che mancano ({p.modelli.da_scaricare_leggibile})
                        </button>
                      )}
                      {p.modelli.a_mano.map((testo, i) => (
                        <p className="nota" key={i} style={{ whiteSpace: "pre-line" }}>
                          {testo}
                        </p>
                      ))}
                    </div>
                  )
                )}
              </>
            ) : (
              <p className="nota">Sto guardando cosa c'e'…</p>
            )}
          </Gruppo>

          <Gruppo titolo="Caratteri">
            <Riga
              etichetta="Font di sistema"
              spiegazione="Oltre ai sei caratteri di serie, cerca anche fra quelli installati sulla macchina."
            >
              <Interruttore
                acceso={p.dati.caratteri_di_sistema}
                onChange={(v) => cambia({ caratteri_di_sistema: v })}
              />
            </Riga>
            {p.dati.caratteri_aggiunti.length > 0 && (
              <p className="spiegazione">
                {p.dati.caratteri_aggiunti.length} aggiunti a mano. Si aggiungono dalla scheda
                Modifica.
              </p>
            )}
          </Gruppo>

          <Gruppo titolo="Cartelle">
            <Riga etichetta="Modelli">
              <button className="pulsante" onClick={p.onScegliCartellaModelli}>
                <Icone.Cartella />
                Cambia
              </button>
            </Riga>
            <p className="spiegazione">{p.modelli?.cartella ?? p.info?.cartella_modelli ?? "—"}</p>
            <Riga etichetta="Export">
              <button className="pulsante" onClick={p.onScegliCartellaExport}>
                <Icone.Cartella />
                Cambia
              </button>
            </Riga>
            <p className="spiegazione">
              {p.dati.cartella_export ?? "accanto al file di partenza"}
            </p>
          </Gruppo>

          <Gruppo titolo="Informazioni">
            <p className="nota">
              Verba {p.info?.versione ?? "—"} · licenza {p.info?.licenza ?? "MIT"}
              <br />
              <a href={p.info?.repository} target="_blank" rel="noreferrer" style={{ color: "var(--dim)" }}>
                {p.info?.repository}
              </a>
              <br />
              Provider di calcolo attivo: {p.info?.provider ?? "non ancora determinato"}
              <br />
              Dispositivo: {p.info?.dispositivo ?? "—"}
              <br />
              Cartella dati: {p.info?.cartella_dati ?? "—"}
            </p>
          </Gruppo>

          {p.pendenti && (
            <div className="avviso-in-fondo">
              <span className="testo">Le modifiche richiedono una nuova trascrizione</span>
              <button className="pulsante primario a-destra" onClick={p.onRitrascrivi}>
                Ritrascrivi
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
