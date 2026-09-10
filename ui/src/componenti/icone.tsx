/** Le icone: tratto 1,5 px, nessun riempimento, nessun colore proprio.
 *
 *  Sono disegnate qui invece di arrivare da una libreria per due motivi: sono
 *  sei, e una libreria di icone porta con se' migliaia di percorsi che non
 *  servono e uno stile che non e' questo. */

interface Props {
  size?: number;
}

const comuni = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
});

export const Carica = ({ size = 18 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M4 15a4 4 0 0 1 .6-7.9 6 6 0 0 1 11.4-1.3A4.5 4.5 0 0 1 20 15" />
    <path d="M12 12v7M9 15l3-3 3 3" />
  </svg>
);

export const Modifica = ({ size = 18 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M4 20h4l10-10a2.8 2.8 0 0 0-4-4L4 16v4Z" />
    <path d="M13.5 6.5 17.5 10.5" />
  </svg>
);

export const Esporta = ({ size = 18 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M12 16V4M8.5 7.5 12 4l3.5 3.5" />
    <path d="M5 14v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4" />
  </svg>
);

export const Impostazioni = ({ size = 18 }: Props) => (
  <svg {...comuni(size)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 3v2m0 14v2M3 12h2m14 0h2M5.6 5.6 7 7m10 10 1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4" />
  </svg>
);

export const Filmato = ({ size = 15 }: Props) => (
  <svg {...comuni(size)}>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <path d="m10 9 5 3-5 3V9Z" />
  </svg>
);

export const Suono = ({ size = 15 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M4 9v6h3.5L12 19V5L7.5 9H4Z" />
    <path d="M16 9.5a3.5 3.5 0 0 1 0 5M18.5 7a7 7 0 0 1 0 10" />
  </svg>
);

export const File = ({ size = 15 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z" />
    <path d="M14 3v5h5" />
  </svg>
);

export const Play = ({ size = 15 }: Props) => (
  <svg {...comuni(size)} fill="currentColor" stroke="none">
    <path d="M8 5.5v13l11-6.5-11-6.5Z" />
  </svg>
);

export const Pausa = ({ size = 15 }: Props) => (
  <svg {...comuni(size)} fill="currentColor" stroke="none">
    <rect x="7" y="5" width="3.5" height="14" rx="1" />
    <rect x="13.5" y="5" width="3.5" height="14" rx="1" />
  </svg>
);

export const Cartella = ({ size = 15 }: Props) => (
  <svg {...comuni(size)}>
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />
  </svg>
);

export const Marchio = ({ size = 15 }: Props) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
    <path
      d="M5 6.5 12 18l3.2-5.3"
      stroke="#fff"
      strokeWidth="2.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
    <path d="M17.5 5.5 20 5" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
  </svg>
);
