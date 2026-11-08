/* MailCraft: the one LibTomCrypt configuration, shared by SQLCipher and every LibTomCrypt unit
 * so struct layouts agree. Taken from sqlite-wasm-rs (shim/sqlcipher/libtomcrypt.h, MIT),
 * without its wasm-only XCLOCK override: native builds keep LibTomCrypt's OS entropy source. */
#ifndef MAILCRAFT_LTC_H
#define MAILCRAFT_LTC_H

#define LTC_NOTHING
#define LTC_RIJNDAEL
#define LTC_CBC_MODE
#define LTC_SHA1
#define LTC_SHA256
#define LTC_SHA512
#define LTC_HASH_HELPERS
#define LTC_HMAC
#define LTC_FORTUNA
#define LTC_PKCS_5
#define LTC_RNG_GET_BYTES
#define LTC_PRNG_ENABLE_LTC_RNG
#define LTC_NO_TEST
#define LTC_NO_FILE
#define LTC_NO_ASM
#define LTC_NO_MATH
#define LTC_CLEAN_STACK
/* Bad arguments return CRYPT_INVALID_ARG instead of raising a signal. */
#define ARGTYPE 4

#endif
