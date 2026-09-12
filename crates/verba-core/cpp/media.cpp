// Lettura di un file multimediale con libavformat / libavcodec.
//
// Serve a due cose: sapere com'e' fatto il file (proporzioni, durata, frame
// rate, se ha una traccia audio) e ottenere il fotogramma visibile a un dato
// istante, per l'anteprima e per l'export dei sottotitoli impressi.
//
// La conversione a RGBA e' scritta qui invece di passare da libswscale, per la
// stessa ragione per cui l'encoder non la usa nell'altra direzione: e' una
// dipendenza di sistema in meno, e le matrici sono le stesse.

#include "media.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/imgutils.h>
#include <libavutil/pixdesc.h>
#include <libavutil/version.h>
}

// La durata di un fotogramma si chiama `duration` dalla 6.0 di FFmpeg; prima
// era `pkt_duration`, che la 7.0 ha rimosso. Ubuntu 22.04 si ferma alla 4.4,
// quindi chi compila li' — e il runner che costruisce il pacchetto `.deb` —
// vede solo il nome vecchio, e chi ha una FFmpeg recente solo il nuovo.
#if LIBAVUTIL_VERSION_INT >= AV_VERSION_INT(57, 30, 100)
#  define VERBA_DURATA_FOTOGRAMMA(f) ((f)->duration)
#else
#  define VERBA_DURATA_FOTOGRAMMA(f) ((f)->pkt_duration)
#endif

namespace {

void scrivi_errore(char *dest, int len, const std::string &testo) {
    if (!dest || len <= 0) return;
    std::snprintf(dest, static_cast<size_t>(len), "%s", testo.c_str());
}

std::string errore_av(const std::string &contesto, int codice) {
    char buf[AV_ERROR_MAX_STRING_SIZE] = {0};
    av_strerror(codice, buf, sizeof(buf));
    return contesto + ": " + buf;
}

// I coefficienti della matrice YUV -> RGB, per spazio colore.
struct Matrice {
    double kr;
    double kb;
};

// Quando il file non dichiara lo spazio colore si applica la convenzione che
// usano i riproduttori: BT.601 fino alla definizione standard, BT.709 sopra.
// Sbagliare qui non produce un errore ma colori spenti o slavati, che e'
// peggio perche' sembra un difetto del disegno.
Matrice matrice_di(AVColorSpace spazio, int altezza) {
    switch (spazio) {
        case AVCOL_SPC_BT470BG:
        case AVCOL_SPC_SMPTE170M:
            return {0.299, 0.114};              // BT.601
        case AVCOL_SPC_BT2020_NCL:
        case AVCOL_SPC_BT2020_CL:
            return {0.2627, 0.0593};            // BT.2020
        case AVCOL_SPC_BT709:
            return {0.2126, 0.0722};
        default:
            return altezza > 576 ? Matrice{0.2126, 0.0722} : Matrice{0.299, 0.114};
    }
}

struct Piani {
    const uint8_t *y;
    const uint8_t *u;
    const uint8_t *v;
    int passo_y;
    int passo_u;
    int passo_v;
    // Sottocampionamento della crominanza.
    int div_x;
    int div_y;
    // 1 = 8 bit, 2 = 16 bit little-endian con `bit` bit significativi.
    int byte_per_campione;
    int bit;
    // NV12/NV21: crominanza interlacciata in un piano solo.
    int interlacciata;
    int u_prima;
};

int leggi(const uint8_t *p, int byte_per_campione) {
    return byte_per_campione == 1 ? *p : (p[0] | (p[1] << 8));
}

// Riempie `piani` per i formati che si incontrano davvero all'uscita di un
// decoder video. Ritorna 0 se il formato non e' fra questi.
int descrivi(const AVFrame *f, Piani *piani) {
    piani->y = f->data[0];
    piani->u = f->data[1];
    piani->v = f->data[2];
    piani->passo_y = f->linesize[0];
    piani->passo_u = f->linesize[1];
    piani->passo_v = f->linesize[2];
    piani->byte_per_campione = 1;
    piani->bit = 8;
    piani->interlacciata = 0;
    piani->u_prima = 1;

    switch (f->format) {
        case AV_PIX_FMT_YUV420P:
        case AV_PIX_FMT_YUVJ420P:
            piani->div_x = 2; piani->div_y = 2; return 1;
        case AV_PIX_FMT_YUV422P:
        case AV_PIX_FMT_YUVJ422P:
            piani->div_x = 2; piani->div_y = 1; return 1;
        case AV_PIX_FMT_YUV444P:
        case AV_PIX_FMT_YUVJ444P:
            piani->div_x = 1; piani->div_y = 1; return 1;
        case AV_PIX_FMT_NV12:
            piani->div_x = 2; piani->div_y = 2;
            piani->interlacciata = 1; piani->u_prima = 1; return 1;
        case AV_PIX_FMT_NV21:
            piani->div_x = 2; piani->div_y = 2;
            piani->interlacciata = 1; piani->u_prima = 0; return 1;
        case AV_PIX_FMT_YUV420P10LE:
            piani->div_x = 2; piani->div_y = 2;
            piani->byte_per_campione = 2; piani->bit = 10; return 1;
        case AV_PIX_FMT_YUV422P10LE:
            piani->div_x = 2; piani->div_y = 1;
            piani->byte_per_campione = 2; piani->bit = 10; return 1;
        case AV_PIX_FMT_YUV444P10LE:
            piani->div_x = 1; piani->div_y = 1;
            piani->byte_per_campione = 2; piani->bit = 10; return 1;
        case AV_PIX_FMT_YUV420P12LE:
            piani->div_x = 2; piani->div_y = 2;
            piani->byte_per_campione = 2; piani->bit = 12; return 1;
        default:
            return 0;
    }
}

uint8_t a_byte(double v) {
    return static_cast<uint8_t>(std::lround(std::clamp(v, 0.0, 255.0)));
}

// Converte il fotogramma in RGBA opaco.
int in_rgba(const AVFrame *f, uint8_t *rgba, int passo, std::string *errore) {
    Piani p;
    if (!descrivi(f, &p)) {
        const char *nome = av_get_pix_fmt_name(static_cast<AVPixelFormat>(f->format));
        *errore = std::string("formato dei pixel non gestito: ") + (nome ? nome : "sconosciuto");
        return -1;
    }

    const Matrice m = matrice_di(f->colorspace, f->height);
    const double kr = m.kr, kb = m.kb, kg = 1.0 - kr - kb;

    // Gamma di valori: i formati "J" e color_range JPEG usano tutta la scala.
    const bool piena = f->color_range == AVCOL_RANGE_JPEG ||
                       f->format == AV_PIX_FMT_YUVJ420P ||
                       f->format == AV_PIX_FMT_YUVJ422P ||
                       f->format == AV_PIX_FMT_YUVJ444P;

    const double massimo = static_cast<double>((1 << p.bit) - 1);
    const double scala = 255.0 / massimo;
    const double y_min = piena ? 0.0 : 16.0 / 255.0 * massimo;
    const double y_max = piena ? massimo : 235.0 / 255.0 * massimo;
    const double y_guadagno = 255.0 / ((y_max - y_min) * scala);
    const double c_centro = static_cast<double>(1 << (p.bit - 1));
    const double c_ampiezza = piena ? massimo : (240.0 - 16.0) / 255.0 * massimo;
    const double c_guadagno = 255.0 / (c_ampiezza * scala);

    if (passo <= 0) passo = f->width * 4;

    for (int y = 0; y < f->height; ++y) {
        uint8_t *riga = rgba + static_cast<size_t>(y) * passo;
        const uint8_t *riga_y = p.y + static_cast<size_t>(y) * p.passo_y;
        const int cy = y / p.div_y;

        for (int x = 0; x < f->width; ++x) {
            const int cx = x / p.div_x;
            const double luma =
                (leggi(riga_y + static_cast<size_t>(x) * p.byte_per_campione,
                       p.byte_per_campione) - y_min) * scala * y_guadagno;

            double cu, cv;
            if (p.interlacciata) {
                const uint8_t *c = p.u + static_cast<size_t>(cy) * p.passo_u + cx * 2;
                const double a = (c[0] - c_centro) * scala * c_guadagno;
                const double b = (c[1] - c_centro) * scala * c_guadagno;
                cu = p.u_prima ? a : b;
                cv = p.u_prima ? b : a;
            } else {
                cu = (leggi(p.u + static_cast<size_t>(cy) * p.passo_u +
                            static_cast<size_t>(cx) * p.byte_per_campione,
                            p.byte_per_campione) - c_centro) * scala * c_guadagno;
                cv = (leggi(p.v + static_cast<size_t>(cy) * p.passo_v +
                            static_cast<size_t>(cx) * p.byte_per_campione,
                            p.byte_per_campione) - c_centro) * scala * c_guadagno;
            }

            const double r = luma + 2.0 * (1.0 - kr) * cv;
            const double b = luma + 2.0 * (1.0 - kb) * cu;
            const double g = luma - (2.0 * (1.0 - kr) * kr * cv +
                                     2.0 * (1.0 - kb) * kb * cu) / kg;

            uint8_t *px = riga + static_cast<size_t>(x) * 4;
            px[0] = a_byte(r);
            px[1] = a_byte(g);
            px[2] = a_byte(b);
            px[3] = 255;
        }
    }
    return 0;
}

}  // namespace

struct VerbaMedia {
    AVFormatContext *fmt = nullptr;
    AVCodecContext *video = nullptr;
    AVFrame *frame = nullptr;
    AVPacket *pkt = nullptr;
    int indice_video = -1;
    int indice_audio = -1;
    // Ultimo istante raggiunto, per capire se conviene riavvolgere.
    double ultimo_pts = -1.0;
};

extern "C" {

VerbaMedia *verba_media_apri(const char *percorso, VerbaInfo *info,
                             char *errore, int errore_len) {
    if (!percorso || !info) {
        scrivi_errore(errore, errore_len, "argomenti non validi");
        return nullptr;
    }
    std::memset(info, 0, sizeof(*info));

    VerbaMedia *m = new VerbaMedia();
    int ret = avformat_open_input(&m->fmt, percorso, nullptr, nullptr);
    if (ret < 0) {
        scrivi_errore(errore, errore_len, errore_av(std::string("apertura di ") + percorso, ret));
        delete m;
        return nullptr;
    }
    ret = avformat_find_stream_info(m->fmt, nullptr);
    if (ret < 0) {
        scrivi_errore(errore, errore_len, errore_av("lettura delle tracce", ret));
        verba_media_libera(m);
        return nullptr;
    }

    m->indice_video = av_find_best_stream(m->fmt, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
    m->indice_audio = av_find_best_stream(m->fmt, AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);

    if (m->fmt->duration != AV_NOPTS_VALUE) {
        info->durata = static_cast<double>(m->fmt->duration) / AV_TIME_BASE;
    }

    if (m->indice_audio >= 0) {
        info->ha_audio = 1;
        // Il nome del codec, non quello del decodificatore: "mp3" e non
        // "mp3float", che e' un dettaglio di implementazione di ffmpeg.
        std::snprintf(info->codec_audio, sizeof(info->codec_audio), "%s",
                      avcodec_get_name(
                          m->fmt->streams[m->indice_audio]->codecpar->codec_id));
    }

    if (m->indice_video >= 0) {
        AVStream *st = m->fmt->streams[m->indice_video];
        // Le copertine incorporate nei file audio sono tracce video: contarle
        // come video farebbe passare un mp3 per un filmato.
        if (st->disposition & AV_DISPOSITION_ATTACHED_PIC) {
            m->indice_video = -1;
        } else {
            info->ha_video = 1;
            info->larghezza = st->codecpar->width;
            info->altezza = st->codecpar->height;

            AVRational fps = av_guess_frame_rate(m->fmt, st, nullptr);
            if (fps.num <= 0 || fps.den <= 0) fps = AVRational{25, 1};
            info->fps_num = fps.num;
            info->fps_den = fps.den;

            std::snprintf(info->codec_video, sizeof(info->codec_video), "%s",
                          avcodec_get_name(st->codecpar->codec_id));
            const AVCodec *c = avcodec_find_decoder(st->codecpar->codec_id);

            if (c) {
                m->video = avcodec_alloc_context3(c);
                if (m->video &&
                    avcodec_parameters_to_context(m->video, st->codecpar) >= 0) {
                    m->video->thread_count = 0;
                    if (avcodec_open2(m->video, c, nullptr) < 0) {
                        avcodec_free_context(&m->video);
                    }
                }
            }
        }
    }

    if (!info->ha_video && !info->ha_audio) {
        scrivi_errore(errore, errore_len,
                      "il file non contiene tracce audio ne' video utilizzabili");
        verba_media_libera(m);
        return nullptr;
    }

    m->frame = av_frame_alloc();
    m->pkt = av_packet_alloc();
    if (!m->frame || !m->pkt) {
        scrivi_errore(errore, errore_len, "memoria insufficiente");
        verba_media_libera(m);
        return nullptr;
    }
    return m;
}

int verba_media_fotogramma(VerbaMedia *m, double t, uint8_t *rgba, int passo,
                           char *errore, int errore_len) {
    if (!m || !rgba) {
        scrivi_errore(errore, errore_len, "argomenti non validi");
        return -1;
    }
    if (m->indice_video < 0 || !m->video) {
        scrivi_errore(errore, errore_len, "il file non ha una traccia video decodificabile");
        return -1;
    }
    if (t < 0.0) t = 0.0;

    AVStream *st = m->fmt->streams[m->indice_video];

    // Si riavvolge solo quando serve: scorrere in avanti — cioe' cio' che fa
    // l'export — non deve costare una ricerca per fotogramma.
    const bool indietro = m->ultimo_pts < 0.0 || t < m->ultimo_pts;
    if (indietro) {
        int64_t ts = static_cast<int64_t>(t / av_q2d(st->time_base));
        int ret = av_seek_frame(m->fmt, m->indice_video, ts, AVSEEK_FLAG_BACKWARD);
        if (ret < 0) {
            scrivi_errore(errore, errore_len, errore_av("ricerca nel file", ret));
            return -1;
        }
        avcodec_flush_buffers(m->video);
        m->ultimo_pts = -1.0;
    }

    // Si decodifica finche' non si raggiunge l'istante chiesto; l'ultimo
    // fotogramma che comincia prima di `t` e' quello visibile.
    bool trovato = false;
    while (true) {
        int ret = av_read_frame(m->fmt, m->pkt);
        if (ret == AVERROR_EOF) {
            avcodec_send_packet(m->video, nullptr);   // svuota il decoder
        } else if (ret < 0) {
            scrivi_errore(errore, errore_len, errore_av("lettura del pacchetto", ret));
            return -1;
        } else if (m->pkt->stream_index != m->indice_video) {
            av_packet_unref(m->pkt);
            continue;
        } else {
            ret = avcodec_send_packet(m->video, m->pkt);
            av_packet_unref(m->pkt);
            if (ret < 0 && ret != AVERROR(EAGAIN)) {
                scrivi_errore(errore, errore_len, errore_av("decodifica", ret));
                return -1;
            }
        }

        while (true) {
            int r = avcodec_receive_frame(m->video, m->frame);
            if (r == AVERROR(EAGAIN)) break;
            if (r == AVERROR_EOF) {
                // Oltre la fine: se qualcosa era stato decodificato vale
                // quello, altrimenti si dichiara la fine del file.
                return trovato ? 0 : 1;
            }
            if (r < 0) {
                scrivi_errore(errore, errore_len, errore_av("decodifica", r));
                return -1;
            }

            const int64_t pts = m->frame->best_effort_timestamp != AV_NOPTS_VALUE
                                    ? m->frame->best_effort_timestamp
                                    : m->frame->pts;
            const double istante =
                pts == AV_NOPTS_VALUE ? 0.0 : pts * av_q2d(st->time_base);

            std::string problema;
            if (in_rgba(m->frame, rgba, passo, &problema) < 0) {
                scrivi_errore(errore, errore_len, problema);
                return -1;
            }
            trovato = true;
            m->ultimo_pts = istante;

            // Il fotogramma successivo comincia dopo `t`: questo e' quello
            // visibile.
            const int64_t durata_tick = VERBA_DURATA_FOTOGRAMMA(m->frame);
            const double durata = durata_tick > 0
                ? durata_tick * av_q2d(st->time_base)
                : 0.0;
            if (istante + durata > t) {
                return 0;
            }
        }
    }
}

void verba_media_libera(VerbaMedia *m) {
    if (!m) return;
    if (m->video) avcodec_free_context(&m->video);
    if (m->fmt) avformat_close_input(&m->fmt);
    if (m->frame) av_frame_free(&m->frame);
    if (m->pkt) av_packet_free(&m->pkt);
    delete m;
}

}  // extern "C"
