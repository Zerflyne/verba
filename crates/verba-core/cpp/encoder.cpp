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
 * (16..235 e 16..240 riscalati a 10 bit), che e' quanto si aspetta un file
 * ProRes; l'alfa invece e' a range pieno, come in tutti i formati con alfa
 * di FFmpeg.
 */
constexpr double KR = 0.2126;
constexpr double KB = 0.0722;
constexpr double KG = 1.0 - KR - KB;

struct TabelleColore {
    /* y[c] e' il contributo, gia' scalato, del canale c in 8 bit. */
    double yr[256], yg[256], yb[256];
    double ur[256], ug[256], ub[256];
    double vr[256], vg[256], vb[256];
    uint16_t alfa[256];

    TabelleColore() {
        // 10 bit, range video: Y in [64, 940], C in [64, 960] centrata su 512.
        const double scala_y = 876.0 / 255.0;
        const double scala_c = 896.0 / 255.0;
        const double du = 2.0 * (1.0 - KB);
        const double dv = 2.0 * (1.0 - KR);
        for (int i = 0; i < 256; ++i) {
            const double v = static_cast<double>(i);
            yr[i] = KR * v * scala_y;
            yg[i] = KG * v * scala_y;
            yb[i] = KB * v * scala_y;
            ur[i] = (-KR / du) * v * scala_c;
            ug[i] = (-KG / du) * v * scala_c;
            ub[i] = ((1.0 - KB) / du) * v * scala_c;
            vr[i] = ((1.0 - KR) / dv) * v * scala_c;
            vg[i] = (-KG / dv) * v * scala_c;
            vb[i] = (-KB / dv) * v * scala_c;
            // 8 bit -> 10 bit a range pieno, senza perdita di estremi.
            alfa[i] = static_cast<uint16_t>((i * 1023 + 127) / 255);
        }
    }
};

const TabelleColore &tabelle() {
    static const TabelleColore t;
    return t;
}

inline uint16_t limita(double v, int minimo, int massimo) {
    const int i = static_cast<int>(v + 0.5);
    if (i < minimo) return static_cast<uint16_t>(minimo);
    if (i > massimo) return static_cast<uint16_t>(massimo);
    return static_cast<uint16_t>(i);
}

}  // namespace

struct SubEncoder {
    AVFormatContext *fmt = nullptr;
    AVCodecContext *ctx = nullptr;
    AVStream *stream = nullptr;
    AVFrame *frame = nullptr;
    AVPacket *pkt = nullptr;
    int64_t pts = 0;
    bool header_scritto = false;
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

}  // namespace

extern "C" SubEncoder *sub_encoder_apri(const char *percorso,
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
    // ProRes lavora su macroblocchi 16x16: le dimensioni dispari sono rifiutate.
    if ((larghezza % 2) != 0 || (altezza % 2) != 0) {
        copia_errore(errore, errore_len, "larghezza e altezza devono essere pari");
        return nullptr;
    }

    SubEncoder *enc = new SubEncoder();
    std::string msg;

    int ret = avformat_alloc_output_context2(&enc->fmt, nullptr, "mov", percorso);
    if (ret < 0 || enc->fmt == nullptr) {
        copia_errore(errore, errore_len, errore_av("creazione del contenitore MOV", ret));
        sub_encoder_libera(enc);
        return nullptr;
    }

    const AVCodec *codec = avcodec_find_encoder_by_name("prores_ks");
    if (codec == nullptr) {
        copia_errore(errore, errore_len,
                     "encoder prores_ks non disponibile in questa build di libavcodec");
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
    enc->ctx->pix_fmt = AV_PIX_FMT_YUVA444P10LE;
    enc->ctx->time_base = AVRational{fps_den, fps_num};
    enc->ctx->framerate = AVRational{fps_num, fps_den};
    enc->ctx->sample_aspect_ratio = AVRational{1, 1};
    enc->ctx->colorspace = AVCOL_SPC_BT709;
    enc->ctx->color_primaries = AVCOL_PRI_BT709;
    enc->ctx->color_trc = AVCOL_TRC_BT709;
    enc->ctx->color_range = AVCOL_RANGE_MPEG;
    // prores_ks lavora a fette: oltre una quindicina di thread il guadagno
    // sparisce e libavcodec stesso lo segnala, quindi si limita qui.
    const int thread_usati = thread > 0 ? (thread < 16 ? thread : 16) : 0;
    enc->ctx->thread_count = thread_usati;
    if (thread_usati > 1) {
        enc->ctx->thread_type = FF_THREAD_SLICE;
    }
    // ProRes non ha inter-frame: ogni frame e' un keyframe.
    enc->ctx->gop_size = 1;
    enc->ctx->max_b_frames = 0;

    // profilo 4 = 4444, l'unico (con 4444xq) che porta il canale alfa.
    av_opt_set_int(enc->ctx->priv_data, "profile", 4, 0);
    // I quantizzatori di prores_ks sono espressi come qscale globale.
    enc->ctx->flags |= AV_CODEC_FLAG_QSCALE;
    enc->ctx->global_quality = FF_QP2LAMBDA * (qualita > 0 ? qualita : 4);
    // Vendor Apple: senza di questo alcuni montaggi rifiutano il file.
    av_opt_set(enc->ctx->priv_data, "vendor", "apl0", 0);

    if ((enc->fmt->oformat->flags & AVFMT_GLOBALHEADER) != 0) {
        enc->ctx->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    }

    ret = avcodec_open2(enc->ctx, codec, nullptr);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("apertura di prores_ks", ret));
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

    ret = avformat_write_header(enc->fmt, nullptr);
    if (ret < 0) {
        copia_errore(errore, errore_len, errore_av("scrittura dell'intestazione MOV", ret));
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

    const TabelleColore &t = tabelle();
    uint8_t *piano_y = enc->frame->data[0];
    uint8_t *piano_u = enc->frame->data[1];
    uint8_t *piano_v = enc->frame->data[2];
    uint8_t *piano_a = enc->frame->data[3];

    for (int y = 0; y < h; ++y) {
        const uint8_t *src = rgba + static_cast<size_t>(y) * static_cast<size_t>(passo);
        uint16_t *ry = reinterpret_cast<uint16_t *>(piano_y + static_cast<size_t>(y) * enc->frame->linesize[0]);
        uint16_t *ru = reinterpret_cast<uint16_t *>(piano_u + static_cast<size_t>(y) * enc->frame->linesize[1]);
        uint16_t *rv = reinterpret_cast<uint16_t *>(piano_v + static_cast<size_t>(y) * enc->frame->linesize[2]);
        uint16_t *ra = reinterpret_cast<uint16_t *>(piano_a + static_cast<size_t>(y) * enc->frame->linesize[3]);
        for (int x = 0; x < w; ++x) {
            const uint8_t r = src[x * 4 + 0];
            const uint8_t g = src[x * 4 + 1];
            const uint8_t b = src[x * 4 + 2];
            const uint8_t a = src[x * 4 + 3];
            ra[x] = t.alfa[a];
            if ((r | g | b) == 0) {
                // Nero: scorciatoia frequentissima, il frame e' quasi tutto vuoto.
                ry[x] = 64;
                ru[x] = 512;
                rv[x] = 512;
                continue;
            }
            ry[x] = limita(64.0 + t.yr[r] + t.yg[g] + t.yb[b], 64, 940);
            ru[x] = limita(512.0 + t.ur[r] + t.ug[g] + t.ub[b], 64, 960);
            rv[x] = limita(512.0 + t.vr[r] + t.vg[g] + t.vb[b], 64, 960);
        }
    }

    std::string msg;
    for (int i = 0; i < ripetizioni; ++i) {
        enc->frame->pts = enc->pts++;
        ret = codifica(enc, enc->frame, &msg);
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
    if (enc->header_scritto) {
        ret = av_write_trailer(enc->fmt);
        enc->header_scritto = false;
        if (ret < 0) {
            copia_errore(errore, errore_len, errore_av("scrittura del trailer MOV", ret));
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
