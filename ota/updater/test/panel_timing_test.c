/*
 * The updater's panel timing parser (main/panel_timing.c) on the host,
 * against panel_timing_vectors.txt, which opendmd-core's PanelTiming::from_nvs
 * is held to as well. Built and run by `make test`: cc only, no ESP-IDF.
 */
#include <stdio.h>
#include <string.h>

#include "panel.h"

static const char *const NAMES[] = {"shiftreg", "fm6124", "fm6126a", "icn2038s", "mbi5124", "dp3246"};

int main(int argc, char **argv)
{
    FILE *f = fopen(argc > 1 ? argv[1] : "panel_timing_vectors.txt", "r");
    if (f == NULL) {
        perror("vectors");
        return 2;
    }
    char line[256];
    int n = 0, bad = 0;
    while (fgets(line, sizeof line, f) != NULL) {
        line[strcspn(line, "\n")] = 0;
        if (line[0] == '#') {
            continue;
        }
        char *bar = strrchr(line, '|');
        if (bar == NULL) {
            continue;
        }
        *bar = 0;
        const char *want = bar + 1;
        panel_timing_t t;
        panel_timing_default(&t, 96);
        char got[64];
        if (panel_timing_parse(line, &t)) {
            snprintf(got, sizeof got, "%d,%u,%s,%u,%u", t.clk_phase ? 1 : 0, (unsigned)t.latch_blanking,
                     NAMES[t.driver], (unsigned)t.i2s_mhz, (unsigned)t.min_refresh);
        } else {
            panel_timing_t d;
            panel_timing_default(&d, 96);
            if (d.clk_phase != t.clk_phase || d.latch_blanking != t.latch_blanking || d.driver != t.driver ||
                d.i2s_mhz != t.i2s_mhz || d.min_refresh != t.min_refresh || d.brightness != t.brightness) {
                printf("'%s': refused but changed the timing\n", line);
                bad++;
            }
            strcpy(got, "-");
        }
        if (strcmp(got, want) != 0) {
            printf("'%s': got %s, want %s\n", line, got, want);
            bad++;
        }
        n++;
    }
    fclose(f);
    if (bad != 0 || n == 0) {
        printf("panel timing parser: %d of %d vectors wrong\n", bad, n);
        return 1;
    }
    printf("panel timing parser: %d vectors ok\n", n);
    return 0;
}
