/** Sezione 3 — Esporta. Una colonna sola, centrata, molto arieggiata.
 *
 *  Niente finestre modali: l'export e' una sezione, non una finestra che si
 *  apre. Durante la codifica la schermata si sostituisce con l'avanzamento; al
 *  termine, il segno di spunta e i due pulsanti. */

import { useEffect, useState } from "react";
import type { FormatiDisponibili } from "../tipi";
import { durata as leggiDurata } from "./controlli";
import { Cursore, Gruppo } from "./controlli";
import * as Icone from "./icone";

export type StatoExport =
  | { fase: "scelta" }
  | { fase: "in-corso"; frazione: number; avvio: number }
  | { fase: "fatto"; percorso: string; fotogrammi: number; secondi: number };

interface Props {
  formati: FormatiDisponibili;
  scelto: string;
  destinazione: string;
  qualita: number;
  stato: StatoExport;
  soloAudio: boolean;
  onScegliFormato: (id: string) => void;
  onCambiaDestinazione: () => void;
  onQualita: (v: number) => void;
  onEsporta: () => void;
  onAnnulla: () => void;
  onApriCartella: () => void;
  onDiNuovo: () => void;
}

export function Esporta(p: Props) {
  const video = p.formati.video.find((f) => f.id === p.scelto);
  const q = p.stato.fase === "in-corso" ? p.stato.frazione : 0;

  return (
    <section className="sezione">
      <header className="testata">
        <h1 className="titolo-sezione">Esporta</h1>
      </header>

      <div className="corpo">
        <div className="colonna-centrata">
          {p.stato.fase === "scelta" && (
            <>
              <Gruppo titolo="Formato">
                <div className="elenco-formati">
                  {/* In modalita' audio i formati video non ci sono proprio:
                      nasconderli e' piu' onesto che mostrarli spenti. */}
                  {!p.soloAudio &&
                    p.formati.video.map((f) => (
                      <Formato
                        key={f.id}
                        id={f.id}
                        nome={f.etichetta}
                        estensione={f.estensione}
                        descrizione={f.descrizione}
                        scelto={p.scelto === f.id}
                        onScegli={p.onScegliFormato}
                      />
                    ))}
                  {p.formati.testo.map((f) => (
                    <Formato
                      key={f.id}
                      id={f.id}
                      nome={f.etichetta}
                      estensione={f.estensione}
                      descrizione={f.descrizione}
                      scelto={p.scelto === f.id}
                      onScegli={p.onScegliFormato}
                    />
                  ))}
                </div>
              </Gruppo>

              <Gruppo titolo="Destinazione">
                <div className="destinazione">
                  {/* `bdi` isola il percorso: la colonna e' in direzione
                      destra-sinistra per troncare all'inizio e lasciare
                      visibile il nome del file, ma il testo dentro resta
                      sinistra-destra, altrimenti la barra iniziale finirebbe
                      in coda. */}
                  <span className="percorso" title={p.destinazione}>
                    <bdi>{p.destinazione}</bdi>
                  </span>
                  <button className="pulsante" onClick={p.onCambiaDestinazione}>
                    <Icone.Cartella />
                    Cambia
                  </button>
                </div>
              </Gruppo>

              {video && (
                <Gruppo titolo="Opzioni">
                  <div className="riga-controllo">
                    <span className="etichetta">
                      {video.id === "h264" || video.id === "vp9" ? "Qualita' (CRF)" : "Quantizzatore"}
                    </span>
                    <Cursore
                      valore={p.qualita}
                      min={0}
                      max={40}
                      onChange={p.onQualita}
                      formato={(v) => (v === 0 ? "consigliata" : String(v))}
                    />
                  </div>
                  <p className="spiegazione">
                    Piu' basso, piu' bit e piu' nitidezza sui bordi del testo. Zero lascia decidere
                    al formato.
                  </p>
                </Gruppo>
              )}

              <div className="bottoni-in-fila" style={{ justifyContent: "flex-start" }}>
                <button className="pulsante primario" onClick={p.onEsporta}>
                  Esporta
                </button>
              </div>
            </>
          )}

          {p.stato.fase === "in-corso" && (
            <InCorso frazione={q} avvio={p.stato.avvio} onAnnulla={p.onAnnulla} />
          )}

          {p.stato.fase === "fatto" && (
            <div className="esito">
              <div className="segno-grande">✓</div>
              <p className="file-fatto">{p.stato.percorso}</p>
              <div className="bottoni-in-fila">
                <button className="pulsante" onClick={p.onApriCartella}>
                  <Icone.Cartella />
                  Apri cartella
                </button>
                <button className="pulsante" onClick={p.onDiNuovo}>
                  Esporta di nuovo
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function Formato({
  id,
  nome,
  estensione,
  descrizione,
  scelto,
  onScegli,
}: {
  id: string;
  nome: string;
  estensione: string;
  descrizione: string;
  scelto: boolean;
  onScegli: (id: string) => void;
}) {
  return (
    <button className={`formato${scelto ? " scelto" : ""}`} onClick={() => onScegli(id)}>
      <span className="nome">
        {nome}
        <span className="estensione">.{estensione}</span>
      </span>
      <span className="descrizione">{descrizione}</span>
    </button>
  );
}

function InCorso({
  frazione,
  avvio,
  onAnnulla,
}: {
  frazione: number;
  avvio: number;
  onAnnulla: () => void;
}) {
  const [ora, setOra] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setOra(Date.now()), 500);
    return () => clearInterval(id);
  }, []);

  const trascorsi = (ora - avvio) / 1000;
  // La stima e' lineare: nella codifica lo e' davvero, perche' i fotogrammi
  // costano tutti uguale.
  const residuo = frazione > 0.02 ? trascorsi / frazione - trascorsi : null;

  return (
    <div style={{ paddingTop: 80 }}>
      <p style={{ marginBottom: 14, fontSize: 15 }}>Codifica in corso</p>
      <div className="barra">
        <i style={{ width: `${(frazione * 100).toFixed(1)}%` }} />
      </div>
      <p className="nota" style={{ marginTop: 12 }}>
        {(frazione * 100).toFixed(0)} % · {leggiDurata(trascorsi)} trascorsi
        {residuo !== null ? ` · ${leggiDurata(residuo)} rimanenti` : ""}
      </p>
      <div className="bottoni-in-fila" style={{ justifyContent: "flex-start", marginTop: 24 }}>
        <button className="pulsante" onClick={onAnnulla}>
          Annulla
        </button>
      </div>
      <p className="nota">Annullando, il file parziale viene cancellato.</p>
    </div>
  );
}
