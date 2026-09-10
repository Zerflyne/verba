/** La finestra di parole intorno alla posizione corrente.
 *
 *  Nella 0.1 e' in sola lettura: mostra dove si e' e segnala le parole a bassa
 *  confidenza, che sono il posto dove conviene guardare. Nella 0.2 diventa
 *  modificabile, ed e' per questo che ogni parola porta con se' il proprio
 *  identificativo invece della sola posizione nell'elenco. */

import type { ParolaVista } from "../tipi";

interface Props {
  parole: ParolaVista[];
  attiva: number | null;
  onVai: (t: number) => void;
}

export function StrisciaParole({ parole, attiva, onVai }: Props) {
  return (
    <div className="striscia">
      {parole.map((p) => (
        <button
          key={p.id}
          className={`parola${p.id === attiva ? " attiva" : ""}${p.incerta ? " incerta" : ""}`}
          onClick={() => onVai(p.inizio)}
          title={p.incerta ? `confidenza ${(p.confidenza * 100).toFixed(0)} %` : undefined}
        >
          {p.testo}
        </button>
      ))}
    </div>
  );
}
