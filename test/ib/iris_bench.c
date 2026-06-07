/*
 * iris_bench.c — GDC-friendly benchmark for IRIS (IRIX 6.5, openssl 0.9.7, zlib).
 *
 * Build on IRIX:
 *   cc -O2 -n32 -mips3 iris_bench.c -lcrypto -lz -o iris_bench
 *
 * Run:
 *   ./iris_bench [repeats]    # default 100
 *
 * Design:
 *   - 2 MB working buffer, filled from a fixed LCG seed (reproducible).
 *   - Sweep the buffer in 32 KB chunks. On EACH chunk, switch to a different
 *     cipher/hash/compress algorithm. With ~8 algorithms in rotation, one pass
 *     across the 2 MB buffer cycles through every algorithm 8 times.
 *   - This keeps L1I under constant pressure: each chunk forces a code-path
 *     change >> 32 KB L1I, so without GDC the decoder runs nearly continuously,
 *     and with GDC the same hot blocks are reused immediately.
 *   - Inner work per chunk is bounded (32 KB << L1D-friendly), so we don't sit
 *     in any one algo's tight loop long enough to make it the only thing
 *     measured.
 *   - Timing is via times(2) (HZ=100 per openssl build options).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/times.h>
#include <unistd.h>

#include <openssl/md5.h>
#include <openssl/sha.h>
#include <openssl/aes.h>
#include <openssl/des.h>
#include <openssl/blowfish.h>
#include <openssl/rc4.h>
#include <zlib.h>

#define BUFSZ        (2 * 1024 * 1024)  /* 2 MB working set */
#define CHUNK         (32 * 1024)        /* switch algo every 32 KB */
#define NCHUNKS       (BUFSZ / CHUNK)    /* 64 chunks per pass */

static unsigned char buf[BUFSZ];
static unsigned char obuf[BUFSZ + 4096]; /* zlib can grow slightly */

static void seed_lcg(unsigned s)
{
    unsigned x = s;
    int i;
    for (i = 0; i < BUFSZ; i++) {
        x = x * 1103515245u + 12345u;
        buf[i] = (unsigned char)(x >> 16);
    }
}

int main(int argc, char **argv)
{
    int reps = (argc > 1) ? atoi(argv[1]) : 100;
    if (reps <= 0) reps = 100;

    seed_lcg(0xdeadbeefu);

    /* Pre-derive keys from the buffer head (deterministic). */
    AES_KEY aes_k;
    AES_set_encrypt_key(buf, 128, &aes_k);

    DES_key_schedule des_k;
    DES_set_key_unchecked((DES_cblock *)buf, &des_k);

    BF_KEY bf_k;
    BF_set_key(&bf_k, 16, buf);

    RC4_KEY rc4_k;
    RC4_set_key(&rc4_k, 16, buf);

    unsigned char md[64];
    unsigned char iv[16];

    printf("iris_bench: BUFSZ=%d CHUNK=%d NCHUNKS=%d reps=%d\n",
           BUFSZ, CHUNK, NCHUNKS, reps);

    struct tms tms0, tms1;
    clock_t t0 = times(&tms0);

    unsigned long long bytes = 0;
    int r;
    for (r = 0; r < reps; r++) {
        int c;
        for (c = 0; c < NCHUNKS; c++) {
            unsigned char *in  = buf + c * CHUNK;
            unsigned char *out = obuf + c * CHUNK;

            /* Algo rotates every chunk. 8 algorithms in cycle. */
            memset(iv, (unsigned char)(c + r), sizeof(iv));

            switch (c & 7) {
            case 0:
                MD5(in, CHUNK, md);
                break;
            case 1:
                SHA1(in, CHUNK, md);
                break;
            case 2:
                AES_cbc_encrypt(in, out, CHUNK, &aes_k, iv, AES_ENCRYPT);
                break;
            case 3:
                DES_ncbc_encrypt(in, out, CHUNK, &des_k,
                                 (DES_cblock *)iv, DES_ENCRYPT);
                break;
            case 4:
                BF_cbc_encrypt(in, out, CHUNK, &bf_k, iv, BF_ENCRYPT);
                break;
            case 5: {
                RC4_KEY local_k;
                RC4_set_key(&local_k, 16, in);
                RC4(&local_k, CHUNK, in, out);
                break;
            }
            case 6: {
                /* zlib compress */
                uLongf zlen = CHUNK + 1024;
                compress(out, &zlen, in, CHUNK);
                break;
            }
            case 7: {
                /* zlib decompress of a freshly compressed copy:
                 * compress in→tmp, decompress tmp→out. Two distinct code
                 * paths; lots of icache churn. */
                static unsigned char tmp[CHUNK + 1024];
                uLongf zlen = sizeof(tmp);
                if (compress(tmp, &zlen, in, CHUNK) == Z_OK) {
                    uLongf dlen = CHUNK;
                    uncompress(out, &dlen, tmp, zlen);
                }
                break;
            }
            }
            bytes += CHUNK;
        }
    }

    clock_t t1 = times(&tms1);
    double secs = (double)(t1 - t0) / (double)sysconf(_SC_CLK_TCK);
    double user = (double)(tms1.tms_utime - tms0.tms_utime) /
                  (double)sysconf(_SC_CLK_TCK);
    double mb = (double)bytes / (1024.0 * 1024.0);

    printf("elapsed: %.2fs wall, %.2fs user\n", secs, user);
    printf("processed: %.1f MB total\n", mb);
    if (secs > 0.0) {
        printf("throughput: %.2f MB/s wall, %.2f MB/s user\n",
               mb / secs, mb / (user > 0.0 ? user : secs));
    }

    /* Print a checksum of the final state so the compiler can't elide work. */
    unsigned long acc = 0;
    int i;
    for (i = 0; i < BUFSZ; i += 4096) acc += obuf[i];
    printf("anti-elide: %lu\n", acc);

    return 0;
}
