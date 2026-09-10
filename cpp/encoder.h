/*
 * Encoder video per sottotitoli con sfondo trasparente.
 *
 * Interfaccia C (ABI stabile) sopra libavcodec + libavformat: il contenitore e'
 * MOV e l'encoder e' prores_ks in profilo 4444, l'unico profilo ProRes che
 * trasporta il canale alfa. Il formato dei pixel interno e' yuva444p10le: 4:4:4
 * senza sottocampionamento della crominanza (i bordi del testo restano netti) e
 * alfa a 10 bit.
 *
 * Il chiamante fornisce frame RGBA8 con alfa "dritta" (non premoltiplicata),
 * cosi' come la producono i compositor e come la si attende un montaggio video.
 */
#ifndef AUTOSUBTITLER_ENCODER_H
#define AUTOSUBTITLER_ENCODER_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct SubEncoder SubEncoder;

/*
 * Apre il file e prepara l'encoder.
 *
 * `qualita` e' il quantizzatore ProRes (parametro -qscale di FFmpeg): valori
 * bassi = piu' qualita' e piu' bit. 4 e' il valore consigliato per il 4444.
 * In caso di errore restituisce NULL e scrive il motivo in `errore`.
 */
SubEncoder *sub_encoder_apri(const char *percorso,
                             int larghezza,
                             int altezza,
                             int fps_num,
                             int fps_den,
                             int qualita,
                             int thread,
                             char *errore,
                             int errore_len);

/*
 * Converte un frame RGBA e lo scrive `ripetizioni` volte.
 *
 * La conversione avviene una sola volta: i sottotitoli restano identici per
 * molti frame consecutivi, e ripetere il frame gia' convertito evita di
 * rifare la trasformazione colore a ogni fotogramma.
 *
 * `passo` e' la lunghezza in byte di una riga di `rgba` (>= larghezza * 4).
 * Restituisce 0 se tutto e' andato bene, altrimenti un valore negativo.
 */
int sub_encoder_scrivi(SubEncoder *enc,
                       const uint8_t *rgba,
                       int passo,
                       int ripetizioni,
                       char *errore,
                       int errore_len);

/* Svuota l'encoder, scrive il trailer e chiude il file. */
int sub_encoder_chiudi(SubEncoder *enc, char *errore, int errore_len);

/* Libera la struttura. Va chiamata anche se `sub_encoder_chiudi` fallisce. */
void sub_encoder_libera(SubEncoder *enc);

/* Numero di frame scritti finora (utile per il log). */
int64_t sub_encoder_frame_scritti(const SubEncoder *enc);

#ifdef __cplusplus
}
#endif

#endif /* AUTOSUBTITLER_ENCODER_H */
