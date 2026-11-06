// The updater's panel: see panel.h. The C++ half, as the firmware's
// components/hub75_shim, confined to this file.

#include "panel.h"

#include <new>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ESP32-HUB75-MatrixPanel-I2S-DMA.h"
#include "esp_log.h"
#include "esp_private/periph_ctrl.h"
#include "esp_rom_gpio.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "hal/gpio_ll.h"
#include "hal/pcnt_ll.h"
#include "picture.h"
#include "soc/gpio_sig_map.h"
#include "soc/soc_caps.h"

/* --- Pins and geometry: keep in step with firmware/src/config.rs ---------- */

#define PIN_R1 25
#define PIN_G1 26
#define PIN_B1 27
#define PIN_R2 14
#define PIN_G2 12
#define PIN_B2 13
#define PIN_A 33
#define PIN_B 32
#define PIN_C 22
#define PIN_D 17
#define PIN_E -1
#define PIN_LAT 4
#define PIN_OE 15
#define PIN_CLK 16
#define PANEL_WIDTH 64
#define PANEL_HEIGHT 32
#define PANEL_CHAIN 2

static_assert(PANEL_WIDTH * PANEL_CHAIN == PICTURE_W && PANEL_HEIGHT == PICTURE_H,
              "picture.h is not the panel's size");

static MatrixPanel_I2S_DMA *s_panel = nullptr;
/* The picture is on the panel (and the dots may be drawn). */
static volatile bool s_up = false;
/* panel_blank has stopped the DMA (it is not restarted). */
static volatile bool s_stopped = false;
static int s_dot = -1;

/* OE driven high as a plain GPIO, register writes only (hub75_shim's
 * oe_high): GPIO_OUT's bit, output enable, the pad's matrix output taken
 * back from I2S1, the IO_MUX function. */
static void oe_high(void)
{
    gpio_ll_set_level(&GPIO, PIN_OE, 1);
    gpio_ll_output_enable(&GPIO, PIN_OE);
    esp_rom_gpio_connect_out_signal(PIN_OE, SIG_GPIO_OUT_IDX, false, false);
    esp_rom_gpio_pad_select_gpio(PIN_OE);
}

extern "C" void panel_blank(void)
{
    oe_high();
    s_up = false;
    if (s_panel != nullptr && !s_stopped) {
        s_stopped = true;
        s_panel->stopDMAoutput();
    }
}

static uint8_t pixel(int x, int y)
{
    const int i = y * PICTURE_W + x;
    return (PICTURE_BITS[i / 4] >> ((i % 4) * 2)) & 3;
}

static void dot(int i, bool on)
{
    const uint8_t *rgb = PICTURE_DOT_RGB;
    for (int y = 0; y < PICTURE_DOT; y++) {
        for (int x = 0; x < PICTURE_DOT; x++) {
            s_panel->drawPixelRGB888(PICTURE_DOT_XY[i][0] + x, PICTURE_DOT_XY[i][1] + y, on ? rgb[0] : 0,
                                     on ? rgb[1] : 0, on ? rgb[2] : 0);
        }
    }
}

extern "C" void panel_dot(int i)
{
    if (i >= 0) {
        i %= PICTURE_DOTS;
    }
    if (!s_up || s_stopped || i == s_dot) {
        return;
    }
    if (s_dot >= 0) {
        dot(s_dot, false);
    }
    if (i >= 0) {
        dot(i, true);
        s_dot = i;
    } else {
        s_dot = -1;
    }
}

static void blank_on_shutdown(void)
{
    panel_blank();
}

/* --- The refresh watch ------------------------------------------------------
 *
 * Does the I2S DMA keep refreshing the panel while the flash is erased and
 * written (the cache off, the other CPU parked)? A pulse counter counts the
 * D row-address line's rising edges at its pad, one a refresh (rows 8-15 of
 * each half), in hardware, whatever the CPUs are doing; the count is read
 * at the marks (panel up, the write pass's start and end) and every tick of
 * the dots task. The checking pass (card reads only, no flash operation)
 * gives the rate the write pass is held to. A board with no panel counts
 * the same: the pad toggles either way. Diagnostics only: nothing here
 * decides anything.
 */
#define WATCH_UNIT 0
/* The counter goes back to 0 at this. */
#define WATCH_LIM 30000
static bool s_watch = false;
static portMUX_TYPE s_watch_lock = portMUX_INITIALIZER_UNLOCKED;
static int s_watch_count;
static int64_t s_watch_us;
static uint32_t s_frames;
static int64_t s_mark_us[3];
static uint32_t s_mark_frames[3];
static bool s_marked[3];
/* The longest time between two looks during the write pass (a flash
 * operation holds the dots task off), and the frames counted in it. */
static uint32_t s_gap_us, s_gap_frames;

static void watch_start(void)
{
    periph_module_enable(PERIPH_PCNT_MODULE);
    pcnt_dev_t *hw = &PCNT;
    pcnt_ll_stop_count(hw, WATCH_UNIT);
    pcnt_ll_disable_all_events(hw, WATCH_UNIT);
    pcnt_ll_enable_glitch_filter(hw, WATCH_UNIT, false);
    pcnt_ll_set_high_limit_value(hw, WATCH_UNIT, WATCH_LIM);
    pcnt_ll_set_low_limit_value(hw, WATCH_UNIT, -WATCH_LIM);
    pcnt_ll_enable_high_limit_event(hw, WATCH_UNIT, true);
    pcnt_ll_set_edge_action(hw, WATCH_UNIT, 0, PCNT_CHANNEL_EDGE_ACTION_INCREASE, PCNT_CHANNEL_EDGE_ACTION_HOLD);
    pcnt_ll_set_level_action(hw, WATCH_UNIT, 0, PCNT_CHANNEL_LEVEL_ACTION_KEEP, PCNT_CHANNEL_LEVEL_ACTION_KEEP);
    pcnt_ll_set_edge_action(hw, WATCH_UNIT, 1, PCNT_CHANNEL_EDGE_ACTION_HOLD, PCNT_CHANNEL_EDGE_ACTION_HOLD);
    pcnt_ll_set_level_action(hw, WATCH_UNIT, 1, PCNT_CHANNEL_LEVEL_ACTION_KEEP, PCNT_CHANNEL_LEVEL_ACTION_KEEP);
    /* The pad's input on (its output, I2S1's D, is untouched), into the
     * unit's channel 0. */
    gpio_ll_input_enable(&GPIO, PIN_D);
    esp_rom_gpio_connect_in_signal(PIN_D, PCNT_SIG_CH0_IN0_IDX, false);
    pcnt_ll_clear_count(hw, WATCH_UNIT);
    pcnt_ll_start_count(hw, WATCH_UNIT);
    s_watch_count = 0;
    s_watch_us = esp_timer_get_time();
    s_frames = 0;
    s_watch = true;
}

/* Fold the counter into s_frames; the time between this look and the last. */
static uint32_t watch_look(uint32_t *frames)
{
    const int c = pcnt_ll_get_count(&PCNT, WATCH_UNIT);
    const int64_t now = esp_timer_get_time();
    int d = c - s_watch_count;
    if (d < 0) {
        d += WATCH_LIM;
    }
    const uint32_t dt = (uint32_t)(now - s_watch_us);
    s_watch_count = c;
    s_watch_us = now;
    s_frames += (uint32_t)d;
    *frames = (uint32_t)d;
    return dt;
}

extern "C" void panel_watch_tick(void)
{
    if (!s_watch) {
        return;
    }
    portENTER_CRITICAL(&s_watch_lock);
    uint32_t f;
    const uint32_t dt = watch_look(&f);
    if (s_marked[1] && !s_marked[2] && dt > s_gap_us) {
        s_gap_us = dt;
        s_gap_frames = f;
    }
    portEXIT_CRITICAL(&s_watch_lock);
}

extern "C" void panel_watch_mark(int i)
{
    if (!s_watch || i < 0 || i > 2) {
        return;
    }
    portENTER_CRITICAL(&s_watch_lock);
    uint32_t f;
    watch_look(&f);
    s_mark_us[i] = s_watch_us;
    s_mark_frames[i] = s_frames;
    s_marked[i] = true;
    if (i == 1) {
        /* A new write pass (a retry's too). */
        s_marked[2] = false;
        s_gap_us = 0;
        s_gap_frames = 0;
    }
    portEXIT_CRITICAL(&s_watch_lock);
}

extern "C" bool panel_watch_report(char *out, size_t len)
{
    if (!s_watch || !s_marked[0] || !s_marked[1] || !s_marked[2]) {
        return false;
    }
    const double tq = (double)(s_mark_us[1] - s_mark_us[0]) / 1e6;
    const double tw = (double)(s_mark_us[2] - s_mark_us[1]) / 1e6;
    const uint32_t q = s_mark_frames[1] - s_mark_frames[0];
    const uint32_t w = s_mark_frames[2] - s_mark_frames[1];
    if (q == 0 || tq <= 0 || tw <= 0) {
        snprintf(out, len, "panel refresh: %u frames counted checking, %u writing: the DMA is not running", (unsigned)q,
                 (unsigned)w);
        return true;
    }
    const double hz = q / tq;
    /* What the write pass lacked against the checking pass's rate (+: frames
     * missing, a stall), and the same for its longest look away. */
    const double short_ms = (tw - w / hz) * 1e3;
    const double gap_short_ms = (s_gap_us / 1e6 - s_gap_frames / hz) * 1e3;
    snprintf(out, len,
             "panel refresh: %.2f Hz checking (%u frames in %.1f s), %.2f Hz writing (%u in %.1f s, %.0f ms short); "
             "longest look away %.0f ms, %u frames in it (%.0f ms short)",
             hz, (unsigned)q, tq, w / tw, (unsigned)w, tw, short_ms, s_gap_us / 1e3, (unsigned)s_gap_frames,
             gap_short_ms);
    return true;
}

extern "C" bool panel_start(const panel_timing_t *t, char *why, size_t why_len)
{
    why[0] = 0;
    if (s_panel != nullptr) {
        snprintf(why, why_len, "started already");
        return false;
    }
    /* Before anything can light it: a restart from here on blanks first. */
    if (esp_register_shutdown_handler(blank_on_shutdown) != ESP_OK) {
        snprintf(why, why_len, "no shutdown handler");
        return false;
    }
    HUB75_I2S_CFG::i2s_pins pins = {
        PIN_R1, PIN_G1, PIN_B1, PIN_R2, PIN_G2, PIN_B2, PIN_A, PIN_B, PIN_C, PIN_D, PIN_E, PIN_LAT, PIN_OE, PIN_CLK,
    };
    HUB75_I2S_CFG cfg(PANEL_WIDTH, PANEL_HEIGHT, PANEL_CHAIN, pins,
                      static_cast<HUB75_I2S_CFG::shift_driver>(t->driver), HUB75_I2S_CFG::TYPE138,
                      false /* single buffer, as the firmware (config::DOUBLE_BUFFER) */,
                      static_cast<HUB75_I2S_CFG::clk_speed>(t->i2s_mhz * 1000000), t->latch_blanking, t->clk_phase,
                      t->min_refresh);
    /* Not the fourteen "Using GPIO n for ..." lines (the pins are fixed);
     * the driver's I2S-DMA lines (memory, refresh rate) stay. */
    esp_log_level_set("begin()", ESP_LOG_WARN);
    MatrixPanel_I2S_DMA *p = new (std::nothrow) MatrixPanel_I2S_DMA(cfg);
    if (p == nullptr) {
        snprintf(why, why_len, "no memory for the driver");
        return false;
    }
    if (!p->begin()) {
        /* setupDMA's allocation (or the config) refused before the DMA
         * started: OE back to a plain GPIO, high; the object is left. */
        oe_high();
        snprintf(why, why_len, "the driver's begin() failed");
        return false;
    }
    s_panel = p;
    p->setBrightness8(t->brightness);
    for (int y = 0; y < PICTURE_H; y++) {
        for (int x = 0; x < PICTURE_W; x++) {
            const uint8_t k = pixel(x, y);
            if (k != 0) {
                const uint8_t *c = PICTURE_PALETTE[k];
                p->drawPixelRGB888(x, y, c[0], c[1], c[2]);
            }
        }
    }
    s_up = !s_stopped;
    if (!s_up) {
        snprintf(why, why_len, "blanked while starting");
        return false;
    }
    watch_start();
    panel_watch_mark(0);
    snprintf(why, why_len, "%d Hz", p->calculated_refresh_rate);
    return true;
}
