/** Riproduzione, posizione, durata e forma d'onda.
 *
 *  L'audio si sente davvero: il motore scrive un WAV temporaneo dal PCM gia'
 *  decodificato e qui lo suona un `<audio>` nascosto. Non si passa il file di
 *  partenza perche' la webview non sa suonare tutto quello che Verba sa
 *  aprire — di un `.mkv` o di un `.opus` resterebbe muta senza dire perche'.
 *
 *  Quando l'audio c'e', **e' lui l'orologio**: la posizione mostrata e' la sua
 *  `currentTime`, non un contatore parallelo. Due orologi distinti sullo
 *  stesso nastro divergono sempre, e la deriva si vedrebbe proprio dove non
 *  deve — nei sottotitoli che arrivano un attimo prima o dopo la voce. Senza
 *  audio (il banco di prova nel browser) resta il contatore, che e' l'unica
 *  cosa che ci sia da far scorrere. */

import { useEffect, useRef, useState } from "react";
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
  /** L'indirizzo della traccia da suonare; senza, il trasporto e' muto. */
  audio?: string | null;
}

/** Oltre questo scarto fra posizione mostrata e posizione dell'audio si tratta
 *  di un salto voluto — un clic sull'onda — e non della normale deriva di un
 *  fotogramma. */
const SALTO = 0.25;

export function Trasporto({
  tempo,
  durata,
  inRiproduzione,
  onTempo,
  onRiproduzione,
  onda,
  bloccoAttivo,
  audio,
}: Props) {
  const elemento = useRef<HTMLAudioElement>(null);
  const ultimo = useRef(0);
  /** L'ultima posizione che abbiamo emesso noi: serve a distinguere un salto
   *  chiesto da fuori dal normale avanzare della riproduzione. */
  const emesso = useRef(0);
  const [muto, setMuto] = useState(false);
  const [errore, setErrore] = useState<string | null>(null);

  // Avvio e arresto. `tempo` non e' fra le dipendenze: entrarci rimetterebbe
  // in moto l'audio a ogni fotogramma.
  useEffect(() => {
    const a = elemento.current;
    if (!a || !audio) return;
    if (inRiproduzione) {
      if (Math.abs(a.currentTime - tempo) > SALTO) a.currentTime = tempo;
      void a.play().catch((e) => setErrore(String(e)));
    } else {
      a.pause();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inRiproduzione, audio]);

  // Un salto chiesto da fuori (un clic sull'onda, una parola della striscia).
  useEffect(() => {
    const a = elemento.current;
    if (!a || !audio) return;
    if (Math.abs(tempo - emesso.current) > SALTO) {
      a.currentTime = tempo;
      emesso.current = tempo;
    }
  }, [tempo, audio]);

  // L'orologio della riproduzione.
  useEffect(() => {
    if (!inRiproduzione) return;
    const a = audio ? elemento.current : null;
    ultimo.current = performance.now();
    let vivo = true;
    let t = tempo;
    let id = 0;

    const passo = () => {
      if (!vivo) return;
      if (a) {
        t = a.currentTime;
        if (a.ended || t >= durata) {
          onTempo(durata);
          onRiproduzione(false);
          return;
        }
      } else {
        const ora = performance.now();
        t += (ora - ultimo.current) / 1000;
        ultimo.current = ora;
        if (t >= durata) {
          onTempo(durata);
          onRiproduzione(false);
          return;
        }
      }
      emesso.current = t;
      onTempo(t);
      id = requestAnimationFrame(passo);
    };

    id = requestAnimationFrame(passo);
    return () => {
      vivo = false;
      cancelAnimationFrame(id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inRiproduzione, durata, audio]);

  const vaiA = (e: React.MouseEvent<HTMLElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    onTempo(Math.max(0, Math.min(durata, ((e.clientX - r.left) / r.width) * durata)));
  };

  const q = durata > 0 ? tempo / durata : 0;

  return (
    <>
      {audio && (
        <audio
          ref={elemento}
          src={audio}
          preload="auto"
          muted={muto}
          onError={() =>
            // Un audio che non parte e non dice niente e' il difetto piu'
            // difficile da capire: qui almeno si legge che e' successo.
            setErrore("la traccia d'anteprima non si e' caricata")
          }
          onPlaying={() => setErrore(null)}
        />
      )}
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
        {audio && (
          <button
            className="riproduci piccolo"
            onClick={() => setMuto(!muto)}
            title={muto ? "Riattiva l'audio" : "Togli l'audio"}
          >
            {muto ? "🔇" : "🔊"}
          </button>
        )}
      </div>
      {errore && <p className="nota errore-audio">{errore}</p>}
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
