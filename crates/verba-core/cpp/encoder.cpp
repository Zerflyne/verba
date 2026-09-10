#include "encoder.h"

#include <cstdio>
#include <cstring>
#include <string>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/avutil.h>
#include <libavutil/opt.h>
}

namespace {

void copia_errore(char *buf, int len, const std::string &msg) {
    if (buf == nullptr || len <= 0) {
        return;
    }
    std::snprintf(buf, static_cast<size_t>(len), "%s", msg.c_str());
}

std::string errore_av(const std::string &contesto, int codice) {
    char tmp[AV_ERROR_MAX_STRING_SIZE] = {0};
    av_strerror(codice, tmp, sizeof(tmp));
    return contesto + ": " + tmp;
}

/*
 * Coefficienti BT.709. La luminanza e la crominanza vanno in "range video"
 * (16..235 e 16..240, riscalati alla profondita' del formato), che e' quanto si
 * aspetta ogni encoder video; l'alfa invece e' a range pieno, come in tutti i
 * formati con alfa di FFmpeg.
 */
constexpr double KR = 0.2126;
constexpr double KB = 0.0722;
constexpr double KG = 1.0 - KR - KB;

/* Contributo normalizzato di ciascun canale, in [0, 1] per Y e [-0.5, 0.5]
 * per U e V. La scala alla profondita' voluta arriva dopo. */
struct TabelleColore {
    double yr[256], yg[256], yb[256];
    double ur[256], ug[256], ub[256];
    double vr[256], vg[256], vb[256];

    TabelleColore() {
        const double du = 2.0 * (1.0 - KB);
        const double dv = 2.0 * (1.0 - KR);
        for (int i = 0; i < 256; ++i) {
            const double v = static_cast<double>(i) / 255.0;
            yr[i] = KR * v;
            yg[i] = KG * v;
            yb[i] = KB * v;
            ur[i] = (-KR / du) * v;
            ug[i] = (-KG / du) * v;
            ub[i] = ((1.0 - KB) / du) * v;
            vr[i] = ((1.0 - KR) / dv) * v;
            vg[i] = (-KG / dv) * v;
            vb[i] = (-KB / dv) * v;
        }
    }
};

const TabelleColore &tabelle() {
    static const TabelleColore t;
    return t;
}

inline int limita(double v, int minimo, int massimo) {
    const int i = static_cast<int>(v + 0.5);
    if (i < minimo) return minimo;
    if (i > massimo) return massimo;
    return i;
}

/* Come e' fatto il formato dei pixel di destinazione. */
struct Descrizione {
    AVPixelFormat pix;
    int profondita;      /* 8 o 10 bit */
    int sotto_x;         /* 1 = 4:4:4, 2 = crominanza dimezzata in orizzontale */
    int sotto_y;
    bool alfa;
};

/* Scrive un campione alla profondita' data. */
inline void scrivi(uint8_t *piano, int passo, int x, int y, int profondita, int v) {
    if (profondita == 8) {
        piano[static_cast<size_t>(y) * passo + x] = static_cast<uint8_t>(v);
    } else {
        reinterpret_cast<uint16_t *>(piano + static_cast<size_t>(y) * passo)[x] =
            static_cast<uint16_t>(v);
    }
}

/*
 * Riempie il frame partendo da RGBA.
 *
 * Quando la crominanza e' sottocampionata si media prima l'RGB del blocco e
 * poi si converte: mediare i valori di crominanza gia' convertiti produce
 * frange colorate sui bordi netti, che e' esattamente dove sta il testo.
 */
void riempi(AVFrame *frame, const Descrizione &d, const uint8_t *rgba, int passo,
            int w, int h) {
    const TabelleColore &t = tabelle();
    const int massimo = (1 << d.profondita) - 1;
    const double scala_y = 219.0 * (1 << (d.profondita - 8));
    const double scala_c = 224.0 * (1 << (d.profondita - 8));
    const double base_y = 16.0 * (1 << (d.profondita - 8));
    const double centro_c = 1 << (d.profondita - 1);

    for (int y = 0; y < h; ++y) {
        const uint8_t *src = rgba + static_cast<size_t>(y) * passo;
        for (int x = 0; x < w; ++x) {
            const uint8_t r = src[x * 4 + 0];
            const uint8_t g = src[x * 4 + 1];
            const uint8_t b = src[x * 4 + 2];
            scrivi(frame->data[0], frame->linesize[0], x, y, d.profondita,
                   limita(base_y + (t.yr[r] + t.yg[g] + t.yb[b]) * scala_y,
                          static_cast<int>(base_y), massimo));
            if (d.alfa) {
                const int a = src[x * 4 + 3];
                scrivi(frame->data[3], frame->linesize[3], x, y, d.profondita,
                       (a * massimo + 127) / 255);
            }
        }
    }

    const int cw = (w + d.sotto_x - 1) / d.sotto_x;
    const int ch = (h + d.sotto_y - 1) / d.sotto_y;
    for (int cy = 0; cy < ch; ++cy) {
        for (int cx = 0; cx < cw; ++cx) {
            int somma_r = 0, somma_g = 0, somma_b = 0, n = 0;
            for (int dy = 0; dy < d.sotto_y; ++dy) {
                const int y = cy * d.sotto_y + dy;
                if (y >= h) break;
                const uint8_t *src = rgba + static_cast<size_t>(y) * passo;
                for (int dx = 0; dx < d.sotto_x; ++dx) {
                    const int x = cx * d.sotto_x + dx;
                    if (x >= w) break;
                    somma_r += src[x * 4 + 0];
                    somma_g += src[x * 4 + 1];
                    somma_b += src[x * 4 + 2];
                    ++n;
                }
            }
            const int r = (somma_r + n / 2) / n;
            const int g = (somma_g + n / 2) / n;
            const int b = (somma_b + n / 2) / n;
            const int minimo_c = static_cast<int>(centro_c - scala_c / 2.0);
            const int massimo_c = static_cast<int>(centro_c + scala_c / 2.0);
            scrivi(frame->data[1], frame->linesize[1], cx, cy, d.profondita,
                   limita(centro_c + (t.ur[r] + t.ug[g] + t.ub[b]) * scala_c,
                          minimo_c, massimo_c));
            scrivi(frame->data[2], frame->linesize[2], cx, cy, d.profondita,
                   limita(centro_c + (t.vr[r] + t.vg[g] + t.vb[b]) * scala_c,
                          minimo_c, massimo_c));
        }
    }
}

/* Il contenitore, il codificatore e il formato dei pixel di ogni uscita. */
struct Profilo {
    const char *contenitore;
    const char *codec;
    const char *estensione;
    Descrizione pixel;
    int qualita_consigliata;
};

const Profilo *profilo_di(int formato) {
    static const Profilo profili[] = {
        {"mov",  "prores_ks",  "mov",
         {AV_PIX_FMT_YUVA444P10LE, 10, 1, 1, true},  4},
        {"mov",  "prores_ks",  "mov",
         {AV_PIX_FMT_YUV422P10LE, 10, 2, 1, false},  4},
        {"mp4",  "libx264",    "mp4",
         {AV_PIX_FMT_YUV420P,      8, 2, 2, false}, 18},
        {"webm", "libvpx-vp9", "webm",
         {AV_PIX_FMT_YUVA420P,     8, 2, 2, true},  24},
    };
    if (formato < 0 || formato > 3) return nullptr;
    return &profili[formato];
}

}  // namespace

struct SubEncoder {
    Descrizione pixel{};
    AVFormatContext *fmt = nullptr;
    AVCodecContext *ctx = nullptr;
    AVStream *stream = nullptr;
    AVFrame *frame = nullptr;
    AVPacket *pkt = nullptr;
    int64_t pts = 0;
    bool header_scritto = false;

    /* Traccia audio copiata dal file di partenza, se richiesta. */
    AVFormatContext *audio_in = nullptr;
    AVStream *audio_out = nullptr;
    AVPacket *audio_pkt = nullptr;
    int audio_idx = -1;
    bool audio_finito = false;
};

namespace {

/* Manda il frame all'encoder e drena i pacchetti pronti. */
int codifica(SubEncoder *enc, AVFrame *frame, std::string *errore) {
    int ret = avcodec_send_frame(enc->ctx, frame);
    if (ret < 0) {
        *errore = errore_av("invio del frame all'encoder", ret);
        return ret;
    }
    while (true) {
        ret = avcodec_receive_packet(enc->ctx, enc->pkt);
        if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
            return 0;
        }
        if (ret < 0) {
            *errore = errore_av("ricezione del pacchetto codificato", ret);
            return ret;
        }
        av_packet_rescale_ts(enc->pkt, enc->ctx->time_base, enc->stream->time_base);
        enc->pkt->stream_index = enc->stream->index;
        ret = av_interleaved_write_frame(enc->fmt, enc->pkt);
        av_packet_unref(enc->pkt);
        if (ret < 0) {
            *errore = errore_av("scrittura del pacchetto nel contenitore", ret);
            return ret;
        }
    }
}

/*
 * Scrive i pacchetti audio fino all'istante `fino_a` (in secondi).
 *
 * Si procede a pari passo con il video invece di scrivere tutto l'audio alla
 * fine: cosi' il multiplexer non deve tenere in memoria un'intera traccia per
 * poterla interlacciare. `fino_a` negativo significa "tutto quello che resta".
 */
int drena_audio(SubEncoder *enc, double fino_a, std::string *errore) {
    if (enc->audio_in == nullptr || enc->audio_finito) {
        return 0;
    }
    AVStream *in = enc->audio_in->streams[enc->audio_idx];
    while (true) {
        int ret = av_read_frame(enc->audio_in, enc->audio_pkt);
        if (ret == AVERROR_EOF) {
            enc->audio_finito = true;
            return 0;
        }
        if (ret < 0) {
            *errore = errore_av("lettura della traccia audio", ret);
            return ret;
        }
        if (enc->audio_pkt->stream_index != enc->audio_idx) {
            av_packet_unref(enc->audio_pkt);
            continue;
        }

        const int64_t pts = enc->audio_pkt->pts != AV_NOPTS_VALUE
                                ? enc->audio_pkt->pts
                                : enc->audio_pkt->dts;
        const double istante = pts == AV_NOPTS_VALUE ? 0.0 : pts * av_q2d(in->time_base);

        av_packet_rescale_ts(enc->audio_pkt, in->time_base, enc->audio_out->time_base);
        enc->audio_pkt->stream_index = enc->audio_out->index;
        enc->audio_pkt->pos = -1;
        ret = av_interleaved_write_frame(enc->fmt, enc->audio_pkt);
        av_packet_unref(enc->audio_pkt);
        if (ret < 0) {
            *errore = errore_av("scrittura del pacchetto audio", ret);
            return ret;
        }
        if (fino_a >= 0.0 && istante > fino_a) {
            return 0;
        }
    }
}

}  // namespace

extern "C" int sub_formato_ha_alfa(int formato) {
    const Profilo *p = profilo_di(formato);
    return p != nullptr && p->pixel.alfa ? 1 : 0;
}

extern "C" const char *sub_formato_estensione(int formato) {
    const Profilo *p = profilo_di(formato);
    return p != nullptr ? p->estensione : "";
}

extern "C" SubEncoder *sub_encoder_apri(const char *percorso,
                                        int formato,
                                        const char *audio_da,
                                        int larghezza,
                                        int altezza,
                                        int fps_num,
                                        int fps_den,
                                        int qualita,
                                        int thread,
                                        char *errore,
                                        int errore_len) {
    if (percorso == nullptr || larghezza <= 0 || altezza <= 0 || fps_num <= 0 || fps_den <= 0) {
        copia_errore(errore, errore_len, "parametri non validi");
        return nullptr;
    }
    const Profilo *profilo = profilo_di(formato);
    if (profilo == nullptr) {
        copia_errore(errore, errore_len, "formato di uscita sconosciuto");
        return nullptr;
    }
    // Tutti gli encoder in questione lavorano a macroblocchi: le dimensioni
    // dispari sono rifiutate.
    if ((larghezza % 2) != 0 || (altezza % 2) != 0) {
        copia_errore(errore, errore_len, "larghezza e altezza devono essere pari");
        return nullptr;
    }

    SubEncoder *enc = new SubEncoder();
    enc->pixel = profilo->pixel;
    std::string msg;

    int ret = avformat_alloc_output_context2(&enc->fmt, nullptr, profilo->contenitore, percorso);
    if (ret < 0 || enc->fmt == nullptr) {
        copia_errore(errore, errore_len,
                     errore_av(std::string("creazione del contenitore ") + profilo->contenitore,
                               ret));
        sub_encoder_libera(enc);
        return nullptr;
    }

    const AVCodec *codec = avcodec_find_encoder_by_name(profilo->codec);
    if (codec == nullptr) {
        copia_errore(errore, errore_len,
                     std::string("encoder ") + profilo->codec +
                         " non disponibile in questa build di libavcodec");
        sub_encoder_libera(enc);
        return nullptr;
    }

    enc->ctx = avcodec_alloc_context3(codec);
    if (enc->ctx == nullptr) {
        copia_errore(errore, errore_len, "allocazione del contesto dell'encoder fallita");
        sub_encoder_libera(enc);
        return nullptr;
    }

    enc->ctx->width = larghezza;
    enc->ctx->height = altezza;
    enc->ctx->pix_fmt = profilo->pixel.pix;
    enc->ctx->time_base = AVRational{fps_den, fps_num};
    enc->ctx->framerate = AVRational{fps_num, fps_den};
    enc->ctx->sample_aspect_ratio = AVRational{1, 1};
    enc->ctx->colorspace = AVCOL_SPC_BT709;
    enc->ctx->color_primaries = AVCOL_PRI_BT709;
    enc->ctx->color_trc = AVCOL_TRC_BT709;
    enc->ctx->color_range = AVCOL_RANGE_MPEG;

    const int q = qualita > 0 ? qualita : profilo->qualita_consigliata;

    switch (formato) {
        case SUB_FORMATO_PRORES_4444:
        case SUB_FORMATO_PRORES_422: {
            // prores_ks lavora a fette: oltre una quindicina di thread il
            // guadagno sparisce e libavcodec stesso lo segnala.
            const int thread_usati = thread > 0 ? (thread < 16 ? thread : 16) : 0;
            enc->ctx->thread_count = thread_usati;
            if (thread_usati > 1) enc->ctx->thread_type = FF_THREAD_SLICE;
            // ProRes non ha inter-frame: ogni frame e' un keyframe.
            enc->ctx->gop_size = 1;
            enc->ctx->max_b_frames = 0;
            // profilo 4 = 4444 (l'unico, con 4444xq, che porta l'alfa);
            // profilo 3 = 422 HQ.
            av_opt_set_int(enc->ctx->priv_data,
                           "profile", formato == SUB_FORMATO_PRORES_4444 ? 4 : 3, 0);
            enc->ctx->flags |= AV_CODEC_FLAG_QSCALE;
            enc->ctx->global_quality = FF_QP2LAMBDA * q;
            // Vendor Apple: senza di questo alcuni montaggi rifiutano il file.
            av_opt_set(enc->ctx->priv_data, "vendor", "apl0", 0);
            break;
        }
        case SUB_FORMATO_H264: {
            enc->ctx->thread_count = thread > 0 ? thread : 0;
            enc->ctx->gop_size = 12 * fps_num / fps_den;
            enc->ctx->max_b_frames = 2;
            av_opt_set_int(enc->ctx->priv_data, "crf", q, 0);
            av_opt_set(enc->ctx->priv_data, "preset", "medium", 0);
            // yuv420p con profilo High: e' cio' che riproduce qualsiasi cosa.
            av_opt_set(enc->ctx->priv_data, "profile", "high", 0);
            break;
        }
        case SUB_FORMATO_VP9_ALPHA: {
            enc->ctx->thread_count = thread > 0 ? thread : 0;
            enc->ctx->gop_size = 12 * fps_num / fps_den;
            enc->ctx->max_b_frames = 0;
            av_opt_set_int(enc->ctx->priv_data, "crf", q, 0);
            // Con il CRF il bitrate va messo a zero, altrimenti libvpx passa
            // in modalita' a bitrate costante e il CRF viene ignorato.
            enc->ctx->bit_rate = 0;
            av_opt_set(enc->ctx->priv_data, "deadline", "good", 0);
            av_opt_set_int(enc->ctx->priv_data, "cpu-used", 2, 0);
            av_opt_set_int(enc->ctx->priv_data, "row-mt", 1, 0);
            break;
        }
        default:
            break;
    }

    if ((enc->fmt->oformat->flags & AVFMT_GLOBALHEADER) != 0) {
        enc->ctx->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    }

    ret = avcodec_open2(enc->ctx, codec, nullptr);
    if (ret < 0) {
        copia_errore(errore, errore_len,
                     errore_av(std::string("apertura di ") + profilo->codec, ret));
        sub_encoder_libera(enc);
        return nullptr;
    }

    enc->stream = avformat_new_stream(enc->fmt, nullptr);
    if (enc->stream == nullptr) {
        copia_errore(errore, errore_len, "creazione della traccia video fallita");
        sub_encoder_libera(enc);
        return nullptr;
    }
    enc->stream->time_base = enc->ctx->time_base;
    enc->stream->avg_frame_rate = enc->ctx->framerate;
    ret = avcodec_parameters_from_context(enc->stream->codecpar, enc->ctx);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("copia dei parametri nella traccia", ret));
        sub_encoder_libera(enc);
        return nullptr;
    }

    if ((enc->fmt->oformat->flags & AVFMT_NOFILE) == 0) {
        ret = avio_open(&enc->fmt->pb, percorso, AVIO_FLAG_WRITE);
        if (ret < 0) {
            copia_errore(errore, errore_len,
                         errore_av(std::string("apertura di ") + percorso, ret));
            sub_encoder_libera(enc);
            return nullptr;
        }
    }

    // La traccia audio si copia tale e quale: nessuna ricodifica, nessuna
    // perdita. Va aggiunta prima dell'intestazione, che dichiara le tracce.
    if (audio_da != nullptr && audio_da[0] != '\0' && !profilo->pixel.alfa) {
        ret = avformat_open_input(&enc->audio_in, audio_da, nullptr, nullptr);
        if (ret < 0) {
            copia_errore(errore, errore_len,
                         errore_av(std::string("apertura di ") + audio_da, ret));
            sub_encoder_libera(enc);
            return nullptr;
        }
        if (avformat_find_stream_info(enc->audio_in, nullptr) < 0) {
            avformat_close_input(&enc->audio_in);
        } else {
            enc->audio_idx = av_find_best_stream(enc->audio_in, AVMEDIA_TYPE_AUDIO,
                                                 -1, -1, nullptr, 0);
            if (enc->audio_idx < 0) {
                avformat_close_input(&enc->audio_in);
            } else {
                AVStream *in = enc->audio_in->streams[enc->audio_idx];
                enc->audio_out = avformat_new_stream(enc->fmt, nullptr);
                if (enc->audio_out == nullptr ||
                    avcodec_parameters_copy(enc->audio_out->codecpar, in->codecpar) < 0) {
                    copia_errore(errore, errore_len, "copia della traccia audio fallita");
                    sub_encoder_libera(enc);
                    return nullptr;
                }
                enc->audio_out->codecpar->codec_tag = 0;
                enc->audio_out->time_base = in->time_base;
                enc->audio_pkt = av_packet_alloc();
                if (enc->audio_pkt == nullptr) {
                    copia_errore(errore, errore_len, "allocazione del pacchetto audio fallita");
                    sub_encoder_libera(enc);
                    return nullptr;
                }
            }
        }
    }

    ret = avformat_write_header(enc->fmt, nullptr);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("scrittura dell'intestazione", ret));
        sub_encoder_libera(enc);
        return nullptr;
    }
    enc->header_scritto = true;

    enc->frame = av_frame_alloc();
    enc->pkt = av_packet_alloc();
    if (enc->frame == nullptr || enc->pkt == nullptr) {
        copia_errore(errore, errore_len, "allocazione di frame/pacchetto fallita");
        sub_encoder_libera(enc);
        return nullptr;
    }
    enc->frame->format = enc->ctx->pix_fmt;
    enc->frame->width = larghezza;
    enc->frame->height = altezza;
    enc->frame->colorspace = enc->ctx->colorspace;
    enc->frame->color_range = enc->ctx->color_range;
    ret = av_frame_get_buffer(enc->frame, 0);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("allocazione del buffer del frame", ret));
        sub_encoder_libera(enc);
        return nullptr;
    }

    return enc;
}

extern "C" int sub_encoder_scrivi(SubEncoder *enc,
                                  const uint8_t *rgba,
                                  int passo,
                                  int ripetizioni,
                                  char *errore,
                                  int errore_len) {
    if (enc == nullptr || rgba == nullptr) {
        copia_errore(errore, errore_len, "encoder o frame nullo");
        return -1;
    }
    if (ripetizioni <= 0) {
        return 0;
    }
    const int w = enc->ctx->width;
    const int h = enc->ctx->height;
    if (passo < w * 4) {
        copia_errore(errore, errore_len, "passo del frame RGBA troppo corto");
        return -1;
    }

    int ret = av_frame_make_writable(enc->frame);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("frame non scrivibile", ret));
        return ret;
    }

    riempi(enc->frame, enc->pixel, rgba, passo, w, h);

    std::string msg;
    for (int i = 0; i < ripetizioni; ++i) {
        enc->frame->pts = enc->pts++;
        ret = codifica(enc, enc->frame, &msg);
        if (ret < 0) {
            copia_errore(errore, errore_len, msg);
            return ret;
        }
    }

    if (enc->audio_in != nullptr) {
        const double fino_a = enc->pts * av_q2d(enc->ctx->time_base);
        ret = drena_audio(enc, fino_a, &msg);
        if (ret < 0) {
            copia_errore(errore, errore_len, msg);
            return ret;
        }
    }
    return 0;
}

extern "C" int sub_encoder_chiudi(SubEncoder *enc, char *errore, int errore_len) {
    if (enc == nullptr) {
        return -1;
    }
    std::string msg;
    int ret = codifica(enc, nullptr, &msg);  // svuota l'encoder
    if (ret < 0) {
        copia_errore(errore, errore_len, msg);
        return ret;
    }
    ret = drena_audio(enc, -1.0, &msg);      // e cio' che resta dell'audio
    if (ret < 0) {
        copia_errore(errore, errore_len, msg);
        return ret;
    }
    if (enc->header_scritto) {
        ret = av_write_trailer(enc->fmt);
        enc->header_scritto = false;
        if (ret < 0) {
            copia_errore(errore, errore_len, errore_av("scrittura del trailer", ret));
            return ret;
        }
    }
    if (enc->fmt != nullptr && enc->fmt->pb != nullptr) {
        avio_closep(&enc->fmt->pb);
    }
    return 0;
}

extern "C" void sub_encoder_libera(SubEncoder *enc) {
    if (enc == nullptr) {
        return;
    }
    if (enc->audio_pkt != nullptr) {
        av_packet_free(&enc->audio_pkt);
    }
    if (enc->audio_in != nullptr) {
        avformat_close_input(&enc->audio_in);
    }
    if (enc->frame != nullptr) {
        av_frame_free(&enc->frame);
    }
    if (enc->pkt != nullptr) {
        av_packet_free(&enc->pkt);
    }
    if (enc->ctx != nullptr) {
        avcodec_free_context(&enc->ctx);
    }
    if (enc->fmt != nullptr) {
        if (enc->fmt->pb != nullptr) {
            avio_closep(&enc->fmt->pb);
        }
        avformat_free_context(enc->fmt);
        enc->fmt = nullptr;
    }
    delete enc;
}

extern "C" int64_t sub_encoder_frame_scritti(const SubEncoder *enc) {
    return enc == nullptr ? 0 : enc->pts;
}
