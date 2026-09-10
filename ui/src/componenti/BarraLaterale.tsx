/** La barra laterale: identita', quattro voci, e in fondo versione e
 *  repository.
 *
 *  Modifica ed Esporta restano spente finche' non c'e' una trascrizione. E'
 *  il modo piu' semplice per far capire l'ordine delle cose senza spiegarlo;
 *  una volta accese, la navigazione e' libera in avanti e indietro. */

import type { Sezione } from "../tipi";
import * as Icone from "./icone";

interface Props {
  attiva: Sezione;
  sbloccate: boolean;
  versione: string;
  repository: string;
  onVai: (s: Sezione) => void;
}

const VOCI: { id: Sezione; etichetta: string; icona: typeof Icone.Carica; serve: boolean }[] = [
  { id: "carica", etichetta: "Carica", icona: Icone.Carica, serve: false },
  { id: "modifica", etichetta: "Modifica", icona: Icone.Modifica, serve: true },
  { id: "esporta", etichetta: "Esporta", icona: Icone.Esporta, serve: true },
  { id: "impostazioni", etichetta: "Impostazioni", icona: Icone.Impostazioni, serve: false },
];

export function BarraLaterale({ attiva, sbloccate, versione, repository, onVai }: Props) {
  return (
    <nav className="laterale">
      <div className="identita">
        <span className="marchio">
          <Icone.Marchio size={16} />
        </span>
        <span className="nome-app">Verba</span>
      </div>

      <div className="voci">
        {VOCI.map((v) => {
          const spenta = v.serve && !sbloccate;
          const Icona = v.icona;
          return (
            <button
              key={v.id}
              className={`voce${attiva === v.id ? " attiva" : ""}`}
              disabled={spenta}
              onClick={() => onVai(v.id)}
              title={spenta ? "Prima serve una trascrizione" : undefined}
            >
              <Icona />
              <span>{v.etichetta}</span>
            </button>
          );
        })}
      </div>

      <div className="piede-laterale">
        <span>v{versione}</span>
        <a href={repository} target="_blank" rel="noreferrer">
          Repository
        </a>
      </div>
    </nav>
  );
}
