/** Riproduzione, posizione, durata e forma d'onda.
 *
 *  Non c'e' un lettore audio: la riproduzione fa avanzare il tempo, e cio' che
 *  si vede e' l'anteprima ridisegnata. Nella 0.1 e' quello che serve — si
 *  guarda se i sottotitoli cadono al momento giusto, non si ascolta. */

import { useEffect, useRef } from "react";
import { Pausa, Play } from "./icone";
import { orologio } from "./controlli";

interface Props {
  tempo: number;
  durata: number;
  inRiproduzione: boolean;
  onTempo: (t: number) => void;
  onRiproduzione: (v: boolean) => void;
  onda?: number[];
  /** L'intervallo del blocco attualmente a schermo, da colorare nell'onda. */
  bloccoAttivo?: [number, number] | null;
}

export function Trasporto({
  tempo,
  durata,
  inRiproduzione,
  onTempo,
  onRiproduzione,
  onda,
  bloccoAttivo,
}: Props) {
  const ultimo = useRef(0);

  // L'orologio della riproduzione: avanza in tempo reale e si ferma alla fine.
  useEffect(() => {
    if (!inRiproduzione) return;
    ultimo.current = performance.now();
    let vivo = true;
    let t = tempo;
    const passo = () => {
      if (!vivo) return;
      const ora = performance.now();
      t += (ora - ultimo.current) / 1000;
      ultimo.current = ora;
      if (t >= durata) {
        onTempo(durata);
        onRiproduzione(false);
        return;
      }
      onTempo(t);
      requestAnimationFrame(passo);
    };
    const id = requestAnimationFrame(passo);
    return () => {
      vivo = false;
      cancelAnimationFrame(id);
    };
    // `tempo` non e' fra le dipendenze apposta: entrarci farebbe ripartire
    // l'orologio a ogni fotogramma.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inRiproduzione, durata]);

  const vaiA = (e: React.MouseEvent<HTMLElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    onTempo(Math.max(0, Math.min(durata, ((e.clientX - r.left) / r.width) * durata)));
  };

  const q = durata > 0 ? tempo / durata : 0;

  return (
    <>
      <div className="trasporto">
        <button
          className="riproduci"
          onClick={() => onRiproduzione(!inRiproduzione)}
          title={inRiproduzione ? "Metti in pausa" : "Riproduci"}
        >
          {inRiproduzione ? <Pausa /> : <Play />}
        </button>
        <span className="tempo-corrente">
          {orologio(tempo)} / {orologio(durata)}
        </span>
        <span className="scorrimento" onMouseDown={vaiA} onClick={vaiA}>
          <i style={{ width: `${(q * 100).toFixed(2)}%` }} />
        </span>
      </div>
      {onda && onda.length > 0 && (
        <FormaOnda
          onda={onda}
          durata={durata}
          tempo={tempo}
          bloccoAttivo={bloccoAttivo ?? null}
          onTempo={onTempo}
        />
      )}
    </>
  );
}

/** La forma d'onda dell'intero file, col blocco a schermo in `--accent`. */
function FormaOnda({
  onda,
  durata,
  tempo,
  bloccoAttivo,
  onTempo,
}: {
  onda: number[];
  durata: number;
  tempo: number;
  bloccoAttivo: [number, number] | null;
  onTempo: (t: number) => void;
}) {
  const tela = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const c = tela.current;
    if (!c) return;
    const scala = window.devicePixelRatio || 1;
    const larghezza = c.clientWidth;
    const altezza = c.clientHeight;
    c.width = Math.round(larghezza * scala);
    c.height = Math.round(altezza * scala);
    const g = c.getContext("2d");
    if (!g) return;
    g.setTransform(scala, 0, 0, scala, 0, 0);
    g.clearRect(0, 0, larghezza, altezza);

    const passo = 3;
    const barre = Math.floor(larghezza / passo);
    const meta = altezza / 2;

    for (let i = 0; i < barre; i++) {
      const t = (i / barre) * durata;
      const campione = onda[Math.min(onda.length - 1, Math.floor((i / barre) * onda.length))];
      const h = Math.max(1.5, campione * (altezza - 6));
      const dentro = bloccoAttivo && t >= bloccoAttivo[0] && t <= bloccoAttivo[1];
      g.fillStyle = dentro ? "#a78bfa" : t <= tempo ? "#5a5a5a" : "#3a3a3a";
      g.fillRect(i * passo, meta - h / 2, passo - 1.2, h);
    }
  }, [onda, durata, tempo, bloccoAttivo]);

  return (
    <canvas
      className="onda"
      ref={tela}
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        onTempo(Math.max(0, Math.min(durata, ((e.clientX - r.left) / r.width) * durata)));
      }}
    />
  );
}
