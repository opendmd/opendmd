/*
 * The updater's panel (docs/ota.md): the build-time picture (picture.h) put
 * on the HUB75 panel once, through mrcodetastic's ESP32-HUB75-MatrixPanel-
 * I2S-DMA as the firmware drives it, then left to the I2S DMA, which loops
 * over its frame buffer by itself while the CPU copies the firmware.
 *
 * Nothing here is on the copy's path: panel_start runs once before it, the
 * dots step on their own task on CPU1, and any failure leaves the panel
 * dark, as an updater without a panel would.
 */
#ifndef OPENDMD_UPDATER_PANEL_H
#define OPENDMD_UPDATER_PANEL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The firmware's panel timing (opendmd_core::panel_timing::PanelTiming), in
 * its NVS form "clk,latch,driver,mhz,hz", and the brightness to show at. */
typedef struct {
    bool clk_phase;
    uint8_t latch_blanking;
    /* HUB75_I2S_CFG::shift_driver: 0 shiftreg .. 5 dp3246. */
    int driver;
    uint8_t i2s_mhz;
    uint8_t min_refresh;
    uint8_t brightness;
} panel_timing_t;

/* What PanelTiming::default() is (positive edge, 1 clock of blanking, a
 * shift register, 8 MHz, 60 Hz), at `brightness`. */
void panel_timing_default(panel_timing_t *t, uint8_t brightness);

/* PanelTiming::from_nvs: false (and *t untouched) for anything malformed or
 * out of range, as the firmware takes the defaults then. */
bool panel_timing_parse(const char *s, panel_timing_t *t);

/* Start the panel with `t` and draw the picture. False if it did not come
 * up (`why` says why); the HUB75 lines are blanked again (OE high) then. */
bool panel_start(const panel_timing_t *t, char *why, size_t why_len);

/* Light dot `i` (mod PICTURE_DOTS) of PICTURE_DOT_XY and put out the one
 * lit before; negative: none. A no-op unless the panel is up. */
void panel_dot(int i);

/* The refresh watch (panel.cpp): the panel's refreshes counted in hardware,
 * looked at every tick of the dots task and at the marks: 1 when the write
 * pass starts, 2 when it ends (0 is panel_start's). */
void panel_watch_tick(void);
void panel_watch_mark(int i);
/* After mark 2: the refresh rate checking and writing, and how many ms of
 * refreshes the write pass (and its longest flash operation) lacked
 * against the checking rate. False without a panel or a mark. */
bool panel_watch_report(char *out, size_t len);

/* OE high as a plain GPIO, then the DMA stopped (its buffer blacked). Safe
 * to call more than once, before or after panel_start, from any task;
 * registered as a shutdown handler by panel_start. */
void panel_blank(void);

#ifdef __cplusplus
}
#endif

#endif
