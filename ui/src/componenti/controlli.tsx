/** I controlli del pannello di destra.
 *
 *  Nessuno di questi ha un riquadro intorno: i gruppi si separano con lo
 *  spazio e con intestazioni testuali. Il violetto compare solo nel
 *  riempimento del cursore e nel bordo di cio' che e' selezionato. */

import type { ReactNode } from "react";

export function Riga({
  etichetta,
  children,
  spiegazione,
}: {
  etichetta: string;
  children: ReactNode;
  spiegazione?: string;
}) {
  return (
    <>
      <div className="riga-controllo">
        <span className="etichetta">{etichetta}</span>
        {children}
      </div>
      {spiegazione && <p className="spiegazione">{spiegazione}</p>}
    </>
  );
}

export function Cursore({
  valore,
  min,
  max,
  passo = 1,
  onChange,
  formato,
}: {
  valore: number;
  min: number;
  max: number;
  passo?: number;
  onChange: (v: number) => void;
  formato?: (v: number) => string;
}) {
  const q = max > min ? (valore - min) / (max - min) : 0;
  return (
    <div className="cursore">
      <span className="pista">
        <i className="riempimento" style={{ width: `calc(${(q * 100).toFixed(2)}% - ${q * 13}px + 6px)` }} />
        <input
          type="range"
          min={min}
          max={max}
          step={passo}
          value={valore}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      </span>
      <span className="valore">{formato ? formato(valore) : valore}</span>
    </div>
  );
}

export function Interruttore({
  acceso,
  onChange,
}: {
  acceso: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      className={`interruttore${acceso ? " acceso" : ""}`}
      role="switch"
      aria-checked={acceso}
      onClick={() => onChange(!acceso)}
    >
      <i />
    </button>
  );
}

export function Scelta<T extends string | number>({
  valore,
  opzioni,
  onChange,
}: {
  valore: T;
  opzioni: { valore: T; etichetta: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="scelta">
      {opzioni.map((o) => (
        <button
          key={String(o.valore)}
          className={o.valore === valore ? "attiva" : ""}
          onClick={() => onChange(o.valore)}
        >
          {o.etichetta}
        </button>
      ))}
    </div>
  );
}

/** Un colore: il codice esadecimale si scrive, il campione apre il selettore. */
export function Colore({
  valore,
  onChange,
}: {
  valore: string;
  onChange: (v: string) => void;
}) {
  // Il selettore del sistema conosce solo #RRGGBB: l'eventuale alfa scritta a
  // mano viene tenuta da parte e riattaccata.
  const senzaAlfa = valore.slice(0, 7);
  const alfa = valore.length > 7 ? valore.slice(7) : "";
  return (
    <div className="colore">
      <span className="campione" style={{ background: senzaAlfa }}>
        <input
          type="color"
          value={senzaAlfa}
          onChange={(e) => onChange(e.target.value.toUpperCase() + alfa)}
        />
      </span>
      <input
        className="esadecimale"
        value={valore}
        spellCheck={false}
        onChange={(e) => {
          const v = e.target.value.trim();
          if (/^#[0-9a-fA-F]{0,8}$/.test(v)) onChange(v.toUpperCase());
        }}
      />
    </div>
  );
}

export function Gruppo({ titolo, children }: { titolo: string; children: ReactNode }) {
  return (
    <section className="gruppo">
      <h3 className="intestazione-gruppo">{titolo}</h3>
      {children}
    </section>
  );
}

/** `00:12` oppure `1:02:03`. */
export function orologio(secondi: number): string {
  const s = Math.max(0, Math.floor(secondi));
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const dd = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${dd(m % 60)}:${dd(s % 60)}` : `${dd(m)}:${dd(s % 60)}`;
}

/** `2s`, `1m 04s`, `1h 02m`: la stessa forma della riga di comando. */
export function durata(secondi: number): string {
  const s = Math.max(0, Math.round(secondi));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}
