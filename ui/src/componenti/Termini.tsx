/** L'editor dei termini noti.
 *
 *  I termini sono l'unica leva che fa scrivere «Zerflyne» invece di «zer
 *  fline»: finiscono nell'*initial prompt* di Whisper, che e' la via ufficiale
 *  per orientare il modello su nomi propri e sigle. Prima si potevano solo
 *  scegliere da un CSV gia' pronto, il che vuol dire aprire un altro
 *  programma, e chi non ne aveva uno semplicemente non li usava.
 *
 *  Qui si scrivono e basta. Il file CSV resta il formato in cui vengono
 *  salvati — cosi' lo si puo' ancora aprire con un foglio di calcolo, e chi ne
 *  aveva gia' uno se lo ritrova — ma non serve piu' averlo per cominciare. */

import { useEffect, useRef, useState } from "react";
import * as Icone from "./icone";

interface Props {
  /** I termini da cui partire. */
  iniziali: string[];
  /** Il file in cui finiranno. */
  percorso: string;
  onSalva: (elenco: string[]) => void;
  onChiudi: () => void;
  onImporta: () => void;
}

export function Termini({ iniziali, percorso, onSalva, onChiudi, onImporta }: Props) {
  const [righe, setRighe] = useState<string[]>(iniziali.length ? iniziali : [""]);
  const ultimo = useRef<HTMLInputElement>(null);
  const daMettereAFuoco = useRef(false);

  // Un termine appena aggiunto prende il fuoco: aggiungerne dieci di fila deve
  // costare dieci volte «scrivi, Invio», non dieci volte «clicca, scrivi».
  useEffect(() => {
    if (daMettereAFuoco.current) {
      ultimo.current?.focus();
      daMettereAFuoco.current = false;
    }
  }, [righe.length]);

  const aggiungi = () => {
    daMettereAFuoco.current = true;
    setRighe((r) => [...r, ""]);
  };

  const cambia = (i: number, v: string) =>
    setRighe((r) => r.map((x, k) => (k === i ? v : x)));

  const togli = (i: number) =>
    setRighe((r) => {
      const dopo = r.filter((_, k) => k !== i);
      return dopo.length ? dopo : [""];
    });

  const puliti = righe.map((r) => r.trim()).filter((r) => r.length > 0);

  // I duplicati non sono un errore da bloccare — il motore li scarta da solo —
  // ma dirlo evita che qualcuno li conti nel totale.
  const doppi = puliti.length - new Set(puliti.map((p) => p.toLowerCase())).size;

  return (
    <div className="velo" onMouseDown={onChiudi}>
      <div className="pannello" onMouseDown={(e) => e.stopPropagation()}>
        <header className="pannello-testata">
          <h2>Termini noti</h2>
          <button className="pulsante discreto" onClick={onChiudi} title="Chiudi">
            <Icone.Chiudi />
          </button>
        </header>

        <p className="spiegazione">
          Nomi propri, sigle, parole tecniche: quelle che un modello generico sbaglia. Vengono
          suggerite a Whisper prima della trascrizione, e sono la differenza fra «Zerflyne» e «zer
          fline». Bastano quelle che ricorrono davvero — un elenco lungo non aiuta di piu'.
        </p>

        <div className="elenco-termini">
          {righe.map((r, i) => (
            <div className="riga-termine" key={i}>
              <input
                ref={i === righe.length - 1 ? ultimo : undefined}
                value={r}
                placeholder="un nome, una sigla, un termine…"
                onChange={(e) => cambia(i, e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    aggiungi();
                  }
                }}
              />
              <button
                className="pulsante discreto"
                onClick={() => togli(i)}
                title="Togli questo termine"
              >
                <Icone.Meno />
              </button>
            </div>
          ))}
        </div>

        <div className="bottoni-in-fila">
          <button className="pulsante" onClick={aggiungi}>
            <Icone.Piu />
            Aggiungi termine
          </button>
          <button className="pulsante discreto" onClick={onImporta}>
            <Icone.File />
            Importa da un CSV…
          </button>
        </div>

        <footer className="pannello-piede">
          <span className="conteggio">
            {puliti.length === 0
              ? "nessun termine"
              : `${puliti.length} termini${doppi > 0 ? ` · ${doppi} ripetuti, verranno scartati` : ""}`}
          </span>
          {/* Come in Esporta: la colonna e' destra-sinistra per troncare
              all'inizio e lasciare visibile il nome del file, ma il `bdi`
              tiene il testo sinistra-destra — senza, la barra iniziale
              finirebbe in coda. */}
          <span className="percorso" title={percorso}>
            <bdi>{percorso}</bdi>
          </span>
          <button className="pulsante primario a-destra" onClick={() => onSalva(puliti)}>
            Salva
          </button>
        </footer>
      </div>
    </div>
  );
}
