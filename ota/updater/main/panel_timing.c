/*
 * The firmware's panel timing in its NVS form (panel.h): the parser, plain C
 * with no ESP-IDF in it, so `make test` runs it on the host against the
 * vectors opendmd-core's PanelTiming::from_nvs is held to
 * (ota/updater/test/panel_timing_vectors.txt).
 */
#include <stdbool.h>
#include <stdint.h>
#include <string.h>
#include <strings.h>

#include "panel.h"

void panel_timing_default(panel_timing_t *t, uint8_t brightness)
{
    t->clk_phase = true;
    t->latch_blanking = 1;
    t->driver = 0;
    t->i2s_mhz = 8;
    t->min_refresh = 60;
    t->brightness = brightness;
}

/* A decimal field of 1-3 digits, nothing else. */
static bool number(const char *s, unsigned *out)
{
    size_t n = strlen(s);
    if (n == 0 || n > 3) {
        return false;
    }
    unsigned v = 0;
    for (size_t i = 0; i < n; i++) {
        if (s[i] < '0' || s[i] > '9') {
            return false;
        }
        v = v * 10 + (unsigned)(s[i] - '0');
    }
    *out = v;
    return true;
}

/* opendmd_core::panel_timing::Driver's names, in the library's order. */
static const char *const DRIVERS[] = {"shiftreg", "fm6124", "fm6126a", "icn2038s", "mbi5124", "dp3246"};

bool panel_timing_parse(const char *s, panel_timing_t *t)
{
    char buf[48];
    size_t n = strlen(s);
    if (n == 0 || n >= sizeof buf) {
        return false;
    }
    /* from_nvs trims the whole string, then splits on ',' with no trim of
     * the fields (Driver::from_name trims its own and ignores case). */
    while (n > 0 && (s[n - 1] == ' ' || s[n - 1] == '\t' || s[n - 1] == '\n' || s[n - 1] == '\r')) {
        n--;
    }
    while (n > 0 && (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r')) {
        s++;
        n--;
    }
    memcpy(buf, s, n);
    buf[n] = 0;
    char *field[6];
    int k = 0;
    field[k++] = buf;
    for (char *p = buf; *p; p++) {
        if (*p == ',') {
            if (k == 6) {
                return false;
            }
            *p = 0;
            field[k++] = p + 1;
        }
    }
    if (k != 5) {
        return false;
    }
    panel_timing_t v = *t;
    if (strcmp(field[0], "1") == 0) {
        v.clk_phase = true;
    } else if (strcmp(field[0], "0") == 0) {
        v.clk_phase = false;
    } else {
        return false;
    }
    unsigned latch, mhz, hz;
    if (!number(field[1], &latch) || !number(field[3], &mhz) || !number(field[4], &hz)) {
        return false;
    }
    /* The driver's name, trimmed, any case. */
    char *d = field[2];
    while (*d == ' ' || *d == '\t') {
        d++;
    }
    size_t dl = strlen(d);
    while (dl > 0 && (d[dl - 1] == ' ' || d[dl - 1] == '\t')) {
        d[--dl] = 0;
    }
    int driver = -1;
    for (int i = 0; i < (int)(sizeof DRIVERS / sizeof DRIVERS[0]); i++) {
        if (strcasecmp(d, DRIVERS[i]) == 0) {
            driver = i;
        }
    }
    /* PanelTiming::valid: LATCH_MIN..=LATCH_MAX, I2S_SPEEDS, REFRESH_MIN..=REFRESH_MAX. */
    if (driver < 0 || latch < 1 || latch > 4 || (mhz != 8 && mhz != 16 && mhz != 20) || hz < 30 || hz > 240) {
        return false;
    }
    v.latch_blanking = (uint8_t)latch;
    v.driver = driver;
    v.i2s_mhz = (uint8_t)mhz;
    v.min_refresh = (uint8_t)hz;
    *t = v;
    return true;
}

